use async_tungstenite::tungstenite::{Error as WsError, Message as WsMessage};
use futures::{
    channel::{mpsc, oneshot},
    future::{Future, FutureExt, RemoteHandle},
    io::{AsyncRead, AsyncWrite},
    select,
    sink::SinkExt,
    stream::StreamExt,
    task::{Spawn, SpawnError, SpawnExt},
};

use socket_io_protocol::{
    engine::{Decoder, Error as EngineError, Packet},
    socket::{self, Error as SocketError},
};

use super::Error;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Error deserializing engine.io protocol: {0}")]
    EngineError(#[from] EngineError),
    #[error("Error deserializing socket.io protocol: {0}")]
    SocketError(#[from] SocketError),
}

pub async fn receive_task(
    receiver: mpsc::Receiver<WsMessage>,
    _sender: mpsc::Sender<WsMessage>,
    spawn: &impl Spawn,
) -> Result<RemoteHandle<()>, Error> {
    let task = async move {
        let mut decoder = Decoder::new();

        while let Some(msg) = receiver.next() {
            match msg {
                WsMessage::Close(frame) => {
                    log::debug!("Closed with close frame {:?}", frame);
                    return Ok(());
                }
                WsMessage::Ping(_) | WsMessage::Pong(_) => (), // handled at a lower later
                WsMessage::Text(_) | WsMessage::Binary(_) => {
                    let packet = decoder.decode(&msg)?;
                    match packet {
                        Packet::Open(open) => {
                            // TODO: forward this info to the client
                            log::info!("Received open packet: {:?}", open);
                        }
                        Packet::Ping => {
                            // TODO: send pong when we have a serializer
                        }
                        Packet::Pong => {
                            // TODO: send message to timer task to reset the timeout
                        }
                        Packet::Message(msg) => {
                            let result = socket::deserialize(msg)?;
                            log::info!("Received message packet: {:?}", result);
                        }
                    }
                }
            }
        }
    };

    let handle = spawn.spawn_with_handle(task)?;
    Ok(handle)
}
