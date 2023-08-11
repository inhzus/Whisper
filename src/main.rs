use lambda_http::{run, service_fn, Body, Error, Request, Response};
use teloxide::{
    requests::Requester,
    types::{Update, UpdateKind},
    Bot,
};

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

    let handler = |event: Request| {
        let bot = bot.clone();
        async move {
            let bot = bot.clone();

            let reply = |code, body| {
                Response::builder()
                    .status(code)
                    .body(body)
                    .map_err(Box::new)
            };
            let update = match event.body() {
                &Body::Text(ref s) => serde_json::from_str::<Update>(s)?,
                &Body::Binary(ref b) => serde_json::from_slice::<Update>(b)?,
                &Body::Empty => {
                    return Ok::<Response<Body>, Error>(reply(400, Body::from("Empty body"))?);
                }
            };

            match update.kind {
                UpdateKind::Message(msg) => {
                    bot.send_message(msg.chat.id, "Hello!").await?;
                }
                _ => {}
            }

            Ok::<Response<Body>, Error>(reply(200, Body::Empty)?)
        }
    };
    run(service_fn(handler)).await
}
