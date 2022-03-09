//! gRPC codec that used a Pact interaction

use anyhow::anyhow;
use bytes::BufMut;
use itertools::Itertools;
use pact_models::v4::sync_message::SynchronousMessage;
use prost::encoding::{encode_key, encode_varint, WireType};
use prost_types::{DescriptorProto, FileDescriptorSet, ServiceDescriptorProto};
use tonic::codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use tonic::Status;
use tracing::{error, instrument, trace};

use crate::message_decoder::{decode_message, ProtobufField, ProtobufFieldData};

#[derive(Debug, Clone)]
pub(crate) struct PactCodec {
  message: SynchronousMessage,
  input_message: DescriptorProto,
  output_message: DescriptorProto,
  file_descriptor_set: FileDescriptorSet,
}

impl PactCodec {
  pub(crate) fn new(
    file: &FileDescriptorSet,
    input_message: &DescriptorProto,
    output_message: &DescriptorProto,
    message: &SynchronousMessage
  ) -> Self {
    PactCodec {
      file_descriptor_set: file.clone(),
      input_message: input_message.clone(),
      output_message: output_message.clone(),
      message: message.clone()
    }
  }
}

impl Default for PactCodec {
  fn default() -> Self {
    panic!("Default called for PactCodec, but it requires a service descriptor and Pact message")
  }
}

impl Codec for PactCodec {
  type Encode = DynamicMessage;
  type Decode = DynamicMessage;
  type Encoder = DynamicMessageEncoder;
  type Decoder = DynamicMessageDecoder;

  fn encoder(&mut self) -> Self::Encoder {
    DynamicMessageEncoder::new(self)
  }

  fn decoder(&mut self) -> Self::Decoder {
    DynamicMessageDecoder::new(self)
  }
}

#[derive(Debug, Clone)]
pub(crate) struct DynamicMessage {
  descriptor: DescriptorProto,
  fields: Vec<ProtobufField>,
}

impl DynamicMessage {
  pub(crate) fn new(descriptor: &DescriptorProto, fields: &Vec<ProtobufField>) -> DynamicMessage {
    DynamicMessage {
      descriptor: descriptor.clone(),
      fields: fields.clone()
    }
  }

  pub(crate) fn proto_fields(&self) -> &[ProtobufField] {
    self.fields.as_slice()
  }

  #[instrument]
  pub(crate) fn write_to(&self, buffer: &mut EncodeBuf) -> anyhow::Result<()> {
    for field in self.fields.iter().sorted_by(|a, b| Ord::cmp(&a.field_num, &b.field_num)) {
      trace!(field = field.to_string().as_str(), "Writing");
      encode_key(field.field_num, field.wire_type, buffer);
      match field.wire_type {
        WireType::Varint => match field.data {
          ProtobufFieldData::Boolean(b) => encode_varint(b as u64, buffer),
          ProtobufFieldData::UInteger32(n) => encode_varint(n as u64, buffer),
          ProtobufFieldData::Integer32(n) => encode_varint(n as u64, buffer),
          ProtobufFieldData::UInteger64(n) => encode_varint(n, buffer),
          ProtobufFieldData::Integer64(n) => encode_varint(n as u64, buffer),
          ProtobufFieldData::Enum(n, _) => encode_varint(n as u64, buffer),
          _ => return Err(anyhow!("Expected a varint, but field is {}", field.data))
        },
        WireType::SixtyFourBit => match field.data {
          ProtobufFieldData::UInteger64(n) => buffer.put_u64_le(n),
          ProtobufFieldData::Integer64(n) => buffer.put_i64_le(n),
          ProtobufFieldData::Double(n) => buffer.put_f64_le(n),
          _ => return Err(anyhow!("Expected a 64 bit value, but field is {}", field.data))
        }
        WireType::LengthDelimited => match &field.data {
          ProtobufFieldData::String(s) => {
            encode_varint(s.len() as u64, buffer);
            buffer.put_slice(s.as_bytes());
          }
          ProtobufFieldData::Bytes(b) => {
            encode_varint(b.len() as u64, buffer);
            buffer.put_slice(b.as_slice());
          }
          ProtobufFieldData::Message(m, _) => {
            encode_varint(m.len() as u64, buffer);
            buffer.put_slice(m.as_slice());
          }
          _ => return Err(anyhow!("Expected a length delimited value, but field is {}", field.data))
        }
        WireType::ThirtyTwoBit => match field.data {
          ProtobufFieldData::UInteger32(n) => buffer.put_u32_le(n),
          ProtobufFieldData::Integer32(n) => buffer.put_i32_le(n),
          ProtobufFieldData::Float(n) => buffer.put_f32_le(n),
          _ => return Err(anyhow!("Expected a 32 bit value, but field is {}", field.data))
        }
        _ => return Err(anyhow!("Groups are not supported"))
      }
    }
    Ok(())
  }
}

#[derive(Debug, Clone)]
pub(crate) struct DynamicMessageEncoder {
  descriptor: DescriptorProto,
  message: SynchronousMessage,
  file_descriptor_set: FileDescriptorSet
}

impl DynamicMessageEncoder {
  fn new(codec: &PactCodec) -> Self {
    DynamicMessageEncoder {
      descriptor: codec.output_message.clone(),
      message: codec.message.clone(),
      file_descriptor_set: codec.file_descriptor_set.clone()
    }
  }
}

impl Encoder for DynamicMessageEncoder {
  type Item = DynamicMessage;
  type Error = Status;

  #[instrument]
  fn encode(&mut self, item: Self::Item, dst: &mut EncodeBuf<'_>) -> Result<(), Self::Error> {
    item.write_to(dst).map_err(|err| {
      error!("Failed to encode the message - {err}");
      Status::invalid_argument(format!("Failed to encode the message - {err}"))
    })
  }
}

#[derive(Debug, Clone)]
pub(crate) struct DynamicMessageDecoder {
  descriptor: DescriptorProto,
  message: SynchronousMessage,
  file_descriptor_set: FileDescriptorSet,
}

impl DynamicMessageDecoder {
  fn new(codec: &PactCodec) -> Self {
    DynamicMessageDecoder {
      descriptor: codec.input_message.clone(),
      message: codec.message.clone(),
      file_descriptor_set: codec.file_descriptor_set.clone()
    }
  }
}

impl Decoder for DynamicMessageDecoder {
  type Item = DynamicMessage;
  type Error = Status;

  #[instrument]
  fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Self::Error> {
    match decode_message(src, &self.descriptor, &self.file_descriptor_set) {
      Ok(fields) => Ok(Some(DynamicMessage::new(&self.descriptor, &fields))),
      Err(err) => {
        error!("Failed to decode the message - {err}");
        Err(Status::invalid_argument(format!("Failed to decode the message - {err}")))
      }
    }
  }
}
