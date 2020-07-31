use async_tungstenite::tungstenite::Message as WsMessage;
use serde::{Deserialize, Serialize};
use serde_json::error::Error as JsonError;

#[derive(Debug, Deserialize, Serialize)]
pub enum Packet<'a> {
    #[serde(borrow)]
    Open(Open<'a>),
    Close,
    Ping,
    Pong,
    #[serde(borrow)]
    Message(Message<'a>),
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Open<'a> {
    sid: &'a str,
    ping_timeout: u64,
    ping_interval: u64,
}

#[derive(Debug, Deserialize, Serialize)]
pub enum Message<'a> {
    Text(&'a str),
    Binary(&'a [u8]),
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

    pub fn decode<'a>(&mut self, msg: &'a WsMessage) -> Result<Packet<'a>, Error> {
        use WsMessage::*;
        if self.state == State::Closed {
            return Err(Error::MessageAfterClose);
        }
        match msg {
            Ping(_) | Pong(_) | Close(_) => Err(Error::WrongMessageType(msg.clone())),
            Text(text) => self.decode_text(text),
            Binary(data) => self.decode_binary(&*data),
        }
    }

    fn decode_text<'a>(&mut self, text: &'a str) -> Result<Packet<'a>, Error> {
        let invalid_msg = || Error::InvalidMessage(WsMessage::Text(text.to_string()));
        let typ = text.as_bytes().first().ok_or_else(invalid_msg)?;
        match *typ as char {
            '0' => {
                if self.state != State::Initial {
                    Err(Error::SecondOpen)
                } else {
                    let result = parse_open(text)?;
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
            '4' => Ok(Packet::Message(Message::Text(&text[1..]))),
            _ => Err(invalid_msg()),
        }
    }

    fn decode_binary<'a>(&mut self, data: &'a [u8]) -> Result<Packet<'a>, Error> {
        let invalid_msg = || Error::InvalidMessage(WsMessage::Binary(data.to_vec()));
        if self.state == State::Initial {
            Err(Error::MessageBeforeOpen)
        } else if *data.first().ok_or_else(invalid_msg)? != 4 {
            Err(invalid_msg())
        } else {
            Ok(Packet::Message(Message::Binary(&data[1..])))
        }
    }
}

fn parse_open<'a>(text: &'a str) -> Result<Open<'a>, Error> {
    Ok(serde_json::from_str(text)?)
}
