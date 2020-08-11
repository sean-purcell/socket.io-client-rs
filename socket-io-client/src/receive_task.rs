use async_tungstenite::tungstenite::Message as WsMessage;
use futures::{
    channel::mpsc,
    future::RemoteHandle,
    stream::StreamExt,
    task::{Spawn, SpawnError, SpawnExt},
};

use socket_io_protocol::{
    engine::{Decoder, Error as EngineError, Packet},
    socket::{self, Error as SocketError},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Error deserializing engine.io protocol: {0}")]
    EngineError(#[from] EngineError),
    #[error("Error deserializing socket.io protocol: {0}")]
    SocketError(#[from] SocketError),
}

pub fn receive_task(
    mut receiver: mpsc::UnboundedReceiver<WsMessage>,
    _sender: mpsc::UnboundedSender<WsMessage>,
    spawn: &impl Spawn,
) -> Result<RemoteHandle<Result<(), Error>>, SpawnError> {
    let task = async move {
        let mut decoder = Decoder::new();

        let mut process_message = |msg| {
            let packet = decoder.decode(msg)?;
            match packet {
                Packet::Open(open) => {
                    // TODO: forward this info to the client
                    log::info!("Received open packet: {:?}", open);
                }
                Packet::Close => {
                    log::info!("Received close");
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
            let result: Result<(), Error> = Ok(());
            result
        };

        while let Some(msg) = receiver.next().await {
            log::info!("message received: {:?}", msg);
            match msg {
                WsMessage::Close(frame) => {
                    log::debug!("Closed with close frame {:?}", frame);
                    return Ok(());
                }
                WsMessage::Ping(_) | WsMessage::Pong(_) => (), // handled at a lower layer
                WsMessage::Text(text) => process_message(WsMessage::Text(text))?,
                WsMessage::Binary(data) => process_message(WsMessage::Binary(data))?,
            }
        }

        Ok(())
    };

    let handle = spawn.spawn_with_handle(task)?;
    Ok(handle)
}
