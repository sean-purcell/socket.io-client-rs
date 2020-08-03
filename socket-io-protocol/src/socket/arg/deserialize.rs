use std::fmt::Display;

use paste::paste;
use serde::{
    de::{Error as DeError, Visitor},
    Deserialize, Deserializer,
};
use serde_json::{de::StrRead, Deserializer as JsonDeserializer};

use super::BinaryArg;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),
}

impl DeError for Error {
    fn custom<T>(msg: T) -> Self
    where
        T: Display,
    {
        Error::Message(msg.to_string())
    }
}

struct BinaryDeserializer<'de> {
    d: JsonDeserializer<StrRead<'de>>,
    buffers: &'de [&'de [u8]],
}

pub fn deserialize<'a, T>(arg: &BinaryArg<'a>) -> Result<T, Error>
where
    T: Deserialize<'a>,
{
    let mut deserializer = BinaryDeserializer {
        d: JsonDeserializer::from_str(arg.0.get()),
        buffers: arg.1,
    };
    T::deserialize(&mut deserializer)
}

macro_rules! deserialize_unimplemented {
    ($($f:ident)*) => {
        $(
            paste! {
                fn [<deserialize_ $f>]<V>(self, _visitor: V) -> Result<V::Value, Error>
                where
                    V: Visitor<'de>,
                {
                    unimplemented!()
                }
            }
        )*
    };
}

impl<'de, 'a> Deserializer<'de> for &'a mut BinaryDeserializer<'de> {
    type Error = Error;

    deserialize_unimplemented!(any bool byte_buf bytes char f32 f64 i16 i32 i64 i8 identifier
        ignored_any map  option seq str string u16 u32 u64
        u8 unit i128 u128);

    fn deserialize_unit_struct<V>(self, _name: &'static str, _visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        unimplemented!()
    }

    fn deserialize_newtype_struct<V>(
        self,
        _name: &'static str,
        _visitor: V,
    ) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        unimplemented!()
    }

    fn deserialize_tuple<V>(self, _len: usize, _visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        unimplemented!()
    }

    fn deserialize_tuple_struct<V>(
        self,
        _name: &'static str,
        _len: usize,
        _visitor: V,
    ) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        unimplemented!()
    }

    fn deserialize_struct<V>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        _visitor: V,
    ) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        unimplemented!()
    }

    fn deserialize_enum<V>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        _visitor: V,
    ) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::borrow::Cow;

    use serde_json::value::RawValue;

    #[derive(Deserialize)]
    struct BinaryBorrowed<'a> {
        array: &'a [u8],
    }

    #[test]
    fn test_simple() {
        let attachment = [222, 173, 190, 239];
        let attachments = [&attachment[..]];
        let json = Cow::Owned(
            RawValue::from_string("{\"array\": {\"_placeholder\":true,\"num\":0}}".to_string())
                .unwrap(),
        );
        let arg = BinaryArg(&json, &attachments[..]);
        let _: BinaryBorrowed = deserialize(&arg).unwrap();
    }
}
