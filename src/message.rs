//! Encoders, decoders and traits for messages.
use bytecodec::combinator::{Map, MapErr, MapFrom, PreEncode, TryMap, TryMapFrom};
use bytecodec::{ByteCount, Decode, Encode, Eos, Error, ErrorKind, ExactBytesEncode, Result};
use std;

use field::{FieldDecode, FieldEncode, UnknownFieldDecoder};
use value::{OptionalValueDecode, ValueDecode, ValueEncode};
use wire::{LengthDelimitedDecoder, LengthDelimitedEncoder, TagDecoder, WireType};

/// This trait allows for decoding messages.
pub trait MessageDecode: Decode {
    /// Merges duplicate messages.
    fn merge_messages(old: &mut Self::Item, new: Self::Item);
}
impl<M, T, F> MessageDecode for Map<M, T, F>
where
    M: MessageDecode,
    F: Fn(M::Item) -> T,
{
    fn merge_messages(old: &mut Self::Item, new: Self::Item) {
        *old = new;
    }
}
impl<M, F, T, E> MessageDecode for TryMap<M, F, T, E>
where
    M: MessageDecode,
    F: Fn(M::Item) -> std::result::Result<T, E>,
    Error: From<E>,
{
    fn merge_messages(old: &mut Self::Item, new: Self::Item) {
        *old = new;
    }
}
impl<M, F, E> MessageDecode for MapErr<M, F, E>
where
    M: MessageDecode,
    F: Fn(Error) -> E,
    Error: From<E>,
{
    fn merge_messages(old: &mut Self::Item, new: Self::Item) {
        M::merge_messages(old, new);
    }
}

/// This trait allows for encoding messages.
pub trait MessageEncode: Encode {}
impl<M: MessageEncode> MessageEncode for PreEncode<M> {}
impl<M, T, F> MessageEncode for MapFrom<M, T, F>
where
    M: MessageEncode,
    F: Fn(T) -> M::Item,
{
}
impl<M, T, E, F> MessageEncode for TryMapFrom<M, T, E, F>
where
    M: MessageEncode,
    F: Fn(T) -> std::result::Result<M::Item, E>,
    Error: From<E>,
{
}
impl<M, F, E> MessageEncode for MapErr<M, F, E>
where
    M: MessageEncode,
    F: Fn(Error) -> E,
    Error: From<E>,
{
}

/// Decoder for messages.
#[derive(Debug, Default)]
pub struct MessageDecoder<F> {
    tag: TagDecoder,
    field: F,
    unknown_field: UnknownFieldDecoder,
    is_tag_decoding: bool,
}
impl<F: FieldDecode> MessageDecoder<F> {
    /// Makes a new `MessageDecoder` instance.
    pub fn new(field_decoder: F) -> Self {
        MessageDecoder {
            tag: TagDecoder::default(),
            field: field_decoder,
            unknown_field: UnknownFieldDecoder::default(),
            is_tag_decoding: false,
        }
    }
}
impl<F: FieldDecode> Decode for MessageDecoder<F> {
    type Item = F::Item;

    fn decode(&mut self, buf: &[u8], eos: Eos) -> Result<(usize, Option<Self::Item>)> {
        let mut offset = 0;
        while offset < buf.len() {
            if self.field.is_decoding() {
                let size = track!(self.field.field_decode(&buf[offset..], eos))?;
                offset += size;
                if self.field.is_decoding() {
                    return Ok((offset, None));
                }
            } else if self.unknown_field.is_decoding() {
                let size = track!(self.unknown_field.field_decode(&buf[offset..], eos))?;
                offset += size;
                if self.unknown_field.is_decoding() {
                    return Ok((offset, None));
                }
            } else {
                let (size, item) = track!(self.tag.decode(&buf[offset..], eos))?;
                offset += size;
                if size != 0 {
                    self.is_tag_decoding = true;
                }
                if let Some(tag) = item {
                    let started = track!(self.field.start_decoding(tag))?;
                    if !started {
                        track!(self.unknown_field.start_decoding(tag))?;
                    }
                    self.is_tag_decoding = false;
                }
            }
        }
        if eos.is_reached() {
            track_assert!(!self.is_tag_decoding, ErrorKind::UnexpectedEos);
            let v = track!(self.field.finish_decoding())?;
            track!(self.unknown_field.finish_decoding())?;
            Ok((offset, Some(v)))
        } else {
            Ok((offset, None))
        }
    }

    fn requiring_bytes(&self) -> ByteCount {
        self.tag
            .requiring_bytes()
            .add_for_decoding(self.field.requiring_bytes())
            .add_for_decoding(self.unknown_field.requiring_bytes())
    }
}
impl<F: FieldDecode> MessageDecode for MessageDecoder<F> {
    fn merge_messages(old: &mut Self::Item, new: Self::Item) {
        F::merge_fields(old, new)
    }
}

/// Decoder for embedded messages.
#[derive(Debug, Default)]
pub struct EmbeddedMessageDecoder<M>(LengthDelimitedDecoder<M>);
impl<M: MessageDecode> EmbeddedMessageDecoder<M> {
    /// Makes a new `EmbeddedMessageDecoder` instance.
    pub fn new(message_decoder: M) -> Self {
        EmbeddedMessageDecoder(LengthDelimitedDecoder::new(message_decoder))
    }
}
impl<M: MessageDecode> Decode for EmbeddedMessageDecoder<M> {
    type Item = M::Item;

    fn decode(&mut self, buf: &[u8], eos: Eos) -> Result<(usize, Option<Self::Item>)> {
        track!(self.0.decode(buf, eos))
    }

    fn requiring_bytes(&self) -> ByteCount {
        self.0.requiring_bytes()
    }
}
impl<M: MessageDecode> ValueDecode for EmbeddedMessageDecoder<M> {
    fn wire_type(&self) -> WireType {
        WireType::LengthDelimited
    }

    fn merge_values(old: &mut Self::Item, new: Self::Item) {
        M::merge_messages(old, new);
    }
}
impl<M: MessageDecode> OptionalValueDecode for EmbeddedMessageDecoder<M> {
    type Optional = Option<M::Item>;

    fn merge_optional_values(old: &mut Self::Optional, new: Self::Optional) {
        match (old.take(), new) {
            (None, new) => {
                *old = new;
            }
            (Some(v), None) => {
                *old = Some(v);
            }
            (Some(mut v), Some(new)) => {
                Self::merge_values(&mut v, new);
                *old = Some(v);
            }
        }
    }
}

/// Encoder for messages.
#[derive(Debug, Default)]
pub struct MessageEncoder<F> {
    field: F,
}
impl<F: FieldEncode> MessageEncoder<F> {
    /// Makes a new `MessageEncoder` instance.
    pub fn new(field_encoder: F) -> Self {
        MessageEncoder {
            field: field_encoder,
        }
    }
}
impl<F: FieldEncode> Encode for MessageEncoder<F> {
    type Item = F::Item;

    fn encode(&mut self, buf: &mut [u8], eos: Eos) -> Result<usize> {
        track!(self.field.encode(buf, eos))
    }

    fn start_encoding(&mut self, item: Self::Item) -> Result<()> {
        track!(self.field.start_encoding(item))
    }

    fn is_idle(&self) -> bool {
        self.field.is_idle()
    }

    fn requiring_bytes(&self) -> ByteCount {
        self.field.requiring_bytes()
    }
}
impl<F: FieldEncode + ExactBytesEncode> ExactBytesEncode for MessageEncoder<F> {
    fn exact_requiring_bytes(&self) -> u64 {
        self.field.exact_requiring_bytes()
    }
}
impl<F: FieldEncode> MessageEncode for MessageEncoder<F> {}

/// Encoder for embedded messages.
#[derive(Debug, Default)]
pub struct EmbeddedMessageEncoder<M> {
    message: LengthDelimitedEncoder<M>,
}
impl<M: MessageEncode + ExactBytesEncode> EmbeddedMessageEncoder<M> {
    /// Makes a new `EmbeddedMessageEncoder` instance.
    pub fn new(message_encoder: M) -> Self {
        EmbeddedMessageEncoder {
            message: LengthDelimitedEncoder::new(message_encoder),
        }
    }
}
impl<M: MessageEncode + ExactBytesEncode> Encode for EmbeddedMessageEncoder<M> {
    type Item = M::Item;

    fn encode(&mut self, buf: &mut [u8], eos: Eos) -> Result<usize> {
        track!(self.message.encode(buf, eos))
    }

    fn start_encoding(&mut self, item: Self::Item) -> Result<()> {
        track!(self.message.start_encoding(item))
    }

    fn is_idle(&self) -> bool {
        self.message.is_idle()
    }

    fn requiring_bytes(&self) -> ByteCount {
        self.message.requiring_bytes()
    }
}
impl<M: MessageEncode + ExactBytesEncode> ExactBytesEncode for EmbeddedMessageEncoder<M> {
    fn exact_requiring_bytes(&self) -> u64 {
        self.message.exact_requiring_bytes()
    }
}
impl<M: MessageEncode + ExactBytesEncode> ValueEncode for EmbeddedMessageEncoder<M> {
    fn wire_type(&self) -> WireType {
        WireType::LengthDelimited
    }
}
