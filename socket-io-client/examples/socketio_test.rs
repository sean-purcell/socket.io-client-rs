use std::time::Duration;

use async_tungstenite::tokio::TokioAdapter;
use futures::{
    future::FutureExt,
    task::{FutureObj, Spawn, SpawnError},
};
use structopt::StructOpt;
use tokio::{io, net::TcpStream};

use socket_io_client::{protocol, AckBuilder, Client};

#[derive(Debug, StructOpt)]
#[structopt(name = "ws_connect")]
struct Opt {
    /// The websocket server to connect to
    url: String,

    /// Timeout seconds
    #[structopt(short, long, default_value = "1")]
    timeout: u64,
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

    client.set_fallback_callback(|args: &protocol::Args, _ack| println!("{}", args));
    client.set_event_callback("types", |args: &protocol::Args, ack: Option<AckBuilder>| {
        println!("types messaged received: {}", args);
        if let Some(ack) = ack {
            println!("Emitting ack");
            ack.args().arg("message received").unwrap().send();
        }
    });
    println!("Callbacks registered");

    client
        .emit("event")
        .binary(true)
        .callback(|args: &protocol::Args| {
            println!("event ack received: {}", args);
        })
        .args()
        .arg(&vec![0xdeu8, 0xad, 0xbe, 0xef])?
        .arg("hello")?
        .send();

    let timeout = tokio::time::delay_for(Duration::from_secs(opt.timeout)).fuse();

    timeout.await;

    client.close().await?;

    Ok(())
}
