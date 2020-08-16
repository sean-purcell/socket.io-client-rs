use paste::paste;
use serde::{ser, Serialize};
use tungstenite::Message as WsMessage;

struct Serializer<'a, S>
where
    S: ser::Serializer,
{
    s: S,
    buffers: &'a mut Vec<WsMessage>,
}

macro_rules! serialize_forward {
    ($($fn:ident $(<$param:ident>)? ( $( $arg:ident : $ty:ty),* ) , )*) => {
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

impl<'a, S> ser::Serializer for Serializer<'a, S>
where
    S: ser::Serializer,
{
    type Ok = S::Ok;
    type Error = S::Error;

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
            v: &T
        ),
    }

    fn is_human_readable(&self) -> bool {
        self.s.is_human_readable()
    }
}
