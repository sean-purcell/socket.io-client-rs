use std::{
    borrow::Cow,
    sync::{Arc, Mutex},
};

use async_tungstenite::tungstenite::Message as WsMessage;
use futures::channel::{mpsc, oneshot};

use socket_io_protocol::{
    engine::{
        self, Decoder, Error as EngineError, Message as EngineMessage, Packet as EnginePacket,
    },
    socket::{self, ArgsError, Data, DeserializeResult, Error as SocketError, Packet, Partial},
};

use super::{AckBuilder, Callbacks};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Error deserializing engine.io protocol: {0}")]
    EngineError(#[from] EngineError),
    #[error("Error deserializing socket.io protocol: {0}")]
    SocketError(#[from] SocketError),
    #[error("Error deserializing argument: {0}")]
    ArgsError(#[from] ArgsError),
    #[error("Event packet with no arguments: {0:?}")]
    EventNoArgs(Packet),
    #[error("Unexpected ack received: {0:?}")]
    UnexpectedAck(Packet),
}

pub struct Receiver {
    decoder: Decoder,
    in_progress: Option<InProgress>,
    sender: mpsc::UnboundedSender<Vec<WsMessage>>,
    callbacks: Arc<Mutex<Callbacks>>,
    open: Option<oneshot::Sender<engine::Open>>,
}

struct InProgress {
    partial: Partial,
    attachments: Vec<EngineMessage>,
}

impl Receiver {
    pub fn new(
        sender: mpsc::UnboundedSender<Vec<WsMessage>>,
        callbacks: Arc<Mutex<Callbacks>>,
        open: oneshot::Sender<engine::Open>,
    ) -> Receiver {
        Receiver {
            decoder: Decoder::new(),
            in_progress: None,
            sender,
            callbacks,
            open: Some(open),
        }
    }

    pub fn process_websocket_packet(&mut self, msg: WsMessage) -> Result<(), Error> {
        log::trace!("Received WebSocket packet: {:?}", msg);
        match msg {
            WsMessage::Close(frame) => {
                log::debug!("Closed with close frame {:?}", frame);
                Ok(())
            }
            WsMessage::Ping(_) | WsMessage::Pong(_) => Ok(()), // already handled by tungstenite
            WsMessage::Text(text) => self.process_message(WsMessage::Text(text)),
            WsMessage::Binary(data) => self.process_message(WsMessage::Binary(data)),
        }
    }

    fn process_message(&mut self, msg: WsMessage) -> Result<(), Error> {
        let packet = self.decoder.decode(msg)?;
        match packet {
            EnginePacket::Open(open) => {
                // TODO: forward this info to the client
                log::trace!("Received open engine packet: {:?}", open);
                if let Some(send) = self.open.take() {
                    let _ = send.send(open);
                } else {
                    log::warn!("Received second open engine packet: {:?}", open);
                }
                Ok(())
            }
            EnginePacket::Close => {
                log::trace!("Received close engine packet");
                Ok(())
            }
            EnginePacket::Ping => {
                log::trace!("Received engine ping packet");
                let _ = self.sender.unbounded_send(vec![engine::encode_pong()]);
                // TODO: send message to timer task to reset the timeout
                Ok(())
            }
            EnginePacket::Pong => {
                log::trace!("Received engine ping packet");
                // TODO: send message to timer task to reset the timeout
                Ok(())
            }
            EnginePacket::Message(msg) => {
                log::trace!("Received message engine packet: {:?}", msg);
                match self.in_progress.take() {
                    Some(mut ip) => {
                        ip.add(msg);
                        if ip.ready() {
                            let packet = ip.deserialize()?;
                            self.process_packet(packet)?;
                        } else {
                            self.in_progress = Some(ip);
                        }
                        Ok(())
                    }
                    None => match socket::deserialize(msg)? {
                        DeserializeResult::Packet(packet) => self.process_packet(packet),
                        DeserializeResult::DataNeeded(partial) => {
                            self.in_progress = Some(InProgress::new(partial));
                            Ok(())
                        }
                    },
                }
            }
        }
    }

    fn process_packet(&mut self, packet: Packet) -> Result<(), Error> {
        log::info!("Received socket packet: {}", packet);
        let namespace = packet.namespace();
        match packet.data() {
            Data::Connect => {
                log::info!("Received connect for {}", namespace);
                // TODO: Call connect callback
            }
            Data::Disconnect => {
                log::info!("Received disconnect for {}", namespace);
                // TODO: Call disconnect callback
            }
            Data::Event { args, id } => {
                let event = args
                    .get(0)
                    .ok_or_else(|| Error::EventNoArgs(packet.clone()))?;
                let event: Cow<'_, str> = event.deserialize()?;
                let ack = id.map(|id| AckBuilder::new(self.sender.clone(), namespace, id));
                // TODO: Use id to create ack callback
                if let Some(mut cb) = self.callbacks.lock().unwrap().get_event(namespace, &*event) {
                    cb.call(&args, ack);
                }
            }
            Data::Ack { id, args } => {
                if let Some(cb) = self
                    .callbacks
                    .lock()
                    .unwrap()
                    .get_and_clear_ack(namespace, id)
                {
                    cb.call(&args);
                } else {
                    return Err(Error::UnexpectedAck(packet.clone()));
                }
            }
        };
        Ok(())
    }
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
