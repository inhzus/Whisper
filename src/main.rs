use lambda_http::{run, service_fn, Body, Error, Request, Response};
use teloxide::{
    net::default_reqwest_settings,
    requests::Requester,
    stop::{mk_stop_token, StopToken},
    types::{Message, Update},
    update_listeners::{StatefulListener, UpdateListener},
    Bot,
};
use tokio::{
    sync::mpsc::{self, UnboundedReceiver},
    time::sleep,
};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::warn;

fn tuple_first_mut<A, B>(tuple: &mut (A, B)) -> &mut A {
    &mut tuple.0
}

fn make_listener(
    rx: UnboundedReceiver<Result<Update, std::convert::Infallible>>,
    stop_token: StopToken,
) -> impl UpdateListener<Err = std::convert::Infallible> {
    let stream = UnboundedReceiverStream::new(rx);
    let listener = StatefulListener::new(
        (stream, stop_token),
        tuple_first_mut,
        |state: &mut (_, StopToken)| state.1.clone(),
    );
    listener
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

    let (tx, rx): (
        mpsc::UnboundedSender<Result<Update, std::convert::Infallible>>,
        _,
    ) = mpsc::unbounded_channel();

    let client = default_reqwest_settings()
        .danger_accept_invalid_certs(true)
        .build()
        .expect("creating reqwest::Client");
    let bot = Bot::from_env_with_client(client.clone());
    let token = bot.token();
    loop {
        let resp = client
            .get(format!("https://api.telegram.com/bot{token}/getMe"))
            .send()
            .await;
        match resp {
            Ok(resp) => {
                if resp.status().is_success() {
                    break;
                }
                warn!(
                    "Failed to getMe: {:?}, text: {}",
                    resp.status(),
                    resp.text().await?
                );
            }
            Err(err) => {
                warn!("Failed to getMe: {:?}", err);
            }
        }
        sleep(tokio::time::Duration::from_secs(1)).await;
    }

    let (stop_token, _) = mk_stop_token();
    let listener = make_listener(rx, stop_token);
    tokio::spawn(async move {
        teloxide::repl_with_listener(
            bot,
            |bot: Bot, msg: Message| async move {
                bot.send_message(msg.chat.id, "Hello world!").await?;
                Ok(())
            },
            listener,
        )
        .await;
    });

    let handler = |event: Request| {
        let tx = tx.clone();
        async move {
            let tx = tx.clone();
            let resp = match event.body() {
                &Body::Text(ref s) => {
                    let update = serde_json::from_str::<Update>(s)?;
                    tx.send(Ok(update))?;
                    "text"
                }
                &Body::Binary(ref b) => {
                    let update = serde_json::from_slice::<Update>(b)?;
                    tx.send(Ok(update))?;
                    "binary"
                }
                &Body::Empty => "empty",
            };
            let resp: Response<Body> = Response::builder()
                .status(200)
                .header("content-type", "text/html")
                .body(resp.into())
                .map_err(Box::new)?;
            Ok::<Response<Body>, Error>(resp)
        }
    };
    run(service_fn(handler)).await
}
