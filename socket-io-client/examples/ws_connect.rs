use std::time::Duration;

use async_tungstenite::{tokio::TokioAdapter, tungstenite::Message as WsMessage};
use futures::{
    future::FutureExt,
    task::{FutureObj, Spawn, SpawnError},
};
use structopt::StructOpt;
use tokio::{io, net::TcpStream};

use socket_io_client::{protocol, Client};

#[derive(Debug, StructOpt)]
#[structopt(name = "ws_connect")]
struct Opt {
    /// The websocket server to connect to
    url: String,

    /// Timeout seconds
    #[structopt(short, long, default_value = "1")]
    timeout: u64,

    /// Send a connect message for the provided namespace
    #[structopt(short, long)]
    namespace: Option<String>,
}

struct TokioSpawn();
impl Spawn for TokioSpawn {
    fn spawn_obj(&self, future: FutureObj<'static, ()>) -> Result<(), SpawnError> {
        let _ = tokio::spawn(future);
        Ok(())
    }
}

async fn connect(host: String, port: u16) -> Result<TokioAdapter<TcpStream>, io::Error> {
    Ok(TokioAdapter(
        TcpStream::connect((host.as_str(), port)).await?,
    ))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let opt = Opt::from_args();
    log::info!("Args: {:?}", opt);

    let spawn = TokioSpawn();

    let mut client = Client::connect(opt.url, connect, &spawn).await?;

    client.set_fallback_callback(|args: &protocol::Args| println!("{}", args));
    let timeout = tokio::time::delay_for(Duration::from_secs(opt.timeout)).fuse();

    if let Some(namespace) = &opt.namespace {
        let n2 = namespace.clone();
        client.set_namespace_fallback_callback(namespace, move |args: &protocol::socket::Args| {
            println!("{}: {}", n2, args)
        });
        client
            .send
            .unbounded_send(WsMessage::Text(format!("40{},", namespace)))?;
    }

    timeout.await;

    client.close().await?;

    Ok(())
}
