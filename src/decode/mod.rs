use std::io::Read;
use futures::Future;
use trackable::error::TrackableError;

use {Error, ErrorKind};
use traits::Field;
use wire::WireType;

pub mod futures;

mod fields;
mod types;
mod wires;

pub trait Decode<R: Read> {
    type Value: Default;
    type Future: Future<Item = (R, Self::Value), Error = Error<R>>;
    fn decode(reader: R) -> Self::Future;
    fn sync_decode(reader: R) -> Result<Self::Value, TrackableError<ErrorKind>> {
        Self::decode(reader).wait().map(|(_, v)| v).map_err(
            |e| e.error,
        )
    }
}

pub trait DecodeField<R: Read>: Field {
    type Value: Default;
    type Future: Future<Item = (R, Self::Value), Error = Error<R>>;
    fn is_target(tag: u32) -> bool;
    fn decode_field(
        reader: R,
        tag: u32,
        wire_type: WireType,
        acc: Self::Value,
    ) -> Result<Self::Future, Error<R>>;
}

#[cfg(test)]
mod test {
    use {Message, Decode};
    use fields::Field;
    use tags::{Tag1, Tag2};
    use types::Int32;

    #[test]
    fn it_works() {
        type M = Message<(Field<Tag1, Int32>, Field<Tag2, Int32>)>;
        let bytes = [0x08, 0x96, 0x01, 0x10, 0x96, 0x01];
        let v = track_try_unwrap!(M::sync_decode(&bytes[..]));
        assert_eq!(v, (150, 150));
    }
}
