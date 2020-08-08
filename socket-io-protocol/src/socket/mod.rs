use std::{borrow::Cow, ops::Range};

use regex::Regex;
use serde::{Deserialize, Deserializer};
use serde_json::{value::RawValue, Error as JsonError};

use super::engine::Message as EngineMessage;

pub mod arg;

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct Packet {
    message: OwnedSubslice<String>,
    kind: Kind,
    namespace: Range<usize>,
    args: Vec<Range<usize>>,
}

pub struct Kind {
    Connect,
    Disconnect,
    Event,
    Ack,
}

#[derive(Copy, Clone, Debug, PartialEq)]
enum PacketKind {
    Connect,
    Disconnect,
    Event,
    Ack,
    BinaryEvent,
    BinaryAck,
}

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
enum PacketData<'a> {
    Connect,
    Disconnect,
    Event {
        id: Option<u64>,
        args: Args<'a>,
    },
    Ack {
        id: u64,
        args: Args<'a>,
    },
    BinaryEvent {
        id: Option<u64>,
        args: BinaryArgs<'a>,
    },
    BinaryAck {
        id: u64,
        args: BinaryArgs<'a>,
    },
}

#[derive(Debug)]
pub struct Args<'a>(Vec<Cow<'a, RawValue>>);

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct BinaryArgs<'a> {
    pub args: Args<'a>,
    pub buffers: Vec<&'a [u8]>,
}

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct Partial(Parse);

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub enum DeserializeResult<'a> {
    Packet(Packet<'a>),
    DataNeeded(Partial<'a>),
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Received non-attachment binary message: {0:?}")]
    NonAttachmentBinary(Vec<u8>),
    #[error("Received text message as attachment: {0}")]
    TextAttachment(String),
    #[error("Invalid message: {0}")]
    InvalidMessage(String),
    #[error("Invalid extra data included in {0} packet: {1}")]
    InvalidExtraData(&'static str, String),
    #[error("Missing data in {0} packet: {1}")]
    MissingData(&'static str, String),
    #[error("Invalid json in message data: {0}, {1:?}")]
    InvalidDataJson(String, JsonError),
    #[error("Wrong number of attachments provided: {0} instead of {1}")]
    InvalidAttachmentCount(u64, u64),
}

#[derive(Copy, Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
struct Parse {
    message: OwnedSubslice<String>,
    kind: PacketKind,
    attachments: Option<u64>,
    namespace: Option<Range<usize>>,
    id: Option<u64>,
    args: Option<Range<usize>>,
}

lazy_static::lazy_static! {
    static ref DESERIALIZE_RE: Regex = {
        let pattern = r#"^([012356])((0|[1-9][0-9]*)-)?((/.+),)?(0|[1-9][0-9]*)?(\[.*\])?$"#;
        Regex::new(pattern).unwrap()
    };
}

pub fn deserialize(msg: EngineMessage) -> Result<DeserializeResult, Error> {
    match msg {
        EngineMessage::Text(text) => deserialize_text(text),
        EngineMessage::Binary(data) => Err(Error::NonAttachmentBinary(data.to_vec())),
    }
}

fn parse_text(text: OwnedSubslice<String>) -> Option<Parse> {
    let captures = DESERIALIZE_RE.captures(&*text)?;
    let kind = {
        use PacketKind::*;
        match *captures
            .get(1)
            .unwrap()
            .as_str()
            .as_bytes()
            .first()
            .unwrap() as char
        {
            '0' => Connect,
            '1' => Disconnect,
            '2' => Event,
            '3' => Ack,
            '5' => BinaryEvent,
            '6' => BinaryAck,
            _ => unreachable!(),
        }
    };
    let attachments = captures.get(3).map(|x| x.as_str().parse::<u64>().unwrap());
    let namespace = captures.get(5).map(|x| x.range());
    let id = captures.get(6).map(|x| x.as_str().parse::<u64>().unwrap());
    let args = captures.get(7).map(|x| x.range());
    Some(Parse {
        message: text,
        kind,
        attachments,
        namespace,
        id,
        args,
    })
}

fn deserialize_text(text: OwnedSubslice<String>) -> Result<DeserializeResult, Error> {
    let parse = parse_text(text).ok_or_else(|| Error::InvalidMessage(text.to_string()))?;
    let namespace = || parse.namespace.map(|x| parse.message[x].to_string());
    let argsless = |name, kind| {
        if parse.attachments.is_some() || parse.id.is_some() || parse.args.is_some() {
            Err(Error::InvalidExtraData(name, text.to_string()))
        } else {
            Ok(DeserializeResult::Packet(Packet {
                data: kind,
                namespace: namespace(),
            }))
        }
    };
    let normal = |name, ack| {
        if parse.attachments.is_some() {
            Err(Error::InvalidExtraData(name, text.to_string()))
        } else if parse.args.is_none() {
            Err(Error::MissingData(name, text.to_string()))
        } else if ack && parse.id.is_none() {
            Err(Error::MissingData(name, text.to_string()))
        } else {
            let args = deserialize_args(parse.args.unwrap())?;
            let data = if !ack {
                PacketData::Event { id: parse.id, args }
            } else {
                PacketData::Ack {
                    id: parse.id.unwrap(),
                    args,
                }
            };
            Ok(DeserializeResult::Packet(Packet {
                data,
                namespace: namespace(),
            }))
        }
    };
    let binary = |name, ack| {
        if ack && parse.id.is_none() {
            Err(Error::MissingData(name, text.to_string()))
        } else if parse.attachments.is_none() || parse.args.is_none() {
            Err(Error::MissingData(name, text.to_string()))
        } else {
            let attachments = parse.attachments.unwrap();
            if attachments == 0 {
                Ok(DeserializeResult::Packet(deserialize_binary(
                    parse,
                    Vec::new(),
                )?))
            } else {
                Ok(DeserializeResult::DataNeeded(Partial(parse)))
            }
        }
    };
    match parse.kind {
        PacketKind::Connect => argsless("connect", PacketData::Connect),
        PacketKind::Disconnect => argsless("disconnect", PacketData::Disconnect),
        PacketKind::Event => normal("event", false),
        PacketKind::Ack => normal("ack", true),
        PacketKind::BinaryEvent => binary("binary event", false),
        PacketKind::BinaryAck => binary("binary ack", true),
    }
}

fn deserialize_args<'a>(args: &'a str) -> Result<Args<'a>, Error> {
    serde_json::from_str(args).map_err(|err| Error::InvalidDataJson(args.to_string(), err))
}

pub fn deserialize_partial<'a>(
    partial: Partial<'a>,
    buffers: impl IntoIterator<Item = EngineMessage<'a>>,
) -> Result<Packet<'a>, Error> {
    let Partial(parse) = partial;
    let buffers = buffers
        .into_iter()
        .map(|x| match x {
            EngineMessage::Text(text) => Err(Error::TextAttachment(text.to_string())),
            EngineMessage::Binary(data) => Ok(data),
        })
        .collect::<Result<_, _>>()?;
    deserialize_binary(parse, buffers)
}

fn deserialize_binary<'a>(parse: Parse, buffers: Vec<&'a [u8]>) -> Result<Packet<'a>, Error> {
    let actual = buffers.len() as u64;
    let expected = parse.attachments.unwrap();
    if actual != expected {
        Err(Error::InvalidAttachmentCount(actual, expected))
    } else {
        let args = deserialize_args(parse.args.unwrap())?;
        let args = BinaryArgs { args, buffers };
        let data = match parse.kind {
            PacketKind::BinaryEvent => PacketData::BinaryEvent { id: parse.id, args },
            PacketKind::BinaryAck => PacketData::BinaryAck {
                id: parse.id.unwrap(),
                args,
            },
            _ => unreachable!(),
        };
        Ok(Packet {
            data,
            namespace: parse.namespace.map(|x| x.into()),
        })
    }
}

impl<'a> DeserializeResult<'a> {
    pub fn packet(self) -> Option<Packet<'a>> {
        match self {
            DeserializeResult::Packet(packet) => Some(packet),
            DeserializeResult::DataNeeded(_) => None,
        }
    }
}

impl Partial {
    pub fn attachments(&self) -> u64 {
        self.0.attachments.unwrap()
    }
}

impl<'de> Deserialize<'de> for Args<'de> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let borroweds = <Vec<&'de RawValue> as Deserialize>::deserialize(deserializer)?;
        Ok(Args(borroweds.into_iter().map(Cow::Borrowed).collect()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl<'a, 'b> PartialEq<Args<'b>> for Args<'a> {
        fn eq(&self, other: &Args<'b>) -> bool {
            if self.0.len() != other.0.len() {
                return false;
            }
            self.0
                .iter()
                .zip(other.0.iter())
                .all(|(r0, r1)| r0.get() == r1.get())
        }
    }

    #[test]
    fn test_deserialize_re() {
        let m0 = "0/nsp,";
        let m1 =
            "52-[\"binary\",{\"_placeholder\":true,\"num\":0},{\"_placeholder\":true,\"num\":1}]";
        let m2 = "20[\"types\",[0,1,2],{\"key\":\"value\"},\"hello\",4]";
        let m3 = "50-/nsp,1[\"binary namespaced message with ack\"]";

        let c0 = DESERIALIZE_RE.captures(m0).unwrap();
        let c1 = DESERIALIZE_RE.captures(m1).unwrap();
        let c2 = DESERIALIZE_RE.captures(m2).unwrap();
        let c3 = DESERIALIZE_RE.captures(m3).unwrap();

        assert_eq!(c0.get(1).unwrap().as_str(), "0");
        assert_eq!(c0.get(5).unwrap().as_str(), "/nsp");

        assert_eq!(c1.get(1).unwrap().as_str(), "5");
        assert_eq!(c1.get(3).unwrap().as_str(), "2");

        assert_eq!(c2.get(1).unwrap().as_str(), "2");
        assert_eq!(c2.get(6).unwrap().as_str(), "0");
        assert_eq!(
            c2.get(7).unwrap().as_str(),
            "[\"types\",[0,1,2],{\"key\":\"value\"},\"hello\",4]"
        );

        assert_eq!(c3.get(1).unwrap().as_str(), "5");
        assert_eq!(c3.get(3).unwrap().as_str(), "0");
        assert_eq!(c3.get(5).unwrap().as_str(), "/nsp");
        assert_eq!(c3.get(6).unwrap().as_str(), "1");
        assert_eq!(
            c3.get(7).unwrap().as_str(),
            "[\"binary namespaced message with ack\"]"
        );
    }

    #[test]
    fn test_parse_text() {
        let m = "50-/nsp,1[\"binary namespaced message with ack\"]";
        let parse = parse_text(m).unwrap();
        assert_eq!(
            parse,
            Parse {
                kind: PacketKind::BinaryEvent,
                attachments: Some(0),
                namespace: Some("/nsp"),
                id: Some(1),
                args: Some("[\"binary namespaced message with ack\"]"),
            }
        );
    }

    #[test]
    fn test_deserialize_connect() {
        let m = "0/nsp,";
        assert_eq!(
            deserialize(EngineMessage::Text(m)).unwrap(),
            DeserializeResult::Packet(Packet {
                data: PacketData::Connect,
                namespace: Some("/nsp".into())
            })
        );
    }

    #[test]
    fn test_deserialize_disconnect() {
        let m = "1/nsp,";
        assert_eq!(
            deserialize(EngineMessage::Text(m)).unwrap(),
            DeserializeResult::Packet(Packet {
                data: PacketData::Disconnect,
                namespace: Some("/nsp".into())
            })
        );
    }

    #[test]
    fn test_deserialize_event() {
        let m = "23[\"test\",\"hello\",{\"key\":\"value\"}]";
        let arg_strs = vec!["\"test\"", "\"hello\"", "{\"key\":\"value\"}"];
        let args = Args(
            arg_strs
                .iter()
                .map(|x| RawValue::from_string(x.to_string()).unwrap())
                .map(Cow::Owned)
                .collect(),
        );
        assert_eq!(
            deserialize(EngineMessage::Text(m)).unwrap(),
            DeserializeResult::Packet(Packet {
                data: PacketData::Event { id: Some(3), args },
                namespace: None,
            })
        );
    }

    #[test]
    fn test_deserialize_ack() {
        let m = "33[\"test\",\"hello\",{\"key\":\"value\"}]";
        let arg_strs = vec!["\"test\"", "\"hello\"", "{\"key\":\"value\"}"];
        let args = Args(
            arg_strs
                .iter()
                .map(|x| RawValue::from_string(x.to_string()).unwrap())
                .map(Cow::Owned)
                .collect(),
        );
        assert_eq!(
            deserialize(EngineMessage::Text(m)).unwrap(),
            DeserializeResult::Packet(Packet {
                data: PacketData::Ack { id: 3, args },
                namespace: None,
            })
        );
    }

    #[test]
    fn test_deserialize_binary_event() {
        let m = "51-[\"binary\",{\"_placeholder\":true,\"num\":0}]";
        let attachment = vec![222, 173, 190, 239];
        let attachments = vec![EngineMessage::Binary(&*attachment)];
        let arg_strs = vec!["\"binary\"", "{\"_placeholder\":true,\"num\":0}"];

        let partial = match deserialize(EngineMessage::Text(m)).unwrap() {
            DeserializeResult::DataNeeded(partial) => partial,
            _ => unreachable!(),
        };

        let packet = deserialize_partial(partial, attachments).unwrap();

        let args = Args(
            arg_strs
                .iter()
                .map(|x| RawValue::from_string(x.to_string()).unwrap())
                .map(Cow::Owned)
                .collect(),
        );
        let buffers = vec![&*attachment];
        assert_eq!(
            packet,
            Packet {
                data: PacketData::BinaryEvent {
                    id: None,
                    args: BinaryArgs { args, buffers },
                },
                namespace: None
            }
        );
    }

    #[test]
    fn test_deserialize_binary_ack() {
        let m = "61-10[\"binary\",{\"_placeholder\":true,\"num\":0}]";
        let attachment = vec![222, 173, 190, 239];
        let attachments = vec![EngineMessage::Binary(&*attachment)];
        let arg_strs = vec!["\"binary\"", "{\"_placeholder\":true,\"num\":0}"];

        let partial = match deserialize(EngineMessage::Text(m)).unwrap() {
            DeserializeResult::DataNeeded(partial) => partial,
            _ => unreachable!(),
        };

        let packet = deserialize_partial(partial, attachments).unwrap();

        let args = Args(
            arg_strs
                .iter()
                .map(|x| RawValue::from_string(x.to_string()).unwrap())
                .map(Cow::Owned)
                .collect(),
        );
        let buffers = vec![&*attachment];
        assert_eq!(
            packet,
            Packet {
                data: PacketData::BinaryAck {
                    id: 10,
                    args: BinaryArgs { args, buffers },
                },
                namespace: None
            }
        );
    }

    #[test]
    fn test_deserialize_zero_copy() {
        let m = "33[\"test\",\"hello\",{\"key\":\"value\"}]";
        let args = match deserialize(EngineMessage::Text(m))
            .unwrap()
            .packet()
            .unwrap()
            .data
        {
            PacketData::Ack { args, .. } => args,
            _ => unreachable!(),
        };

        for arg in args.0 {
            match arg {
                Cow::Borrowed(_) => (),
                Cow::Owned(_) => panic!(
                    "Data was copied when it could have been borrowed: {:?}",
                    arg
                ),
            }
        }
    }
}
