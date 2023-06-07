//! gRPC codec that used a Pact interaction

use std::iter::Peekable;
use std::slice::Iter;

use anyhow::anyhow;
use bytes::{BufMut, Bytes};
use itertools::Itertools;
use pact_models::path_exp::{DocPath, PathToken};
use pact_models::v4::sync_message::SynchronousMessage;
use prost::encoding::{encode_key, encode_varint, WireType};
use prost_types::{DescriptorProto, FileDescriptorSet};
use tonic::codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use tonic::Status;
use tracing::{debug, error, instrument, trace};

use crate::message_decoder::{decode_message, ProtobufField, ProtobufFieldData};

#[derive(Debug, Clone)]
pub struct PactCodec {
  input_message: DescriptorProto,
  file_descriptor_set: FileDescriptorSet,
}

impl PactCodec {
  pub fn new(
    file: &FileDescriptorSet,
    input_message: &DescriptorProto,
    _output_message: &DescriptorProto,
    _message: &SynchronousMessage
  ) -> Self {
    PactCodec {
      file_descriptor_set: file.clone(),
      input_message: input_message.clone()
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
/// Dynamic message support based on a vector of ProtobufField fields
pub struct DynamicMessage {
  fields: Vec<ProtobufField>,
  descriptors: FileDescriptorSet
}

impl DynamicMessage {
  /// Create a new message from the slice of fields
  pub fn new(fields: &[ProtobufField], descriptors: &FileDescriptorSet) -> DynamicMessage {
    DynamicMessage {
      fields: fields.to_vec(),
      descriptors: descriptors.clone()
    }
  }

  /// Return a slice of the fields
  pub fn proto_fields(&self) -> &[ProtobufField] {
    self.fields.as_slice()
  }

  /// Encode this message to the provided buffer
  pub fn write_to<B>(&self, buffer: &mut B) -> anyhow::Result<()> where B: BufMut {
    for field in self.fields.iter().sorted_by(|a, b| Ord::cmp(&a.field_num, &b.field_num)) {
      trace!(field = field.to_string().as_str(), "Writing");
      encode_key(field.field_num, field.wire_type, buffer);
      match field.wire_type {
        WireType::Varint => match &field.data {
          ProtobufFieldData::Boolean(b) => encode_varint(*b as u64, buffer),
          ProtobufFieldData::UInteger32(n) => encode_varint(*n as u64, buffer),
          ProtobufFieldData::Integer32(n) => encode_varint(*n as u64, buffer),
          ProtobufFieldData::UInteger64(n) => encode_varint(*n, buffer),
          ProtobufFieldData::Integer64(n) => encode_varint(*n as u64, buffer),
          ProtobufFieldData::Enum(n, _) => encode_varint(*n as u64, buffer),
          ProtobufFieldData::Unknown(b) => {
            debug!("Writing unknown field {}", field.data);
            buffer.put_slice(b.as_slice());
          },
          _ => return Err(anyhow!("Expected a varint, but field is {}", field.data))
        },
        WireType::SixtyFourBit => match &field.data {
          ProtobufFieldData::UInteger64(n) => buffer.put_u64_le(*n),
          ProtobufFieldData::Integer64(n) => buffer.put_i64_le(*n),
          ProtobufFieldData::Double(n) => buffer.put_f64_le(*n),
          ProtobufFieldData::Unknown(b) => {
            debug!("Writing unknown field {}", field.data);
            buffer.put_slice(b.as_slice());
          }
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
          ProtobufFieldData::Unknown(b) => {
            debug!("Writing unknown field {}", field.data);
            buffer.put_slice(b.as_slice());
          },
          _ => return Err(anyhow!("Expected a length delimited value, but field is {}", field.data))
        }
        WireType::ThirtyTwoBit => match &field.data {
          ProtobufFieldData::UInteger32(n) => buffer.put_u32_le(*n),
          ProtobufFieldData::Integer32(n) => buffer.put_i32_le(*n),
          ProtobufFieldData::Float(n) => buffer.put_f32_le(*n),
          ProtobufFieldData::Unknown(b) => {
            debug!("Writing unknown field {}", field.data);
            buffer.put_slice(b.as_slice());
          },
          _ => return Err(anyhow!("Expected a 32 bit value, but field is {}", field.data))
        }
        _ => return Err(anyhow!("Groups are not supported"))
      }
    }
    Ok(())
  }

  /// Retrieve the value using the given path
  pub fn fetch_value(&mut self, path: &DocPath) -> Option<ProtobufField> {
    let path_tokens = path.tokens().clone();
    let mut iter = path_tokens.iter().peekable();
    if let Some(PathToken::Root) = iter.peek() {
      iter.next();
      let mut found = None;
      self.match_path(&mut iter, |v| {
        found.replace(v.clone());
      });
      found
    } else {
      None
    }
  }

  /// Update the value using the given path
  #[instrument]
  pub fn set_value(&mut self, path: &DocPath, value: ProtobufFieldData) -> anyhow::Result<()> {
    let path_tokens = path.tokens().clone();
    let mut iter = path_tokens.iter().peekable();
    if let Some(PathToken::Root) = iter.peek() {
      iter.next();
      let mut result = Err(anyhow!("Path '{}' did not match any field", path));
      self.match_path(&mut iter, |v| {
        v.data = value.clone();
        result = Ok(());
      });
      result
    } else {
      Err(anyhow!("Path '{}' does not start with a root marker", path))
    }
  }

  #[instrument(skip(callback))]
  fn match_path<F>(
    &mut self,
    path_tokens: &mut Peekable<Iter<PathToken>>,
    callback: F
  ) where F: FnOnce(&mut ProtobufField) {
    let descriptors = self.descriptors.clone();
    let fields = &mut self.fields;
    if let Some(next) = path_tokens.next() {
      match next {
        PathToken::Field(name) => if let Some(field) = find_field(fields, name.as_str()) {
          if path_tokens.peek().is_none() {
            callback(field);
          } else {
            match &field.data {
              ProtobufFieldData::Enum(_, _) => todo!(),
              ProtobufFieldData::Message(data, descriptor) => {
                let mut buffer = Bytes::copy_from_slice(data);
                match decode_message(&mut buffer, descriptor, &descriptors) {
                  Ok(fields) => {
                    let mut message = DynamicMessage::new(fields.as_slice(), &descriptors);
                    message.match_path(path_tokens, callback);
                  }
                  Err(err) => error!("Failed to decode child message: {}", err)
                }
              },
              _ => ()
            }
          }
        }
        PathToken::Index(_) => todo!(),
        PathToken::Star => todo!(),
        PathToken::StarIndex => todo!(),
        _ => ()
      }
    }
  }
}

fn find_field<'a>(fields: &'a mut [ProtobufField], field_name: &str) -> Option<&'a mut ProtobufField> {
  fields.iter_mut()
    .find(|field| field.field_name == field_name)
}

#[derive(Debug, Clone)]
pub struct DynamicMessageEncoder {}

impl DynamicMessageEncoder {
  fn new(_codec: &PactCodec) -> Self {
    DynamicMessageEncoder {}
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
pub struct DynamicMessageDecoder {
  descriptor: DescriptorProto,
  file_descriptor_set: FileDescriptorSet
}

impl DynamicMessageDecoder {
  pub fn new(codec: &PactCodec) -> Self {
    DynamicMessageDecoder {
      descriptor: codec.input_message.clone(),
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
      Ok(fields) => Ok(Some(DynamicMessage::new(&fields, &self.file_descriptor_set))),
      Err(err) => {
        error!("Failed to decode the message - {err}");
        Err(Status::invalid_argument(format!("Failed to decode the message - {err}")))
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use bytes::BytesMut;
  use expectest::prelude::*;
  use pact_models::path_exp::DocPath;
  use prost::encoding::WireType;
  use prost_types::{DescriptorProto, FieldDescriptorProto, FileDescriptorSet};

  use crate::dynamic_message::DynamicMessage;
  use crate::message_decoder::{ProtobufField, ProtobufFieldData};

  #[test]
  fn dynamic_message_fetch_value_with_no_fields() {
    let fields = vec![];
    let descriptors = FileDescriptorSet {
      file: vec![]
    };
    let mut message = DynamicMessage::new(fields.as_slice(), &descriptors);
    let path = DocPath::new("$.one.two.three").unwrap();
    expect!(message.fetch_value(&path)).to(be_none());
  }

  #[test]
  fn dynamic_message_fetch_value_with_no_root() {
    let field = ProtobufField {
      field_num: 1,
      field_name: "one".to_string(),
      wire_type: WireType::Varint,
      data: ProtobufFieldData::Integer64(100)
    };
    let descriptors = FileDescriptorSet {
      file: vec![]
    };
    let fields = vec![ field.clone() ];
    let mut message = DynamicMessage::new(fields.as_slice(), &descriptors);
    let path = DocPath::new("one").unwrap();
    expect!(message.fetch_value(&path)).to(be_some().value(field));
  }

  #[test]
  fn dynamic_message_fetch_value_with_matching_field() {
    let field = ProtobufField {
      field_num: 1,
      field_name: "one".to_string(),
      wire_type: WireType::Varint,
      data: ProtobufFieldData::Integer64(100)
    };
    let descriptors = FileDescriptorSet {
      file: vec![]
    };
    let fields = vec![ field.clone() ];
    let mut message = DynamicMessage::new(fields.as_slice(), &descriptors);
    let path = DocPath::new("$.one").unwrap();
    expect!(message.fetch_value(&path)).to(be_some().value(field));
  }

  #[test]
  fn dynamic_message_fetch_value_with_matching_child_field() {
    let child_descriptor = DescriptorProto {
      name: Some("child".to_string()),
      field: vec![
        FieldDescriptorProto {
          name: Some("two".to_string()),
          number: Some(1),
          label: None,
          r#type: Some(3),
          type_name: None,
          extendee: None,
          default_value: None,
          oneof_index: None,
          json_name: None,
          options: None,
          proto3_optional: None
        },
        FieldDescriptorProto {
          name: Some("three".to_string()),
          number: Some(2),
          label: None,
          r#type: Some(3),
          type_name: None,
          extendee: None,
          default_value: None,
          oneof_index: None,
          json_name: None,
          options: None,
          proto3_optional: None
        }
      ],
      extension: vec![],
      nested_type: vec![],
      enum_type: vec![],
      extension_range: vec![],
      oneof_decl: vec![],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![],
    };
    let child_field = ProtobufField {
      field_num: 1,
      field_name: "two".to_string(),
      wire_type: WireType::Varint,
      data: ProtobufFieldData::Integer64(100)
    };
    let child_field2 = ProtobufField {
      field_num: 2,
      field_name: "three".to_string(),
      wire_type: WireType::Varint,
      data: ProtobufFieldData::Integer64(200)
    };
    let descriptors = FileDescriptorSet {
      file: vec![]
    };
    let child_message = DynamicMessage::new(&[child_field.clone(), child_field2], &descriptors);
    let mut buffer = BytesMut::new();
    child_message.write_to(&mut buffer).unwrap();
    let field = ProtobufField {
      field_num: 1,
      field_name: "one".to_string(),
      wire_type: WireType::LengthDelimited,
      data: ProtobufFieldData::Message(buffer.to_vec(), child_descriptor)
    };
    let fields = vec![ field.clone() ];
    let mut message = DynamicMessage::new(fields.as_slice(), &descriptors);
    let path = DocPath::new("$.one.two").unwrap();
    expect!(message.fetch_value(&path)).to(be_some().value(child_field));
  }
}
