use std::io::{Cursor, Write};

use serde::Serialize;
use tungstenite::Message as WsMessage;

use crate::engine::{Message as EngineMessage, MESSAGE_HEADER as ENGINE_MESSAGE_HEADER};

use super::{args, ArgsError, ProtocolKind};

pub struct PacketBuilder {
    buffer: Vec<u8>,
    approach: Approach,
    first: bool,
}

enum Approach {
    Normal,
    Binary {
        kind: ProtocolKind,
        namespace: Option<String>,
        id: Option<u64>,
        attachments: Vec<WsMessage>,
    },
}

impl PacketBuilder {
    pub fn new_event(event: &str, namespace: Option<&str>, id: Option<u64>, binary: bool) -> Self {
        let mut builder = if !binary {
            let buffer = serialize_header(ProtocolKind::Event, None, namespace, id).into_bytes();
            PacketBuilder {
                buffer,
                approach: Approach::Normal,
                first: true,
            }
        } else {
            let buffer = Vec::new();
            let namespace = namespace.map(|x| x.to_string());
            PacketBuilder {
                buffer,
                approach: Approach::Binary {
                    kind: ProtocolKind::BinaryEvent,
                    namespace,
                    id,
                    attachments: Vec::new(),
                },
                first: true,
            }
        };
        //FIXME: Add event to parameters
        builder
    }

    pub fn new_ack(namespace: Option<&str>, id: u64, binary: bool) -> Self {
        if !binary {
            let buffer =
                serialize_header(ProtocolKind::Ack, None, namespace, Some(id)).into_bytes();
            PacketBuilder {
                buffer,
                approach: Approach::Normal,
                first: true,
            }
        } else {
            let buffer = Vec::new();
            let namespace = namespace.map(|x| x.to_string());
            PacketBuilder {
                buffer,
                approach: Approach::Binary {
                    kind: ProtocolKind::BinaryAck,
                    namespace,
                    id: Some(id),
                    attachments: Vec::new(),
                },
                first: true,
            }
        }
    }

    /// Serialize the given argument using its `Serialize` implementation.  Fails if `T`'s
    /// implementation of `Serialize` decides to fail, or if `T` contains a map with non-string
    /// keys.  If serialization fails, the internal state will be unchanged.
    pub fn serialize_arg<T>(&mut self, arg: &T) -> Result<(), ArgsError>
    where
        T: Serialize,
    {
        let start_pos = self.buffer.len();
        let mut cursor = Cursor::new(&mut self.buffer);
        cursor.set_position(start_pos as u64);
        write!(cursor, ",").unwrap();
        let result = match &mut self.approach {
            Approach::Normal => args::serialize_arg(cursor, arg),
            Approach::Binary { attachments, .. } => {
                let attachment_start = attachments.len();
                let result: Result<(), ArgsError> = unimplemented!();
                if result.is_err() {
                    attachments.resize_with(attachment_start, || panic!("shrinking vector"));
                }
                result
            }
        };
        if result.is_err() {
            self.buffer
                .resize_with(start_pos, || panic!("shrinking vector"));
        }
        result
    }
}

pub fn serialize_connect(namespace: Option<&str>) -> EngineMessage {
    EngineMessage::Text(serialize_header(ProtocolKind::Connect, None, namespace, None).into())
}

pub fn serialize_disconnect(namespace: Option<&str>) -> EngineMessage {
    EngineMessage::Text(serialize_header(ProtocolKind::Disconnect, None, namespace, None).into())
}

fn serialize_header(
    kind: ProtocolKind,
    attachments: Option<u64>,
    namespace: Option<&str>,
    id: Option<u64>,
) -> String {
    let mut header = vec![ENGINE_MESSAGE_HEADER as u8];
    let kind = match kind {
        ProtocolKind::Connect => '0',
        ProtocolKind::Disconnect => '1',
        ProtocolKind::Event => '2',
        ProtocolKind::Ack => '3',
        ProtocolKind::BinaryEvent => '5',
        ProtocolKind::BinaryAck => '6',
    };
    write!(header, "{}", kind).unwrap();
    if let Some(attachments) = attachments {
        write!(header, "{}-", attachments).unwrap();
    }
    if let Some(namespace) = namespace {
        write!(header, "{},", namespace).unwrap();
    }
    if let Some(id) = id {
        write!(header, "{}", id).unwrap();
    }
    unsafe { String::from_utf8_unchecked(header) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connect() {
        assert_eq!(
            serialize_connect(None),
            EngineMessage::Text("40".to_string().into())
        );
    }

    #[test]
    fn test_disconnect() {
        assert_eq!(
            serialize_disconnect(Some("/nsp")),
            EngineMessage::Text("41/nsp,".to_string().into())
        );
    }
}
