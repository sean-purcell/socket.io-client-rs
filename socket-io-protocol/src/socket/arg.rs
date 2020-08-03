use serde::Deserialize;
use serde_json::{value::Value, Error as JsonError};

use super::*;

pub trait Arg<'a> {
    fn to_json_value(&self) -> Value;

    fn deserialize<T>(&self) -> Result<T, JsonError>
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
    fn to_json_value(&self) -> Value {
        serde_json::from_str(self.0.get()).unwrap()
    }

    fn deserialize<T>(&self) -> Result<T, JsonError>
    where
        T: Deserialize<'a>,
    {
        serde_json::from_str(self.0.get())
    }
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
            args.get(2).unwrap().to_json_value(),
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
}
