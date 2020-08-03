use serde::Deserialize;
use serde_json::{value::Value, Error as JsonError};

use super::*;

mod deserialize;

use deserialize::Error as DeserializeError;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Placeholder object is missing num field: {0:?}")]
    NoNumInPlaceholderObject(Value),
    #[error("Placeholder index out of range: {0}/{1}")]
    PlaceholderIndexOutOfRange(u64, u64),
    #[error("Error deserializing json: {0}, {1}")]
    JsonError(String, JsonError),
    #[error("Error deserializing binary arguments: {0}")]
    BinaryError(#[from] DeserializeError),
}

pub trait Arg<'a> {
    fn to_json_value(&self) -> Result<Value, Error>;

    fn deserialize<T>(&self) -> Result<T, Error>
    where
        T: Deserialize<'a>;
}

pub struct TextArg<'a>(&'a Cow<'a, RawValue>);
pub struct BinaryArg<'a>(&'a Cow<'a, RawValue>, &'a [&'a [u8]]);

impl<'a> Args<'a> {
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn get<'b>(&'b self, idx: usize) -> Option<TextArg<'b>>
    where
        'a: 'b,
    {
        self.0.get(idx).map(|x| TextArg(x))
    }
}

impl<'a> BinaryArgs<'a> {
    pub fn len(&self) -> usize {
        self.args.len()
    }

    pub fn get<'b>(&'b self, idx: usize) -> Option<BinaryArg<'b>>
    where
        'a: 'b,
    {
        self.args
            .0
            .get(idx)
            .map(|x| BinaryArg(x, self.buffers.as_slice()))
    }
}

impl<'a> Arg<'a> for TextArg<'a> {
    fn to_json_value(&self) -> Result<Value, Error> {
        // We can unwrap because if the json was going to fail to deserialize we wouldn't have
        // gotten a RawValue
        Ok(serde_json::from_str(self.0.get()).unwrap())
    }

    fn deserialize<T>(&self) -> Result<T, Error>
    where
        T: Deserialize<'a>,
    {
        serde_json::from_str(self.0.get())
            .map_err(|err| Error::JsonError(self.0.get().to_string(), err))
    }
}

impl<'a> Arg<'a> for BinaryArg<'a> {
    fn to_json_value(&self) -> Result<Value, Error> {
        let mut json = serde_json::from_str(self.0.get()).unwrap();
        fill_placeholders_value(&mut json, self.1)?;
        Ok(json)
    }

    fn deserialize<T>(&self) -> Result<T, Error>
    where
        T: Deserialize<'a>,
    {
        Ok(deserialize::deserialize(self)?)
    }
}

fn fill_placeholders_value(value: &mut Value, buffers: &[&[u8]]) -> Result<(), Error> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_arg_value() {
        let m = "23[\"test\",\"hello\",{\"key\":\"value\"}]";
        let args = match deserialize(EngineMessage::Text(m))
            .unwrap()
            .packet()
            .unwrap()
            .data
        {
            PacketData::Event { args, .. } => args,
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
        let args = match deserialize(EngineMessage::Text(m))
            .unwrap()
            .packet()
            .unwrap()
            .data
        {
            PacketData::Event { args, .. } => args,
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
        let args = match deserialize(EngineMessage::Text(m))
            .unwrap()
            .packet()
            .unwrap()
            .data
        {
            PacketData::Event { args, .. } => args,
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
        let attachments = vec![EngineMessage::Binary(&*attachment)];

        let partial = match deserialize(EngineMessage::Text(m)).unwrap() {
            DeserializeResult::DataNeeded(partial) => partial,
            _ => unreachable!(),
        };

        let args = match deserialize_partial(partial, attachments).unwrap().data {
            PacketData::BinaryAck { args, .. } => args,
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
