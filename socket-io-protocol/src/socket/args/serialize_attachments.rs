use std::{cell::RefCell, io::Write};

use paste::paste;
use serde::{
    ser::{
        self, Impossible, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant,
        SerializeTuple, SerializeTupleStruct, SerializeTupleVariant, Serializer,
    },
    Serialize,
};
use serde_json::{Error as JsonError, Serializer as JsonSerializer};
use tungstenite::Message as WsMessage;

use crate::engine;

pub fn serialize<T: ?Sized + Serialize, S: Serializer>(
    arg: &T,
    serializer: S,
    buffers: &mut Vec<WsMessage>,
) -> Result<S::Ok, S::Error> {
    let refcell = RefCell::new(buffers);
    let wrapped = Wrapper {
        s: serializer,
        buffers: &refcell,
    };
    arg.serialize(wrapped)
}

pub fn serialize_json<T: ?Sized + Serialize, W: Write>(
    arg: &T,
    write: W,
    buffers: &mut Vec<WsMessage>,
) -> Result<(), JsonError> {
    let mut serializer = JsonSerializer::new(write);
    serialize(arg, &mut serializer, buffers)
}

struct Wrapper<'a, S> {
    s: S,
    buffers: &'a RefCell<&'a mut Vec<WsMessage>>,
}

struct SeqWrapper<'a, S: Serializer> {
    buffers: &'a RefCell<&'a mut Vec<WsMessage>>,
    state: BytesState<S>,
}

enum BytesState<S: Serializer> {
    Bytes {
        s: Option<S>, // This is an option to allow us to take it while calling serialize
        data: Vec<u8>,
        len: Option<usize>,
    },
    Poisoned {
        s: S::SerializeSeq,
    },
}

struct SeqElementSerializer {}
#[derive(Debug, thiserror::Error)]
#[error("<MARKER OBJECT, IF THIS STRING IS PRINTED SOMEWHERE THIS IS A BUG>")]
struct SeqElementSerializerError {}

#[derive(Serialize)]
struct Placeholder {
    #[serde(rename = "_placeholder")]
    placeholder: bool,
    num: u64,
}

impl Placeholder {
    fn new(idx: usize) -> Self {
        Placeholder {
            placeholder: true,
            num: idx as u64,
        }
    }
}

macro_rules! serialize_forward {
    ($($fn:ident ( $($arg:ident : $ty:ty),* ) , )*) => {
        $(
            paste!{
                fn [<serialize_ $fn>](
                    self,
                    $( $arg: $ty ),*
                ) -> Result<Self::Ok, Self::Error>
                {
                    self.s.[<serialize_ $fn>]($( $arg , )*)
                }
            }
        )*
    };
}

macro_rules! serialize_forward_wrapped {
    ($($fn:ident ( $($arg:ident : $ty:ty),* ) , )*) => {
        $(
            paste!{
                fn [<serialize_ $fn>]<T: Serialize + ?Sized>(
                    self,
                    $( $arg: $ty , )*
                    s: &T
                ) -> Result<Self::Ok, Self::Error>
                {
                    self.s.[<serialize_ $fn>](
                        $( $arg , )*
                        &Wrapper {
                            s,
                            buffers: self.buffers,
                        }
                    )
                }
            }
        )*
    };
}

macro_rules! serialize_forward_compound {
    ($($fn:ident ( $( $arg:ident : $ty:ty ),* ) -> $wrapped:ident , )*) => {
        $(
            paste!{
                type [<Serialize $wrapped>] = Wrapper<'a, S::[<Serialize $wrapped>]>;

                fn [<serialize_ $fn>](
                    self,
                    $( $arg: $ty ),*
                ) -> Result<Self::[<Serialize $wrapped>], Self::Error> {
                    let s = self.s.[<serialize_ $fn>]($($arg),*)?;
                    Ok(Wrapper {
                        s,
                        buffers: self.buffers,
                    })
                }
            }
        )*
    }
}

impl<'a, S> Wrapper<'a, S> {
    fn transform<P>(&self, p: P) -> Wrapper<'a, P> {
        Wrapper {
            s: p,
            buffers: self.buffers,
        }
    }
}

impl<'a, S> Serializer for Wrapper<'a, S>
where
    S: Serializer,
{
    type Ok = S::Ok;
    type Error = S::Error;

    serialize_forward_compound! {
        tuple(len: usize) -> Tuple,
        tuple_struct(name: &'static str, len: usize) -> TupleStruct,
        tuple_variant(
            name: &'static str,
            variant_index: u32,
            variant: &'static str,
            len: usize
        ) -> TupleVariant,
        map(len: Option<usize>) -> Map,
        struct(name: &'static str, len: usize) -> Struct,
        struct_variant(
            name: &'static str,
            variant_index: u32,
            variant: &'static str,
            len: usize
        ) -> StructVariant,
    }

    serialize_forward! {
        bool(v: bool),
        i8(v: i8),
        i16(v: i16),
        i32(v: i32),
        i64(v: i64),
        i128(v: i128),
        u8(v: u8),
        u16(v: u16),
        u32(v: u32),
        u64(v: u64),
        u128(v: u128),
        f32(v: f32),
        f64(v: f64),
        char(v: char),
        str(v: &str),
        none(),
        unit(),
        unit_struct(name: &'static str),
        unit_variant(
            name: &'static str,
            variant_index: u32,
            variant: &'static str
        ),
    }

    serialize_forward_wrapped! {
        some(),
        newtype_struct(name: &'static str),
        newtype_variant(
            name: &'static str,
            variant_index: u32,
            variant: &'static str
        ),
    }

    fn serialize_bytes(self, bytes: &[u8]) -> Result<Self::Ok, Self::Error> {
        let mut buffers = self.buffers.borrow_mut();
        let idx = buffers.len();
        buffers.push(engine::encode_binary(bytes));
        Placeholder::new(idx).serialize(self.s)
    }

    type SerializeSeq = SeqWrapper<'a, S>;

    fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Ok(SeqWrapper {
            buffers: self.buffers,
            state: BytesState::Bytes {
                s: Some(self.s),
                data: Vec::new(),
                len,
            },
        })
    }

    fn is_human_readable(&self) -> bool {
        self.s.is_human_readable()
    }
}

impl<'a, S> Serialize for Wrapper<'a, S>
where
    S: Serialize,
{
    fn serialize<T: Serializer>(&self, serializer: T) -> Result<T::Ok, T::Error> {
        self.s.serialize(self.transform(serializer))
    }
}

impl<'a, S> SerializeTuple for Wrapper<'a, S>
where
    S: SerializeTuple,
{
    type Ok = S::Ok;
    type Error = S::Error;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), Self::Error> {
        self.s.serialize_element(&self.transform(v))
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        self.s.end()
    }
}

impl<'a, S> SerializeTupleStruct for Wrapper<'a, S>
where
    S: SerializeTupleStruct,
{
    type Ok = S::Ok;
    type Error = S::Error;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), Self::Error> {
        self.s.serialize_field(&self.transform(v))
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        self.s.end()
    }
}

impl<'a, S> SerializeTupleVariant for Wrapper<'a, S>
where
    S: SerializeTupleVariant,
{
    type Ok = S::Ok;
    type Error = S::Error;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), Self::Error> {
        self.s.serialize_field(&self.transform(v))
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        self.s.end()
    }
}

impl<'a, S> SerializeMap for Wrapper<'a, S>
where
    S: SerializeMap,
{
    type Ok = S::Ok;
    type Error = S::Error;

    fn serialize_key<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), Self::Error> {
        self.s.serialize_key(&self.transform(v))
    }

    fn serialize_value<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), Self::Error> {
        self.s.serialize_value(&self.transform(v))
    }

    fn serialize_entry<K: ?Sized + Serialize, V: ?Sized + Serialize>(
        &mut self,
        k: &K,
        v: &V,
    ) -> Result<(), Self::Error> {
        self.s
            .serialize_entry(&self.transform(k), &self.transform(v))
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        self.s.end()
    }
}

impl<'a, S> SerializeStruct for Wrapper<'a, S>
where
    S: SerializeStruct,
{
    type Ok = S::Ok;
    type Error = S::Error;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        v: &T,
    ) -> Result<(), Self::Error> {
        self.s.serialize_field(key, &self.transform(v))
    }

    fn skip_field(&mut self, key: &'static str) -> Result<(), Self::Error> {
        self.s.skip_field(key)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        self.s.end()
    }
}

impl<'a, S> SerializeStructVariant for Wrapper<'a, S>
where
    S: SerializeStructVariant,
{
    type Ok = S::Ok;
    type Error = S::Error;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        v: &T,
    ) -> Result<(), Self::Error> {
        self.s.serialize_field(key, &self.transform(v))
    }

    fn skip_field(&mut self, key: &'static str) -> Result<(), Self::Error> {
        self.s.skip_field(key)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        self.s.end()
    }
}

// NOTE: This would be much simpler if serialize_element could be specialized for u8

impl<'a, S> SerializeSeq for SeqWrapper<'a, S>
where
    S: Serializer,
{
    type Ok = S::Ok;
    type Error = S::Error;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), Self::Error> {
        match &mut self.state {
            BytesState::Bytes { s, data, len } => match v.serialize(SeqElementSerializer {}) {
                Ok(b) => {
                    if data.is_empty() {
                        data.push(engine::BINARY_HEADER);
                    }
                    data.push(b);
                    Ok(())
                }
                Err(_) => {
                    // The data isn't all u8's
                    let mut seq = s.take().unwrap().serialize_seq(*len)?;
                    for b in data.iter() {
                        seq.serialize_element(b)?;
                    }
                    seq.serialize_element(&Wrapper {
                        s: v,
                        buffers: self.buffers,
                    })?;
                    self.state = BytesState::Poisoned { s: seq };
                    Ok(())
                }
            },
            BytesState::Poisoned { s } => s.serialize_element(&Wrapper {
                s: v,
                buffers: self.buffers,
            }),
        }
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        match self.state {
            BytesState::Bytes { s, data, len: _ } => {
                if data.is_empty() {
                    let seq = s.unwrap().serialize_seq(Some(0))?;
                    seq.end()
                } else {
                    let mut buffers = self.buffers.borrow_mut();
                    let idx = buffers.len();
                    buffers.push(engine::package_binary(data));
                    Placeholder::new(idx).serialize(s.unwrap())
                }
            }
            BytesState::Poisoned { s } => s.end(),
        }
    }
}

macro_rules! seq_serialize {
    ($($fn:ident ( $($arg:ident : $ty:ty),* ) , )*) => {
        $(
            paste!{
                #[allow(unused_variables)]
                fn [<serialize_ $fn>](
                    self,
                    $( $arg: $ty ),*
                ) -> Result<Self::Ok, Self::Error>
                {
                    Err(SeqElementSerializerError {})
                }
            }
        )*
    };
}

macro_rules! seq_serialize_type_param {
    ($($fn:ident ( $($arg:ident : $ty:ty),* ) , )*) => {
        $(
            paste!{
                #[allow(unused_variables)]
                fn [<serialize_ $fn>]<T: ?Sized + Serialize>(
                    self,
                    $( $arg: $ty , )*
                    _s: &T,
                ) -> Result<Self::Ok, Self::Error>
                {
                    Err(SeqElementSerializerError {})
                }
            }
        )*
    };
}

macro_rules! seq_serialize_compound {
    ($($fn:ident ( $( $arg:ident : $ty:ty ),* ) -> $wrapped:ident , )*) => {
        $(
            paste!{
                type [<Serialize $wrapped>] = Impossible<Self::Ok, Self::Error>;

                #[allow(unused_variables)]
                fn [<serialize_ $fn>](
                    self,
                    $( $arg: $ty ),*
                ) -> Result<Self::[<Serialize $wrapped>], Self::Error> {
                    Err(SeqElementSerializerError {})
                }
            }
        )*
    }
}

impl Serializer for SeqElementSerializer {
    type Ok = u8;
    type Error = SeqElementSerializerError;

    seq_serialize_compound! {
        seq(len: Option<usize>) -> Seq,
        tuple(len: usize) -> Tuple,
        tuple_struct(name: &'static str, len: usize) -> TupleStruct,
        tuple_variant(
            name: &'static str,
            variant_index: u32,
            variant: &'static str,
            len: usize
        ) -> TupleVariant,
        map(len: Option<usize>) -> Map,
        struct(name: &'static str, len: usize) -> Struct,
        struct_variant(
            name: &'static str,
            variant_index: u32,
            variant: &'static str,
            len: usize
        ) -> StructVariant,
    }

    seq_serialize! {
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
        bytes(v: &[u8]),
        none(),
        unit(),
        unit_struct(name: &'static str),
        unit_variant(
            name: &'static str,
            variant_index: u32,
            variant: &'static str
        ),
    }

    seq_serialize_type_param! {
        some(),
        newtype_struct(name: &'static str),
        newtype_variant(
            name: &'static str,
            variant_index: u32,
            variant: &'static str
        ),
    }

    fn serialize_u8(self, v: u8) -> Result<Self::Ok, Self::Error> {
        Ok(v)
    }
}

impl ser::Error for SeqElementSerializerError {
    fn custom<T>(_msg: T) -> Self {
        SeqElementSerializerError {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Cursor;

    const DEADBEEF: &'static [u8] = &[0xde, 0xad, 0xbe, 0xef];

    fn serialize_json_string<T: ?Sized + Serialize>(
        arg: &T,
    ) -> Result<(String, Vec<WsMessage>), JsonError> {
        let mut buffers = Vec::new();
        let mut string = String::new();
        serialize_json(
            arg,
            Cursor::new(unsafe { string.as_mut_vec() }),
            &mut buffers,
        )?;
        Ok((string, buffers))
    }

    #[test]
    fn test_simple() {
        assert_eq!(
            serialize_json_string("test").unwrap(),
            (r#""test""#.to_string(), vec![])
        );
    }

    #[derive(Serialize)]
    struct Basic {
        a: u8,
        b: &'static str,
    }

    #[test]
    fn test_simple_struct() {
        assert_eq!(
            serialize_json_string(&Basic { a: 5, b: "test" }).unwrap(),
            (r#"{"a":5,"b":"test"}"#.to_string(), vec![])
        );
    }

    #[test]
    fn test_bytes() {
        assert_eq!(
            serialize_json_string(DEADBEEF).unwrap(),
            (
                r#"{"_placeholder":true,"num":0}"#.to_string(),
                vec![engine::encode_binary(DEADBEEF)]
            )
        );
    }

    #[derive(Serialize)]
    struct Nested {
        v: Vec<Inner>,
    }

    #[derive(Serialize)]
    struct Inner {
        d: Vec<u8>,
    }

    #[test]
    fn test_nested_bytes() {
        let n = Nested {
            v: vec![
                Inner {
                    d: DEADBEEF.to_vec(),
                },
                Inner { d: vec![] },
                Inner {
                    d: vec![0, 1, 2, 3, 4, 5, 6],
                },
            ],
        };

        assert_eq!(
            serialize_json_string(&n).unwrap(),
            (
                r#"{"v":[{"d":{"_placeholder":true,"num":0}},{"d":[]},{"d":{"_placeholder":true,"num":1}}]}"#.to_string(),
                vec![
                    engine::encode_binary(DEADBEEF),
                    engine::encode_binary(&[0, 1, 2, 3, 4, 5, 6])
                ],
            )
        );
    }
}
