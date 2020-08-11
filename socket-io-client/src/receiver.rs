use async_tungstenite::tungstenite::Message as WsMessage;
use futures::{
    channel::mpsc,
    future::RemoteHandle,
    stream::StreamExt,
    task::{Spawn, SpawnError, SpawnExt},
};

use socket_io_protocol::{
    engine::{Decoder, Error as EngineError, Message as EngineMessage, Packet as EnginePacket},
    socket::{self, DeserializeResult, Error as SocketError, Packet, Partial},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Error deserializing engine.io protocol: {0}")]
    EngineError(#[from] EngineError),
    #[error("Error deserializing socket.io protocol: {0}")]
    SocketError(#[from] SocketError),
}

pub struct Receiver {
    _handle: RemoteHandle<Result<(), Error>>,
}

struct InProgress {
    partial: Partial,
    attachments: Vec<EngineMessage>,
}

impl Receiver {
    pub fn new(
        receiver: mpsc::UnboundedReceiver<WsMessage>,
        sender: mpsc::UnboundedSender<WsMessage>,
        spawn: &impl Spawn,
    ) -> Result<Receiver, SpawnError> {
        let _handle = receive_task(receiver, sender, spawn)?;
        Ok(Receiver { _handle })
    }
}

pub fn receive_task(
    mut receiver: mpsc::UnboundedReceiver<WsMessage>,
    _sender: mpsc::UnboundedSender<WsMessage>,
    spawn: &impl Spawn,
) -> Result<RemoteHandle<Result<(), Error>>, SpawnError> {
    let processing = async move {
        let mut decoder = Decoder::new();

        let mut in_progress: Option<InProgress> = None;

        let log_packet = |packet| {
            log::info!("Received packet: {}", packet);
        };

        let mut process_message = |msg| {
            let packet = decoder.decode(msg)?;
            match packet {
                EnginePacket::Open(open) => {
                    // TODO: forward this info to the client
                    log::info!("Received open packet: {:?}", open);
                }
                EnginePacket::Close => {
                    log::info!("Received close");
                }
                EnginePacket::Ping => {
                    // TODO: send pong when we have a serializer
                }
                EnginePacket::Pong => {
                    // TODO: send message to timer task to reset the timeout
                }
                EnginePacket::Message(msg) => {
                    log::info!("Received message packet: {:?}", msg);
                    match in_progress.take() {
                        Some(mut ip) => {
                            ip.add(msg);
                            if ip.ready() {
                                let packet = ip.deserialize()?;
                                log_packet(packet);
                            } else {
                                in_progress = Some(ip);
                            }
                        }
                        None => match socket::deserialize(msg)? {
                            DeserializeResult::Packet(packet) => log_packet(packet),
                            DeserializeResult::DataNeeded(partial) => {
                                in_progress = Some(InProgress::new(partial));
                            }
                        },
                    }
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

    let task = async move {
        let result = processing.await;
        match &result {
            Ok(()) => (),
            Err(e) => log::error!("Error occurred in processing task: {}", e),
        }
        result
    };

    let handle = spawn.spawn_with_handle(task)?;
    Ok(handle)
}

impl InProgress {
    fn new(partial: Partial) -> Self {
        InProgress {
            partial,
            attachments: Vec::new(),
        }
    }

    fn add(&mut self, msg: EngineMessage) {
        self.attachments.push(msg);
    }

    fn ready(&self) -> bool {
        self.partial.attachments() == self.attachments.len() as u64
    }

    fn deserialize(self) -> Result<Packet, SocketError> {
        socket::deserialize_partial(self.partial, self.attachments)
    }
}
