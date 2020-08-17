use std::cell::RefCell;

use paste::paste;
use serde::{
    ser::{
        SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple,
        SerializeTupleStruct, SerializeTupleVariant, Serializer,
    },
    Serialize,
};
use tungstenite::Message as WsMessage;

use crate::engine;

struct Wrapper<'a, S> {
    s: S,
    buffers: &'a RefCell<Vec<WsMessage>>,
}

struct SeqWrapper<'a, S> {
    buffers: &'a RefCell<Vec<WsMessage>>,
    state: BytesState<S>,
}

enum BytesState<S: Serialize> {
    Bytes {
        s: Option<S>, // This is an option to allow us to take it while calling serialize
        data: Vec<u8>,
        len: Option<usize>,
    },
    Poisoned {
        s: S::SerializeSeq,
    },
}

struct SeqElementSerializer<'a, S> {
    s: S,
    buffers: &'a RefCell<Vec<WsMessage>>,
    data: &'a Vec<u8>,
    len: Option<usize>,
}

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
    ($($fn:ident $(<$param:ident>)? ( $($arg:ident : $ty:ty),* ) , )*) => {
        $(
            paste!{
                fn [<serialize_ $fn>]$(<$param: Serialize + ?Sized>)?(
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

macro_rules! forward_wrapper {
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
    fn new<'c, 'b: 'c, P>(&'b self, p: P) -> Wrapper<'c, P> {
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

    forward_wrapper! {
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
        some<T>(v: &T),
        unit(),
        unit_struct(name: &'static str),
        unit_variant(
            name: &'static str,
            variant_index: u32,
            variant: &'static str
        ),
        newtype_struct<T>(
            name: &'static str,
            v: &T
        ),
        newtype_variant<T>(
            name: &'static str,
            variant_index: u32,
            variant: &'static str,
            v: &T // FIXME: We need to be wrapping these
        ),
    }

    fn serialize_bytes(self, bytes: &[u8]) -> Result<Self::Ok, Self::Error> {
        let buffers = self.buffers.borrow_mut();
        let idx = buffers.len();
        buffers.push(engine::encode_binary(bytes));
        Placeholder::new(idx).serialize(self.s)
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
        self.s.serialize(self.new(serializer))
    }
}

impl<'a, S> SerializeTuple for Wrapper<'a, S>
where
    S: SerializeTuple,
{
    type Ok = S::Ok;
    type Error = S::Error;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), Self::Error> {
        self.s.serialize_element(&self.new(v))
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
        self.s.serialize_field(&self.new(v))
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
        self.s.serialize_field(&self.new(v))
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
        self.s.serialize_key(&self.new(v))
    }

    fn serialize_value<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), Self::Error> {
        self.s.serialize_value(&self.new(v))
    }

    fn serialize_entry<K: ?Sized + Serialize, V: ?Sized + Serialize>(
        &mut self,
        k: &K,
        v: &V,
    ) -> Result<(), Self::Error> {
        self.s.serialize_entry(&self.new(k), &self.new(v))
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
        self.s.serialize_field(key, &self.new(v))
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
        self.s.serialize_field(key, &self.new(v))
    }

    fn skip_field(&mut self, key: &'static str) -> Result<(), Self::Error> {
        self.s.skip_field(key)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        self.s.end()
    }
}

impl<'a, S> SerializeSeq for SeqWrapper<'a, S>
where
    S: SerializeSeq,
{
    type Ok = S::Ok;
    type Error = S::Error;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), Self::Error> {
        match self.bytes {
            BytesState::Bytes { s, data, len } => (SeqElementSerializer {
                s: s.take().unwrap(),
                buffers: self.buffers,
                data: &data,
                len,
            })
            .serialize(v),
            BytesState::Poisoned { s } => s.serialize_element(v),
        }
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        self.s.end()
    }
}

// NOTE: This would be much simpler if serialize_element could be specialized for u8

macro_rules! seq_serialize_forward {
    ($($fn:ident $(<$param:ident>)? ( $($arg:ident : $ty:ty),* ) , )*) => {
        $(
            paste!{
                fn [<serialize_ $fn>]$(<$param: Serialize + ?Sized>)?(
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

macro_rules! seq_forward_wrapper {
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

impl<'a, S> SeqElementSerializer<'a, S>
where
    S: Serialize,
{
    fn to_seq_serializer(self) -> Result<S::SerializeSeq, S::Error> {
        let mut seq = self.s.serialize_seq(self.len)?;
        for b in self.data.iter() {
            seq.serialize_element(b)?;
        }
        Ok(seq)
    }
}

impl<'a, S> Serializer for SeqElementSerializer<'a, S> {}
