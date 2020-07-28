use async_tungstenite::tokio::TokioAdapter;
use futures::task::{FutureObj, Spawn, SpawnError};
use structopt::StructOpt;
use tokio::{io, net::TcpStream};

use socket_io_client::Client;

#[derive(Debug, StructOpt)]
#[structopt(name = "ws_connect")]
struct Opt {
    /// The websocket server to connect to
    uri: String,
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
    let opt = Opt::from_args();
    println!("Args: {:?}", opt);

    let spawn = TokioSpawn();

    Client::connect(opt.uri, connect, spawn).await?;

    Ok(())
}
