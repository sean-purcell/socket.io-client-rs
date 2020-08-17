use std::io::{Cursor, Write};

use serde::Serialize;
use tungstenite::Message as WsMessage;

use crate::engine::{self, Message as EngineMessage, MESSAGE_HEADER as ENGINE_MESSAGE_HEADER};

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
        builder
            .serialize_arg(event)
            .expect("Serialization of &str failed");
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
        T: Serialize + ?Sized,
    {
        let start_pos = self.buffer.len();
        let mut cursor = Cursor::new(&mut self.buffer);
        cursor.set_position(start_pos as u64);
        if self.first {
            write!(cursor, "[").unwrap();
            self.first = false;
        } else {
            write!(cursor, ",").unwrap();
        }
        let result = match &mut self.approach {
            Approach::Normal => args::serialize_arg(cursor, arg),
            Approach::Binary { attachments, .. } => {
                let attachment_start = attachments.len();
                let result = args::serialize_arg_binary(cursor, arg, attachments);
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

    pub fn finish(self) -> Vec<WsMessage> {
        // This is safe because we've only written to this via write!, and json serialization
        let mut s = unsafe { String::from_utf8_unchecked(self.buffer) };
        if !self.first {
            s.push(']');
        }
        match self.approach {
            Approach::Normal => vec![engine::package_message(s)],
            Approach::Binary {
                kind,
                namespace,
                id,
                mut attachments,
            } => {
                // Create the header
                let mut header = serialize_header(
                    kind,
                    Some(attachments.len() as u64),
                    namespace.as_ref().map(|x| x.as_str()),
                    id,
                );
                header.push_str(s.as_str());
                attachments.insert(0, engine::package_message(header));
                attachments
            }
        }
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

    #[test]
    fn test_simple() {
        let packet = PacketBuilder::new_event("event", None, None, false).finish();
        assert_eq!(packet, vec![WsMessage::Text(r#"42["event"]"#.to_string())]);
    }

    #[test]
    fn test_simple_binary() {
        let data = [0xdeu8, 0xad, 0xbe, 0xef];
        let mut builder = PacketBuilder::new_ack(Some("/binary"), 3, true);
        builder.serialize_arg(&data[..]).unwrap();
        let packet = builder.finish();
        assert_eq!(
            packet,
            vec![
                WsMessage::Text(r#"461-/binary,3[{"_placeholder":true,"num":0}]"#.to_string()),
                WsMessage::Binary(vec![4, 0xde, 0xad, 0xbe, 0xef])
            ]
        );
    }
}
