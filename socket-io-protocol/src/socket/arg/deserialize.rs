use std::error::Error as StdError;
use std::fmt;

use paste::paste;
use serde::{
    de::{EnumAccess, Error as DeError, MapAccess, SeqAccess, Visitor},
    Deserialize, Deserializer,
};
use serde_json::{de::StrRead, Deserializer as JsonDeserializer, Error as JsonError};

use super::BinaryArg;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),
}

impl DeError for Error {
    fn custom<T>(msg: T) -> Self
    where
        T: fmt::Display,
    {
        Error::Message(msg.to_string())
    }
}

pub fn deserialize<'a, T>(arg: &BinaryArg<'a>) -> Result<T, Error>
where
    T: Deserialize<'a>,
{
    let mut d = JsonDeserializer::from_str(arg.0.get());
    let deserializer = BinaryDeserializer {
        d: &mut d,
        buffers: arg.1,
    };
    T::deserialize(deserializer)
}

type Buffers<'a> = &'a [&'a [u8]];

struct BinaryDeserializer<'de, D>
where
    D: Deserializer<'de>,
{
    d: D,
    buffers: Buffers<'de>,
}

struct BinaryVisitor<'de, V>
where
    V: Visitor<'de>,
{
    visitor: V,
    buffers: Buffers<'de>,
}

macro_rules! deserialize_forward {
    ($($fn:ident ( $( $arg:ident : $ty:ty),* ) , )*) => {
        $(
            paste!{
                fn [<deserialize_ $fn>]<V>(self, $( $arg: $ty , )* visitor: V) -> Result<V::Value, Error>
                where
                    V: Visitor<'de>
                {
                    Ok(self.d.[<deserialize_ $fn>](
                            $( $arg , )*
                            BinaryVisitor { visitor, buffers: self.buffers }
                        ).map_err(|e| Error::Message(e.to_string()))?)
                }
            }
        )*
    };
}

impl<'de, D> Deserializer<'de> for BinaryDeserializer<'de, D>
where
    D: Deserializer<'de>,
{
    type Error = Error;

    deserialize_forward! {
        any(),
        bool(),
        i8(),
        i16(),
        i32(),
        i64(),
        u8(),
        u16(),
        u32(),
        u64(),
        f32(),
        f64(),
        char(),
        str(),
        string(),
        bytes(),
        byte_buf(),
        option(),
        unit(),
        unit_struct(name: &'static str),
        newtype_struct(name: &'static str),
        seq(),
        tuple(len: usize),
        tuple_struct(name: &'static str, len: usize),
        map(),
        struct(name: &'static str, fields: &'static [&'static str]),
        enum(name: &'static str, variants: &'static [&'static str]),
        identifier(),
        ignored_any(),
        i128(),
        u128(),
    }

    fn is_human_readable(&self) -> bool {
        self.d.is_human_readable()
    }
}

macro_rules! visit_forward {
    ($($fn:ident ( $( $arg:ident : $ty:ty),* ) , )*) => {
        $(
            paste!{
                fn [<visit_ $fn>]<E>(self, $( $arg: $ty , )*) -> Result<V::Value, E>
                where
                    E: DeError,
                {
                    self.visitor.[<visit_ $fn>]($($arg)*)
                }
            }
        )*
    };
}

impl<'de, V> Visitor<'de> for BinaryVisitor<'de, V>
where
    V: Visitor<'de>,
{
    type Value = V::Value;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        self.visitor.expecting(formatter)
    }

    visit_forward! {
        bool(v: bool),
        i8(v: i8),
        i16(v: i16),
        i32(v: i32),
        i64(v: i64),
        i128(v: i128),
        u16(v: u16),
        u32(v: u32),
        u64(v: u64),
        u128(v: u128),
        f32(v: f32),
        f64(v: f64),
        char(v: char),
        str(v: &str),
        borrowed_str(v: &'de str),
        string(v: String),
        bytes(v: &[u8]),
        borrowed_bytes(v: &'de [u8]),
        byte_buf(v: Vec<u8>),
        none(),
        unit(),
    }

    fn visit_some<D>(self, deserializer: D) -> Result<V::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        // FIXME: Need to transform deserializer
        self.visitor.visit_some(deserializer)
    }

    fn visit_newtype_struct<D>(self, deserializer: D) -> Result<V::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        // FIXME: Need to transform deserializer
        self.visitor.visit_newtype_struct(deserializer)
    }

    fn visit_seq<A>(self, seq: A) -> Result<V::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        // FIXME: Need to transform deserializer
        self.visitor.visit_seq(seq)
    }

    fn visit_map<A>(self, map: A) -> Result<V::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        // FIXME: Need to transform deserializer
        self.visitor.visit_map(map)
    }

    fn visit_enum<A>(self, data: A) -> Result<V::Value, A::Error>
    where
        A: EnumAccess<'de>,
    {
        // FIXME: Need to transform deserializer
        self.visitor.visit_enum(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::borrow::Cow;

    use serde_json::value::RawValue;

    #[derive(Debug, Deserialize)]
    struct BinaryNoTranslate {
        array: Placeholder,
    }

    #[derive(Debug, Deserialize)]
    struct Placeholder {
        _placeholder: bool,
        num: u64,
    }

    #[test]
    fn test_translate() {
        let attachment = [222, 173, 190, 239];
        let attachments = [&attachment[..]];
        let json = Cow::Owned(
            RawValue::from_string("{\"array\": {\"_placeholder\":true,\"num\":0}}".to_string())
                .unwrap(),
        );
        let arg = BinaryArg(&json, &attachments[..]);
        let _ = deserialize::<BinaryNoTranslate>(&arg).expect_err("Deserialization succeeded");
    }

    #[derive(Deserialize)]
    struct BinaryBorrowed<'a> {
        array: &'a [u8],
    }

    #[test]
    fn test_borrowed() {
        let attachment = [222, 173, 190, 239];
        let attachments = [&attachment[..]];
        let json = Cow::Owned(
            RawValue::from_string("{\"array\": {\"_placeholder\":true,\"num\":0}}".to_string())
                .unwrap(),
        );
        let arg = BinaryArg(&json, &attachments[..]);
        let res: BinaryBorrowed = deserialize(&arg).unwrap();
        assert_eq!(res.array, &attachment[..]);
    }

    #[derive(Deserialize)]
    struct BinaryOwned {
        array: Vec<u8>,
    }

    #[test]
    fn test_owned() {
        let attachment = [222, 173, 190, 239];
        let attachments = [&attachment[..]];
        let json = Cow::Owned(
            RawValue::from_string("{\"array\": {\"_placeholder\":true,\"num\":0}}".to_string())
                .unwrap(),
        );
        let arg = BinaryArg(&json, &attachments[..]);
        let res: BinaryOwned = deserialize(&arg).unwrap();
        assert_eq!(res.array, attachment[..].to_vec());
    }
}
