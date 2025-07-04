use ezsockets::ClientConfig;
use fluxrpc_core::codec::json::JsonCodec;
use fluxrpc_core::{
    ErrorBody, Request, RpcSession, SessionState, TypedRpcHandler, WebsocketClientTransport,
    websocket_connect, websocket_listen,
};
use futures::future::join_all;
use nanoid::nanoid;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::Instant;
use url::Url;

struct MyState {
    counter: Mutex<u64>,
}

impl MyState {
    pub fn new() -> Self {
        Self {
            counter: Mutex::new(0),
        }
    }

    pub fn inc(&self) -> u64 {
        let mut counter = self.counter.lock().unwrap();
        *counter += 1;
        *counter
    }
}

impl SessionState for MyState {}

async fn start_server(addr: SocketAddr) {
    let codec = JsonCodec::new();
    let mut handler = TypedRpcHandler::<MyState>::new();
    handler.with_open_handler(|s, _| async move {
        println!("OPENED");
        Ok(())
    });
    handler.register_request_handler("ping", |ctx, _: ()| async move {
        let s = ctx.state().inc();
        println!("{} requests", s);

        // send a new request
        ctx.request(
            &Request {
                id: nanoid!(),
                method: "ping".to_string(),
                params: None,
            },
            Duration::from_millis(10000).into(),
        )
        .await
        .unwrap();

        Result::<(), ErrorBody>::Ok(())
    });

    let _ = websocket_listen(addr, codec.clone(), Arc::new(handler), || async move {
        Ok(MyState::new())
    })
    .await
    .unwrap();
}

async fn start_client(
    addr: SocketAddr,
) -> anyhow::Result<Arc<RpcSession<JsonCodec, WebsocketClientTransport, ()>>> {
    let codec = JsonCodec::new();
    let mut handler = TypedRpcHandler::new();
    handler.register_request_handler(
        "ping",
        |s, _: ()| async move { Result::<(), ErrorBody>::Ok(()) },
    );
    let client_url = Url::parse(format!("ws://{}", addr).as_str())?;
    let client_config = ClientConfig::new(client_url);
    let client = websocket_connect(client_config, codec, Arc::new(handler), ()).await?;
    Ok(client)
}

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let addr: SocketAddr = "127.0.0.1:8080".parse()?;

    // 1. start server
    start_server(addr.clone()).await;

    // 2. start client
    let client = start_client(addr).await?;

    let n = 5000;
    let start_at = Instant::now();

    let futures = (0..n)
        .map(|_| {
            let client = client.clone(); // Clone Arc or handle as needed
            tokio::spawn(async move {
                client
                    .request(
                        &Request {
                            id: nanoid!(),
                            method: "ping".to_string(),
                            params: None,
                        },
                        Duration::from_millis(10_000).into(),
                    )
                    .await
            })
        })
        .collect::<Vec<_>>();

    join_all(futures).await;

    println!(
        "{} requests per second",
        n as f64 / start_at.elapsed().as_secs_f64()
    );

    Ok(())
}
