use std::fmt;
use std::ops::Range;

use owned_subslice::OwnedSubslice;
use serde_json::Error as JsonError;

use super::engine::Message as EngineMessage;

mod args;
mod parse;

pub use args::{Arg, Args};
pub use parse::{deserialize, deserialize_partial, DeserializeResult, Partial};

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct Packet {
    message: OwnedSubslice<String>,
    kind: Kind,
    namespace: Option<Range<usize>>,
    id: Option<u64>,
    args: Vec<Range<usize>>,
    attachments: Vec<OwnedSubslice<Vec<u8>>>,
}

#[derive(Debug, Copy, Clone, PartialEq)]
enum Kind {
    Connect,
    Disconnect,
    Event,
    Ack,
}

#[derive(Debug, Clone)]
pub enum Data<'a> {
    Connect,
    Disconnect,
    Event { id: Option<u64>, args: Args<'a> },
    Ack { id: u64, args: Args<'a> },
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

impl Packet {
    pub fn namespace(&self) -> Option<&str> {
        self.namespace.clone().map(|range| &self.message[range])
    }

    pub fn data(&self) -> Data<'_> {
        match self.kind {
            Kind::Connect => Data::Connect,
            Kind::Disconnect => Data::Disconnect,
            Kind::Event => Data::Event {
                id: self.id,
                args: self.args(),
            },
            Kind::Ack => Data::Ack {
                id: self.id.unwrap(),
                args: self.args(),
            },
        }
    }

    fn args(&self) -> Args<'_> {
        Args {
            message: &*self.message,
            args: self.args.as_slice(),
            attachments: self.attachments.as_slice(),
        }
    }
}

impl fmt::Display for Packet {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.data())
    }
}

impl<'a> fmt::Display for Data<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use Data::*;
        match self {
            Connect => write!(f, "Connect"),
            Disconnect => write!(f, "Disconnect"),
            Event { id, args } => write!(f, "Event {{ id: {:?}, args: {} }}", id, args),
            Ack { id, args } => write!(f, "Ack {{ id: {:?}, args: {} }}", id, args),
        }
    }
}
