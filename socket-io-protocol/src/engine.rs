use serde::{Deserialize, Serialize};
use serde_json::error::Error as JsonError;
use tungstenite::Message as WsMessage;

use owned_subslice::OwnedSubslice;

pub const MESSAGE_HEADER: char = '4';
pub const BINARY_HEADER: u8 = 4;

#[derive(Debug, Clone, PartialEq)]
pub enum Packet {
    Open(Open),
    Close,
    Ping,
    Pong,
    Message(Message),
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Open {
    sid: String,
    ping_timeout: u64,
    ping_interval: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Text(OwnedSubslice<String>),
    Binary(OwnedSubslice<Vec<u8>>),
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to parse websocket message: {0:?}")]
    InvalidMessage(WsMessage),
    #[error("Received non-Text, non-Binary websocket message: {0:?}")]
    WrongMessageType(WsMessage),
    #[error("Received message before open")]
    MessageBeforeOpen,
    #[error("Received message after close")]
    MessageAfterClose,
    #[error("Received second open")]
    SecondOpen,
    #[error("Failed to parse json in message: {0:?}")]
    JsonError(#[from] JsonError),
}

#[derive(Debug, PartialEq)]
enum State {
    Initial,
    Active,
    Closed,
}

#[derive(Debug)]
pub struct Decoder {
    state: State,
}

impl Default for Decoder {
    fn default() -> Self {
        Decoder {
            state: State::Initial,
        }
    }
}

impl Decoder {
    pub fn new() -> Decoder {
        Default::default()
    }

    pub fn decode(&mut self, msg: WsMessage) -> Result<Packet, Error> {
        use WsMessage::*;
        if self.state == State::Closed {
            return Err(Error::MessageAfterClose);
        }
        match msg {
            Ping(_) | Pong(_) | Close(_) => Err(Error::WrongMessageType(msg.clone())),
            Text(text) => self.decode_text(text),
            Binary(data) => self.decode_binary(data),
        }
    }

    fn decode_text(&mut self, text: String) -> Result<Packet, Error> {
        let invalid_msg = || Error::InvalidMessage(WsMessage::Text(text.to_string()));
        let typ = text.as_bytes().first().ok_or_else(invalid_msg)?;
        match *typ as char {
            '0' => {
                if self.state != State::Initial {
                    Err(Error::SecondOpen)
                } else {
                    let result = parse_open(&text[1..])?;
                    self.state = State::Active;
                    Ok(Packet::Open(result))
                }
            }
            '1' => {
                if self.state == State::Initial {
                    Err(Error::MessageBeforeOpen)
                } else {
                    self.state = State::Closed;
                    Ok(Packet::Close)
                }
            }
            '2' => {
                if self.state == State::Initial {
                    Err(Error::MessageBeforeOpen)
                } else {
                    Ok(Packet::Ping)
                }
            }
            '3' => {
                if self.state == State::Initial {
                    Err(Error::MessageBeforeOpen)
                } else {
                    Ok(Packet::Pong)
                }
            }
            '4' => {
                let len = text.len();
                Ok(Packet::Message(Message::Text(OwnedSubslice::new(
                    text,
                    1..len,
                ))))
            }
            _ => Err(invalid_msg()),
        }
    }

    fn decode_binary(&mut self, data: Vec<u8>) -> Result<Packet, Error> {
        let invalid_msg = || Error::InvalidMessage(WsMessage::Binary(data.to_vec()));
        if self.state == State::Initial {
            Err(Error::MessageBeforeOpen)
        } else if *data.get(0).ok_or_else(invalid_msg)? != 4 {
            Err(invalid_msg())
        } else {
            let len = data.len();
            Ok(Packet::Message(Message::Binary(OwnedSubslice::new(
                data,
                1..len,
            ))))
        }
    }
}

fn parse_open(text: &str) -> Result<Open, Error> {
    Ok(serde_json::from_str(text)?)
}

/// Creates a Message packet from the given text
pub fn encode_message(text: &str) -> WsMessage {
    package_message(format!("{}{}", MESSAGE_HEADER, text))
}

/// Creates a message from the given text without extra copies or allocations.  The first byte in
/// text must be equal to `MESSAGE_HEADER`, otherwise this function will panic.
pub fn package_message(text: String) -> WsMessage {
    assert_eq!(
        text.as_bytes().first(),
        Some(&(MESSAGE_HEADER as u8)),
        "Invalid header for message: {}",
        text
    );
    WsMessage::Text(text)
}

/// Creates a binary Message packet from the given data.
pub fn encode_binary(data: &[u8]) -> WsMessage {
    let mut vec = Vec::with_capacity(data.len() + 1);
    vec.push(4);
    vec.extend_from_slice(data);
    package_binary(vec)
}

/// Creates a message from the given text without extra copies or allocations.  The first byte in
/// data must be equal to `BINARY_HEADER`, otherwise this function will panic.
pub fn package_binary(data: Vec<u8>) -> WsMessage {
    assert_eq!(
        data.first(),
        Some(&BINARY_HEADER),
        "Invalid header for binary: {:?}",
        data
    );
    WsMessage::Binary(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_close() {
        let mut decoder = Decoder::new();

        assert!(decoder.decode(WsMessage::Close(None)).is_err());
    }

    #[test]
    fn decode_open() {
        let mut decoder = Decoder::new();

        let msg = WsMessage::Text(
            "0{\"sid\":\"0vtWsEAcESDOoPs8AAAA\",\"upgrades\":[],\"pingInterval\":25000,\"pingTimeout\":5000}".to_string());
        let packet = decoder.decode(msg.clone()).unwrap();
        let expected = Packet::Open(Open {
            sid: String::from("0vtWsEAcESDOoPs8AAAA"),
            ping_interval: 25000,
            ping_timeout: 5000,
        });
        assert_eq!(packet, expected);
        let result = decoder.decode(msg);
        assert!(result.is_err());
    }
}
