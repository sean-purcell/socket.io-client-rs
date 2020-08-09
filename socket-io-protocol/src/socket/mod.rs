use std::ops::Range;

use owned_subslice::OwnedSubslice;
use regex::Regex;
use serde::Deserialize;
use serde_json::{value::RawValue, Error as JsonError};

use super::engine::Message as EngineMessage;

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct Packet {
    message: OwnedSubslice<String>,
    kind: Kind,
    namespace: Range<usize>,
    args: Vec<Range<usize>>,
    placeholders: Vec<OwnedSubslice<Vec<u8>>>,
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum Kind {
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
pub struct Partial(Parse);

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub enum DeserializeResult {
    Packet(Packet),
    DataNeeded(Partial),
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

#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
struct Parse {
    message: OwnedSubslice<String>,
    kind: PacketKind,
    attachments: Option<u64>,
    namespace: Option<Range<usize>>,
    id: Option<u64>,
    args: Vec<Range<usize>>,
}

#[derive(Debug)]
struct ArgRange(Range<usize>);

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

fn parse_text(text: OwnedSubslice<String>) -> Result<Parse, Error> {
    let captures = DESERIALIZE_RE
        .captures(&*text)
        .ok_or_else(|| Error::InvalidMessage(text.to_string()))?;
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
    let args = match captures.get(7) {
        Some(m) => {
            let mut args = parse_args(m.as_str())?;
            let offset = m.start();
            args.iter_mut().for_each(|Range { start, end }| {
                *start += offset;
                *end += offset
            });
            args
        }
        None => Vec::new(),
    };

    Ok(Parse {
        message: text,
        kind,
        attachments,
        namespace,
        id,
        args,
    })
}

fn parse_args<'a>(arg_str: &'a str) -> Result<Vec<Range<usize>>, Error> {
    let json_err = |e| Error::InvalidDataJson(arg_str.to_string(), e);
    let args: Vec<&'a RawValue> = serde_json::from_str(arg_str).map_err(json_err)?;

    Ok(args
        .iter()
        .map(|x| {
            let start = x.get().as_ptr() as usize - arg_str.as_ptr() as usize;
            let end = start + x.get().len();
            Range { start, end }
        })
        .collect())
}

fn deserialize_text(text: OwnedSubslice<String>) -> Result<DeserializeResult, Error> {
    unimplemented!()
}

pub fn deserialize_partial(
    partial: Partial,
    buffers: impl IntoIterator<Item = EngineMessage>,
) -> Result<Packet, Error> {
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

fn deserialize_binary(parse: Parse, buffers: Vec<OwnedSubslice<Vec<u8>>>) -> Result<Packet, Error> {
    unimplemented!()
}

impl DeserializeResult {
    pub fn packet(self) -> Option<Packet> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
