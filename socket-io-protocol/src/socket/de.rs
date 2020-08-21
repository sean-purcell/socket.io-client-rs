use std::ops::Range;

use owned_subslice::OwnedSubslice;
use regex::Regex;
use serde_json::value::RawValue;

use super::{EngineMessage, Error, Kind, Packet, ProtocolKind};

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct Partial(Parse);

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub enum DeserializeResult {
    Packet(Packet),
    DataNeeded(Partial),
}

#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
struct Parse {
    message: OwnedSubslice<String>,
    kind: ProtocolKind,
    attachments: Option<u64>,
    namespace: Option<Range<usize>>,
    id: Option<u64>,
    args: Vec<Range<usize>>,
}

impl Partial {
    pub fn attachments(&self) -> u64 {
        self.0.attachments.unwrap()
    }
}

impl DeserializeResult {
    pub fn packet(self) -> Option<Packet> {
        match self {
            DeserializeResult::Packet(packet) => Some(packet),
            DeserializeResult::DataNeeded(_) => None,
        }
    }
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

fn parse_text(text: OwnedSubslice<String>) -> Result<Parse, Error> {
    let captures = DESERIALIZE_RE
        .captures(&*text)
        .ok_or_else(|| Error::InvalidMessage(text.to_string()))?;
    let kind = {
        use ProtocolKind::*;
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
    let parse = parse_text(text)?;

    match parse.kind {
        ProtocolKind::Connect => {
            deserialize_dataless(parse, Kind::Connect, "connect").map(DeserializeResult::Packet)
        }
        ProtocolKind::Disconnect => deserialize_dataless(parse, Kind::Disconnect, "disconnect")
            .map(DeserializeResult::Packet),
        ProtocolKind::Event => deserialize_event(parse, Kind::Event, "event", Vec::new())
            .map(DeserializeResult::Packet),
        ProtocolKind::Ack => {
            deserialize_event(parse, Kind::Ack, "ack", Vec::new()).map(DeserializeResult::Packet)
        }
        ProtocolKind::BinaryEvent => deserialize_binary(parse, Kind::Event, "binary event"),
        ProtocolKind::BinaryAck => deserialize_binary(parse, Kind::Ack, "binary ack"),
    }
}

fn deserialize_dataless(parse: Parse, kind: Kind, name: &'static str) -> Result<Packet, Error> {
    if parse.attachments.is_some() || parse.id.is_some() || !parse.args.is_empty() {
        return Err(Error::InvalidExtraData(name, parse.message.to_string()));
    }
    Ok(Packet {
        message: parse.message,
        kind,
        namespace: parse.namespace,
        id: None,
        args: Vec::new(),
        attachments: Vec::new(),
    })
}

fn deserialize_binary(
    parse: Parse,
    kind: Kind,
    name: &'static str,
) -> Result<DeserializeResult, Error> {
    if let Some(attachments) = parse.attachments {
        if attachments == 0 {
            deserialize_event(parse, kind, name, Vec::new()).map(DeserializeResult::Packet)
        } else {
            Ok(DeserializeResult::DataNeeded(Partial(parse)))
        }
    } else {
        Err(Error::MissingData(name, parse.message.to_string()))
    }
}

pub fn deserialize_partial(
    partial: Partial,
    attachments: impl IntoIterator<Item = EngineMessage>,
) -> Result<Packet, Error> {
    let Partial(parse) = partial;
    let attachments = attachments
        .into_iter()
        .map(|x| match x {
            EngineMessage::Text(text) => Err(Error::TextAttachment(text.to_string())),
            EngineMessage::Binary(data) => Ok(data),
        })
        .collect::<Result<_, _>>()?;
    match parse.kind {
        ProtocolKind::BinaryEvent => {
            deserialize_event(parse, Kind::Event, "binary event", attachments)
        }
        ProtocolKind::BinaryAck => deserialize_event(parse, Kind::Ack, "binary ack", attachments),
        _ => unreachable!(),
    }
}

fn deserialize_event(
    parse: Parse,
    kind: Kind,
    name: &'static str,
    attachments: Vec<OwnedSubslice<Vec<u8>>>,
) -> Result<Packet, Error> {
    if (kind == Kind::Ack && parse.id.is_none()) || parse.args.is_empty() {
        return Err(Error::MissingData(name, parse.message.to_string()));
    }
    if attachments.len() as u64 != parse.attachments.unwrap_or(0) {
        return Err(Error::InvalidAttachmentCount(
            attachments.len() as u64,
            parse.attachments.unwrap_or(0),
        ));
    }
    Ok(Packet {
        message: parse.message,
        kind,
        namespace: parse.namespace,
        id: parse.id,
        args: parse.args,
        attachments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(start: usize, end: usize) -> Range<usize> {
        Range { start, end }
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
        let parse = parse_text(m.to_string().into()).unwrap();
        assert_eq!(
            parse,
            Parse {
                message: m.to_string().into(),
                kind: ProtocolKind::BinaryEvent,
                attachments: Some(0),
                namespace: Some(range(3, 7)),
                id: Some(1),
                args: vec![range(10, 46)],
            }
        );
    }

    #[test]
    fn test_deserialize_connect() {
        let m = "0/nsp,";
        assert_eq!(
            deserialize(EngineMessage::Text(m.to_string().into())).unwrap(),
            DeserializeResult::Packet(Packet {
                message: m.to_string().into(),
                kind: Kind::Connect,
                namespace: Some(range(1, 5)),
                id: None,
                args: Vec::new(),
                attachments: Vec::new(),
            })
        );
    }

    #[test]
    fn test_deserialize_disconnect() {
        let m = "1/nsp,";
        assert_eq!(
            deserialize(EngineMessage::Text(m.to_string().into())).unwrap(),
            DeserializeResult::Packet(Packet {
                message: m.to_string().into(),
                kind: Kind::Disconnect,
                namespace: Some(range(1, 5)),
                id: None,
                args: Vec::new(),
                attachments: Vec::new(),
            })
        );
    }

    #[test]
    fn test_deserialize_event() {
        let m = "23[\"test\",\"hello\",{\"key\":\"value\"}]";
        assert_eq!(
            deserialize(EngineMessage::Text(m.to_string().into())).unwrap(),
            DeserializeResult::Packet(Packet {
                message: m.to_string().into(),
                kind: Kind::Event,
                namespace: None,
                id: Some(3),
                args: vec![range(3, 9), range(10, 17), range(18, 33)],
                attachments: Vec::new(),
            })
        );
    }

    #[test]
    fn test_deserialize_ack() {
        let m = "33[\"test\",\"hello\",{\"key\":\"value\"}]";
        assert_eq!(
            deserialize(EngineMessage::Text(m.to_string().into())).unwrap(),
            DeserializeResult::Packet(Packet {
                message: m.to_string().into(),
                kind: Kind::Ack,
                namespace: None,
                id: Some(3),
                args: vec![range(3, 9), range(10, 17), range(18, 33)],
                attachments: Vec::new(),
            })
        );
    }

    #[test]
    fn test_deserialize_binary_event() {
        let m = "51-[\"binary\",{\"_placeholder\":true,\"num\":0}]";
        let attachment = vec![222, 173, 190, 239];
        let attachments = vec![EngineMessage::Binary(attachment.clone().into())];

        let partial = match deserialize(EngineMessage::Text(m.to_string().into())).unwrap() {
            DeserializeResult::DataNeeded(partial) => partial,
            _ => unreachable!(),
        };

        let packet = deserialize_partial(partial, attachments.clone()).unwrap();

        assert_eq!(
            packet,
            Packet {
                message: m.to_string().into(),
                kind: Kind::Event,
                namespace: None,
                id: None,
                args: vec![range(4, 12), range(13, 42)],
                attachments: vec![attachment.into()],
            }
        );
    }

    #[test]
    fn test_deserialize_binary_ack() {
        let m = "61-10[\"binary\",{\"_placeholder\":true,\"num\":0}]";
        let attachment = vec![222, 173, 190, 239];
        let attachments = vec![EngineMessage::Binary(attachment.clone().into())];

        let partial = match deserialize(EngineMessage::Text(m.to_string().into())).unwrap() {
            DeserializeResult::DataNeeded(partial) => partial,
            _ => unreachable!(),
        };

        let packet = deserialize_partial(partial, attachments.clone()).unwrap();

        assert_eq!(
            packet,
            Packet {
                message: m.to_string().into(),
                kind: Kind::Ack,
                namespace: None,
                id: Some(10),
                args: vec![range(6, 14), range(15, 44)],
                attachments: vec![attachment.into()],
            }
        );
    }
}
