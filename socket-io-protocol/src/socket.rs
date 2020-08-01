use std::borrow::Cow;

use regex::Regex;
use serde::Deserialize;
use serde_json::value::RawValue;

use super::engine::Message as EngineMessage;

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct Packet<'a> {
    pub data: PacketData<'a>,
    pub namespace: Option<Cow<'a, str>>,
}

#[derive(Debug, PartialEq)]
pub enum PacketKind {
    Connect,
    Disconnect,
    Event,
    Ack,
    BinaryEvent,
    BinaryAck,
}

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub enum PacketData<'a> {
    Connect,
    Disconnect,
    Event(Event<'a>),
    Ack(Ack<'a>),
    BinaryEvent(Event<'a>),
    BinaryAck(Ack<'a>),
}

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct Event<'a> {
    pub id: Option<u64>,
    pub data: Data<'a>,
}

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct Ack<'a> {
    pub id: u64,
    pub data: Data<'a>,
}

#[derive(Debug, Deserialize)]
#[serde(transparent)]
pub struct Data<'a>(#[serde(borrow)] Vec<Cow<'a, RawValue>>);

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct Partial<'a>(Parse<'a>);

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub enum DeserializeResult<'a> {
    Packet(Packet<'a>),
    DataNeeded(Partial<'a>),
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Received non-attachment binary message")]
    NonAttachmentBinary(Vec<u8>),
    #[error("Invalid message: {0}")]
    InvalidMessage(String),
    #[error("Invalid extra data included in {0} packet: {1}")]
    InvalidExtraData(&'static str, String),
}

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
struct Parse<'a> {
    kind: PacketKind,
    attachments: Option<u64>,
    namespace: Option<&'a str>,
    id: Option<u64>,
    data: Option<&'a str>,
}

lazy_static::lazy_static! {
    static ref DESERIALIZE_RE: Regex = {
        let pattern = r#"^([012356])((0|[1-9][0-9]*)-)?((/.+),)?(0|[1-9][0-9]*)?(\[.*\])?$"#;
        Regex::new(pattern).unwrap()
    };
}

pub fn deserialize<'a>(msg: EngineMessage<'a>) -> Result<DeserializeResult<'a>, Error> {
    match msg {
        EngineMessage::Text(text) => deserialize_text(text),
        EngineMessage::Binary(data) => Err(Error::NonAttachmentBinary(data.to_vec())),
    }
}

fn parse_text<'a>(text: &'a str) -> Option<Parse<'a>> {
    let captures = DESERIALIZE_RE.captures(text)?;
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
    let namespace = captures.get(5).map(|x| x.as_str());
    let id = captures.get(6).map(|x| x.as_str().parse::<u64>().unwrap());
    let data = captures.get(7).map(|x| x.as_str());
    Some(Parse {
        kind,
        attachments,
        namespace,
        id,
        data,
    })
}

fn deserialize_text<'a>(text: &'a str) -> Result<DeserializeResult<'a>, Error> {
    let parse = parse_text(text).ok_or_else(|| Error::InvalidMessage(text.to_string()))?;
    let dataless = |name, kind| {
        if parse.attachments.is_some() || parse.id.is_some() || parse.data.is_some() {
            Err(Error::InvalidExtraData(name, text.to_string()))
        } else {
            Ok(DeserializeResult::Packet(Packet {
                data: kind,
                namespace: parse.namespace.map(|x| x.into()),
            }))
        }
    };
    match parse.kind {
        PacketKind::Connect => dataless("connect", PacketData::Connect),
        PacketKind::Disconnect => dataless("disconnect", PacketData::Disconnect),
        _ => unimplemented!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl<'a, 'b> PartialEq<Data<'b>> for Data<'a> {
        fn eq(&self, other: &Data<'b>) -> bool {
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
                data: Some("[\"binary namespaced message with ack\"]"),
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
}
