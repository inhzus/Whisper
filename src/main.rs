use std::{
    future::Future,
    sync::{atomic::AtomicU8, Arc},
};

use anyhow::Context;
use aws_sdk_s3::primitives::{ByteStream, SdkBody};
use lambda_http::{run, service_fn, Body, Error, Request, Response};
use s3manager::S3Manager;
use teloxide::{
    requests::Requester,
    types::{Message, Update, UpdateKind},
    Bot,
};
use transcribe::Transcriber;
mod s3manager;
mod transcribe;

struct BotHandler {
    bot: Bot,
    transcriber: Transcriber,
    s3manager: S3Manager,
    uniq_media_id: Arc<AtomicU8>,
}

type EventResult = Result<Response<Body>, Error>;

impl BotHandler {
    fn new(bot: Bot, transcriber: Transcriber, s3manager: S3Manager) -> Self {
        let uniq_media_id = Arc::new(AtomicU8::new(0));
        Self {
            bot,
            transcriber,
            s3manager,
            uniq_media_id,
        }
    }

    async fn handle(&self, event: Request) -> EventResult {
        let update = match event.body() {
            &Body::Text(ref s) => serde_json::from_str::<Update>(s)?,
            &Body::Binary(ref b) => serde_json::from_slice::<Update>(b)?,
            &Body::Empty => {
                return Self::reply(400, Body::from("Empty body"));
            }
        };

        match update.kind {
            UpdateKind::Message(msg) => {
                let chat_id = msg.chat.id;
                let res = self.handle_msg(msg).await;
                let err_handler = |err| async move {
                    self.bot
                        .send_message(chat_id, format!("Error: {:?}", err))
                        .await?;
                    Ok(())
                };
                self.handle_wrapper(res, err_handler).await?;
            }
            _ => {}
        }
        Self::reply(200, Body::Empty)
    }

    async fn handle_wrapper<ErrorCb, ErrorCbFut>(
        &self,
        result: anyhow::Result<()>,
        error_cb: ErrorCb,
    ) -> Result<(), Error>
    where
        ErrorCb: FnOnce(anyhow::Error) -> ErrorCbFut,
        ErrorCbFut: Future<Output = anyhow::Result<()>>,
    {
        if let Err(err) = result {
            tracing::error!("Error: {:?}", err);
            error_cb(err).await?;
        }
        Ok(())
    }

    async fn handle_msg(&self, msg: Message) -> anyhow::Result<()> {
        if let Some(text) = msg.text() {
            if let Ok(url) = url::Url::parse(text) {
                return self.handle_url(msg, url).await;
            }
        }
        self.bot
            .send_message(msg.chat.id, "Send me sth. interesting, huh?")
            .await?;
        Ok(())
    }

    async fn handle_url(&self, msg: Message, url: url::Url) -> anyhow::Result<()> {
        let mut resp = reqwest::get(url).await?;
        let type_prefix_len = 64;
        let mut buffer = Vec::with_capacity(type_prefix_len * 2);
        let (mut tx, channel_body) = hyper::Body::channel();
        let filetype = 'filetype: {
            while let Some(chunk) = resp.chunk().await? {
                buffer.extend_from_slice(&chunk);
                tx.send_data(chunk).await?;
                if buffer.len() < type_prefix_len {
                    continue;
                }
                break 'filetype Self::infer_filetype(&buffer)?;
            }
            return Err(anyhow::Error::msg("File too small"));
        };
        let time_str = chrono::Local::now().format("%Y%m%d-%H-%M-%S");
        let uniq_id = self
            .uniq_media_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        while let Some(chunk) = resp.chunk().await? {
            tx.send_data(chunk).await?;
        }
        drop(tx);

        let bucket = "suun-whisper";
        let filename = format!("{}-{}.{}", time_str, uniq_id, filetype.extension());
        self.s3manager
            .put_object(
                bucket,
                &filename,
                ByteStream::from(SdkBody::from(channel_body)),
            )
            .await?;

        let transcribe_result = self.transcriber.run(bucket, &filename).await?;
        self.bot
            .send_message(msg.chat.id, transcribe_result)
            .await?;
        Ok(())
    }

    fn infer_filetype(buffer: &Vec<u8>) -> anyhow::Result<infer::Type> {
        let filetype = infer::get(buffer).context("infer filetype")?;
        let extension = filetype.extension();
        match extension {
            "amr" | "flac" | "m4a" | "mp3" | "mp4" | "ogg" | "webm" | "wav" => {}
            &_ => {
                let err_msg = format!("Unsupported filetype: {:?}", extension);
                return Err(anyhow::Error::msg(err_msg));
            }
        }
        Ok(filetype)
    }

    fn reply(code: u16, body: Body) -> EventResult {
        Response::builder()
            .status(code)
            .body(body)
            .map_err(|err| lambda_http::Error::from(err))
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        // disable printing the name of the module in every log line.
        .with_target(false)
        // disabling time is handy because CloudWatch will add the ingestion time.
        .without_time()
        .init();

    let bot = Bot::from_env();
    let config = aws_config::load_from_env().await;
    let transcriber = Transcriber::new(&config);
    let s3manager = S3Manager::new(&config);
    let handler = BotHandler::new(bot, transcriber, s3manager);
    let handler_ref = &handler;

    run(service_fn(|event: Request| async move {
        handler_ref.handle(event).await
    }))
    .await
}
