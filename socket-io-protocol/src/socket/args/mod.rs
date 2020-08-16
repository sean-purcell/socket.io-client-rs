use std::{fmt, io::Write};

use owned_subslice::OwnedSubslice;
use serde::{Deserialize, Serialize};
use serde_json::{value::Value, Error as JsonError};

use super::*;

mod deserialize_attachments;
mod serialize_attachments;

#[derive(Debug, Clone)]
pub struct Args<'a> {
    pub(super) message: &'a str,
    pub(super) args: &'a [Range<usize>],
    pub(super) attachments: &'a [OwnedSubslice<Vec<u8>>],
}

#[derive(Debug, Clone)]
pub struct Arg<'a> {
    arg: &'a str,
    attachments: &'a [OwnedSubslice<Vec<u8>>],
}

struct ArgsIter<'a> {
    args: &'a Args<'a>,
    idx: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Placeholder object is missing num field: {0:?}")]
    NoNumInPlaceholderObject(Value),
    #[error("Placeholder index out of range: {0}/{1}")]
    PlaceholderIndexOutOfRange(u64, u64),
    #[error("Error deserializing json: {0}, {1}")]
    JsonDeError(String, JsonError),
    #[error("Error serializing object to json: {0}")]
    JsonSerError(JsonError),
}

impl<'a> Args<'a> {
    pub fn len(&self) -> usize {
        self.args.len()
    }

    pub fn get(&self, idx: usize) -> Option<Arg<'a>> {
        self.args.get(idx).map(|range| Arg {
            arg: &self.message[range.clone()],
            attachments: self.attachments,
        })
    }

    pub fn iter<'b>(&'b self) -> impl Iterator<Item = Arg<'b>> {
        ArgsIter { args: self, idx: 0 }
    }
}

impl<'a> Arg<'a> {
    pub fn to_json_value(&self) -> Result<Value, Error> {
        // We can unwrap because if the json was going to fail to deserialize we would have failed
        // to parse
        let mut value = serde_json::from_str(self.arg).unwrap();
        if self.attachments.len() > 0 {
            fill_placeholders_value(&mut value, &self.attachments)?;
        }
        Ok(value)
    }

    pub fn deserialize<T>(&self) -> Result<T, Error>
    where
        T: Deserialize<'a>,
    {
        (if self.attachments.len() > 0 {
            deserialize_attachments::deserialize(self.arg, self.attachments)
        } else {
            serde_json::from_str(self.arg)
        })
        .map_err(|err| Error::JsonDeError(self.arg.to_string(), err))
    }
}

fn fill_placeholders_value(
    value: &mut Value,
    buffers: &[OwnedSubslice<Vec<u8>>],
) -> Result<(), Error> {
    use Value::*;

    let idx = match value {
        Null | Bool(_) | Number(_) | String(_) => return Ok(()),
        Array(values) => {
            return values
                .iter_mut()
                .map(|x| fill_placeholders_value(x, buffers))
                .collect();
        }
        Object(map) => {
            // Determine if it's a placeholder
            if map.contains_key("_placeholder") {
                map.get("num")
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| Error::NoNumInPlaceholderObject(value.clone()))?
            } else {
                return map
                    .values_mut()
                    .map(|x| fill_placeholders_value(x, buffers))
                    .collect();
            }
        }
    };
    let buffer = buffers
        .get(idx as usize)
        .ok_or_else(|| Error::PlaceholderIndexOutOfRange(idx, buffers.len() as u64))?;
    *value = Value::Array(buffer.iter().copied().map(|x| x.into()).collect());
    Ok(())
}

pub fn serialize_arg(writer: impl Write, arg: &(impl Serialize + ?Sized)) -> Result<(), Error> {
    serde_json::to_writer(writer, arg).map_err(Error::JsonSerError)
}

impl<'a> Iterator for ArgsIter<'a> {
    type Item = Arg<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.args.get(self.idx);
        self.idx += 1;
        item
    }
}

impl<'a> fmt::Display for Args<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "[")?;
        let mut first = true;
        for arg in self.iter() {
            if !first {
                write!(f, ", ")?;
            }
            first = false;
            write!(f, "{}", arg)?;
        }
        write!(f, "]")
    }
}

impl<'a> fmt::Display for Arg<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.to_json_value().unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_arg_value() {
        let m = "23[\"test\",\"hello\",{\"key\":\"value\"}]";
        let packet = deserialize(EngineMessage::Text(m.to_string().into()))
            .unwrap()
            .packet()
            .unwrap();
        let args = match packet.data() {
            Data::Event { args, .. } => args,
            _ => unreachable!(),
        };

        assert_eq!(
            args.get(2).unwrap().to_json_value().unwrap(),
            Value::Object(
                [("key".to_string(), Value::String("value".to_string()))]
                    .iter()
                    .cloned()
                    .collect()
            )
        );
    }

    #[derive(Debug, PartialEq, Deserialize)]
    struct StructBorrowed<'a> {
        key: &'a str,
    }

    #[test]
    fn test_text_arg_deserialize_borrowed() {
        let m = "23[\"test\",\"hello\",{\"key\":\"value\"}]";
        let packet = deserialize(EngineMessage::Text(m.to_string().into()))
            .unwrap()
            .packet()
            .unwrap();
        let args = match packet.data() {
            Data::Event { args, .. } => args,
            _ => unreachable!(),
        };

        assert_eq!(
            args.get(2)
                .unwrap()
                .deserialize::<StructBorrowed>()
                .unwrap(),
            StructBorrowed { key: "value" }
        );
    }

    #[derive(Debug, PartialEq, Deserialize)]
    struct StructOwned {
        key: String,
    }

    #[test]
    fn test_text_arg_deserialize_owned() {
        let m = "23[\"test\",\"hello\",{\"key\":\"value\"}]";
        let packet = deserialize(EngineMessage::Text(m.to_string().into()))
            .unwrap()
            .packet()
            .unwrap();
        let args = match packet.data() {
            Data::Event { args, .. } => args,
            _ => unreachable!(),
        };

        assert_eq!(
            args.get(2).unwrap().deserialize::<StructOwned>().unwrap(),
            StructOwned {
                key: "value".to_string()
            }
        );
    }

    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct BinaryBorrowed<'a> {
        array: &'a [u8],
    }

    #[test]
    fn test_deserialize_binary_ack() {
        let m = "61-10[\"binary\",{\"array\": {\"_placeholder\":true,\"num\":0}}]";
        let attachment = vec![222, 173, 190, 239];
        let attachments = vec![EngineMessage::Binary(attachment.into())];

        let partial = match deserialize(EngineMessage::Text(m.to_string().into())).unwrap() {
            DeserializeResult::DataNeeded(partial) => partial,
            _ => unreachable!(),
        };

        let packet = deserialize_partial(partial, attachments).unwrap();
        let args = match packet.data() {
            Data::Ack { args, .. } => args,
            _ => unreachable!(),
        };

        assert_eq!(
            args.get(1).unwrap().to_json_value().unwrap(),
            Value::Object(
                [(
                    "array".to_string(),
                    Value::Array(vec![222.into(), 173.into(), 190.into(), 239.into()])
                )]
                .iter()
                .cloned()
                .collect()
            )
        );
    }
}
