use std::fmt;

use paste::paste;
use serde::{
    de::{
        value::BorrowedStrDeserializer, DeserializeSeed, EnumAccess, Error as DeError, MapAccess,
        SeqAccess, VariantAccess, Visitor,
    },
    Deserialize, Deserializer,
};
use serde_json::{value::RawValue, Deserializer as JsonDeserializer, Error as JsonError};

use super::BinaryArg;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Error(String);

impl DeError for Error {
    fn custom<T>(msg: T) -> Self
    where
        T: fmt::Display,
    {
        Error(msg.to_string())
    }
}

pub fn deserialize<'a, T>(arg: &BinaryArg<'a>) -> Result<T, JsonError>
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

struct BinarySeqAccess<'de, S>
where
    S: SeqAccess<'de>,
{
    seq: S,
    buffers: Buffers<'de>,
}

struct BinaryEnumAccess<'de, E>
where
    E: EnumAccess<'de>,
{
    data: E,
    buffers: Buffers<'de>,
}

struct BinaryVariantAccess<'de, V>
where
    V: VariantAccess<'de>,
{
    variant: V,
    buffers: Buffers<'de>,
}

struct BinarySeed<'a, T>
where
    T: DeserializeSeed<'a>,
{
    seed: T,
    buffers: Buffers<'a>,
}

struct BinaryMapAccess<'a, M>
where
    M: MapAccess<'a>,
{
    map: M,
    buffers: Buffers<'a>,
    entry0: Option<Option<(&'a str, &'a RawValue)>>,
    entry1: Option<Option<(&'a str, &'a RawValue)>>,
}

macro_rules! deserialize_forward {
    ($($fn:ident ( $( $arg:ident : $ty:ty),* ) , )*) => {
        $(
            paste!{
                fn [<deserialize_ $fn>]<V>(self, $( $arg: $ty , )* visitor: V) -> Result<V::Value, D::Error>
                where
                    V: Visitor<'de>
                {
                    self.d.[<deserialize_ $fn>](
                            $( $arg , )*
                            BinaryVisitor { visitor, buffers: self.buffers }
                        )
                }
            }
        )*
    };
}

macro_rules! deserialize_forward_any {
    ($($fn:ident ( $( $arg:ident : $ty:ty),* ) , )*) => {
        $(
            paste!{
                fn [<deserialize_ $fn>]<V>(self, $( $arg: $ty , )* visitor: V) -> Result<V::Value, D::Error>
                where
                    V: Visitor<'de>
                {
                    self.d.deserialize_any(BinaryVisitor { visitor, buffers: self.buffers })
                }
            }
        )*
    };
}

impl<'de, D> Deserializer<'de> for BinaryDeserializer<'de, D>
where
    D: Deserializer<'de>,
{
    type Error = D::Error;

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

    deserialize_forward_any! {
        str(),
        string(),
        bytes(),
        byte_buf(),
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

    fn visit_some<D>(self, d: D) -> Result<V::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wrapped = BinaryDeserializer {
            d,
            buffers: self.buffers,
        };
        self.visitor.visit_some(wrapped)
    }

    fn visit_newtype_struct<D>(self, d: D) -> Result<V::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wrapped = BinaryDeserializer {
            d,
            buffers: self.buffers,
        };
        self.visitor.visit_newtype_struct(wrapped)
    }

    fn visit_seq<A>(self, seq: A) -> Result<V::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let wrapped = BinarySeqAccess {
            seq,
            buffers: self.buffers,
        };
        self.visitor.visit_seq(wrapped)
    }

    fn visit_enum<A>(self, data: A) -> Result<V::Value, A::Error>
    where
        A: EnumAccess<'de>,
    {
        let wrapped = BinaryEnumAccess {
            data,
            buffers: self.buffers,
        };
        self.visitor.visit_enum(wrapped)
    }

    fn visit_map<A>(self, mut map: A) -> Result<V::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let entry0: Option<(&'de str, &'de RawValue)> = map.next_entry()?;
        let entry1: Option<(&'de str, &'de RawValue)> = if entry0.is_some() {
            map.next_entry()?
        } else {
            None
        };
        match (entry0, entry1) {
            (Some(("_placeholder", _)), Some(("num", num)))
            | (Some(("num", num)), Some(("_placeholder", _))) => {
                let mut deserializer = JsonDeserializer::from_str(num.get());
                let num =
                    u64::deserialize(&mut deserializer).map_err(|err| A::Error::custom(err))?;
                let buffer = self.buffers.get(num as usize).ok_or_else(|| {
                    A::Error::custom(format!(
                        "Placeholder num out of range: {}/{}",
                        num,
                        self.buffers.len()
                    ))
                })?;
                self.visitor.visit_borrowed_bytes(buffer)
            }
            (entry0, entry1) => {
                let map = BinaryMapAccess {
                    map,
                    buffers: self.buffers,
                    entry0: Some(entry0),
                    entry1: Some(entry1),
                };
                self.visitor.visit_map(map)
            }
        }
    }
}

impl<'de, S> SeqAccess<'de> for BinarySeqAccess<'de, S>
where
    S: SeqAccess<'de>,
{
    type Error = S::Error;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>, Self::Error>
    where
        T: DeserializeSeed<'de>,
    {
        self.seq.next_element_seed(BinarySeed {
            seed,
            buffers: self.buffers,
        })
    }
}

impl<'de, E> EnumAccess<'de> for BinaryEnumAccess<'de, E>
where
    E: EnumAccess<'de>,
{
    type Error = E::Error;
    type Variant = BinaryVariantAccess<'de, E::Variant>;

    fn variant_seed<V>(self, seed: V) -> Result<(V::Value, Self::Variant), E::Error>
    where
        V: DeserializeSeed<'de>,
    {
        let (value, variant) = self.data.variant_seed(seed)?;
        Ok((
            value,
            BinaryVariantAccess {
                variant,
                buffers: self.buffers,
            },
        ))
    }
}

impl<'de, V> VariantAccess<'de> for BinaryVariantAccess<'de, V>
where
    V: VariantAccess<'de>,
{
    type Error = V::Error;

    fn unit_variant(self) -> Result<(), Self::Error> {
        self.variant.unit_variant()
    }

    fn newtype_variant_seed<T>(self, seed: T) -> Result<T::Value, V::Error>
    where
        T: DeserializeSeed<'de>,
    {
        self.variant.newtype_variant_seed(BinarySeed {
            seed,
            buffers: self.buffers,
        })
    }

    fn tuple_variant<T>(self, len: usize, visitor: T) -> Result<T::Value, V::Error>
    where
        T: Visitor<'de>,
    {
        self.variant.tuple_variant(
            len,
            BinaryVisitor {
                visitor,
                buffers: self.buffers,
            },
        )
    }

    fn struct_variant<T>(
        self,
        fields: &'static [&'static str],
        visitor: T,
    ) -> Result<T::Value, V::Error>
    where
        T: Visitor<'de>,
    {
        self.variant.struct_variant(
            fields,
            BinaryVisitor {
                visitor,
                buffers: self.buffers,
            },
        )
    }
}

impl<'de, M> MapAccess<'de> for BinaryMapAccess<'de, M>
where
    M: MapAccess<'de>,
{
    type Error = M::Error;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, M::Error>
    where
        K: DeserializeSeed<'de>,
    {
        match (self.entry0, self.entry1, &mut self.map) {
            (Some(entry), _, _) | (None, Some(entry), _) => {
                let key = entry.map(|(key, _)| key);
                if let Some(k) = key {
                    let deserializer = BorrowedStrDeserializer::new(k);
                    seed.deserialize(deserializer).map(Some)
                } else {
                    Ok(None)
                }
            }
            (None, None, map) => {
                // TODO(me@seanp.xyz): This isn't passing the buffers along because the keys should
                // just be strings, but it's possible this assumption is wrong.
                map.next_key_seed(seed)
            }
        }
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, M::Error>
    where
        V: DeserializeSeed<'de>,
    {
        if let Some(entry) = self.entry0.take() {
            let value = entry.unwrap().1;
            let mut deserializer = JsonDeserializer::from_str(value.get());
            seed.deserialize(&mut deserializer)
                .map_err(|err| M::Error::custom(err))
        } else if let Some(entry) = self.entry1.take() {
            let value = entry.unwrap().1;
            let mut deserializer = JsonDeserializer::from_str(value.get());
            seed.deserialize(&mut deserializer)
                .map_err(|err| M::Error::custom(err))
        } else {
            self.map.next_value_seed(BinarySeed {
                seed,
                buffers: self.buffers,
            })
        }
    }
}

impl<'b, T> DeserializeSeed<'b> for BinarySeed<'b, T>
where
    T: DeserializeSeed<'b>,
{
    type Value = T::Value;

    fn deserialize<D>(self, d: D) -> Result<T::Value, D::Error>
    where
        D: Deserializer<'b>,
    {
        let wrapper = BinaryDeserializer {
            d,
            buffers: self.buffers,
        };
        self.seed.deserialize(wrapper)
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

    #[test]
    fn test_passthrough() {
        let attachment = [222, 173, 190, 239];
        let attachments = [];
        let json = Cow::Owned(
            RawValue::from_string("{\"array\": [222, 173, 190, 239]}".to_string()).unwrap(),
        );
        let arg = BinaryArg(&json, &attachments[..]);
        let res: BinaryOwned = deserialize(&arg).unwrap();
        assert_eq!(res.array, attachment[..].to_vec());
    }

    #[derive(Debug, Deserialize, PartialEq)]
    enum Enum {
        A(i8),
        B(u32),
        C(Vec<u8>),
    }

    #[test]
    fn test_enum_passthrough() {
        let attachments = [];
        let json = Cow::Owned(RawValue::from_string("{\"B\": 23}".to_string()).unwrap());
        let arg = BinaryArg(&json, &attachments[..]);
        let res: Enum = deserialize(&arg).unwrap();
        assert_eq!(res, Enum::B(23));
    }
}
