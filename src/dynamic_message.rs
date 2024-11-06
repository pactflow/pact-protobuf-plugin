//! gRPC codec that used a Pact interaction

use std::collections::HashMap;
use std::iter::Peekable;
use std::slice::Iter;

use anyhow::{anyhow, bail};
use bytes::{BufMut, Bytes};
use itertools::Itertools;
use pact_matching::generators::DefaultVariantMatcher;
use pact_models::expression_parser::DataValue;
use pact_models::generators::{
  GenerateValue,
  Generator,
  GeneratorTestMode,
  VariantMatcher
};
use pact_models::path_exp::{DocPath, PathToken};
use pact_models::v4::sync_message::SynchronousMessage;
use prost::encoding::{encode_key, encode_varint, WireType};
use prost_types::{DescriptorProto, FileDescriptorSet};
use serde_json::Value;
use tonic::codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use tonic::Status;
use tracing::{debug, error, instrument, trace, warn};

use crate::message_decoder::{decode_message, ProtobufField, ProtobufFieldData};
use crate::message_decoder::generators::{data_value_to_proto_value, GeneratorError};

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

/// Dynamic message support based on a vector of ProtobufField field values
#[derive(Debug, Clone)]
pub struct DynamicMessage {
  fields: HashMap<u32, Vec<ProtobufField>>,
  descriptors: FileDescriptorSet,
  message_descriptor: DescriptorProto
}

impl DynamicMessage {
  /// Create a new message from the slice of fields
  pub fn new(
    message_descriptor: &DescriptorProto,
    field_data: &[ProtobufField],
    descriptors: &FileDescriptorSet
  ) -> DynamicMessage {
    DynamicMessage {
      fields: field_data.iter().map(|f| (f.field_num, f.clone())).into_group_map(),
      message_descriptor: message_descriptor.clone(),
      descriptors: descriptors.clone()
    }
  }

  /// Return a vector of the fields
  pub fn proto_fields(&self) -> Vec<ProtobufField> {
    self.fields.values().flatten().cloned().collect()
  }

  /// Encode this message to the provided buffer
  pub fn write_to<B>(&self, buffer: &mut B) -> anyhow::Result<()> where B: BufMut {
    for (field_num, values) in self.fields.iter()
      .sorted_by(|(a, _), (b, _)| Ord::cmp(a, b)) {
      for field in values {
        trace!(%field_num, field = field.to_string().as_str(), "Writing");
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
    }
    Ok(())
  }

  /// Retrieve the value for a message field using the given path
  #[instrument(ret, skip(self), fields(path = %path))]
  pub fn fetch_field_value(&mut self, path: &DocPath) -> Option<Vec<ProtobufField>> {
    let path_tokens = path.tokens().clone();
    let mut iter = path_tokens.iter().peekable();
    if let Some(PathToken::Root) = iter.peek() {
      iter.next();
      let mut found = None;
      let result = self.match_path(&mut iter, |v, _| {
        found.replace(v.clone());
      });
      if let Err(err) = result {
        error!("Failed to fetch field value for path '{}': {}", path, err);
      }
      found
    } else {
      None
    }
  }

  /// Update the value using the given path
  #[instrument(ret, skip(self), fields(path = %path))]
  pub fn set_field_value(&mut self, path: &DocPath, value: ProtobufFieldData) -> anyhow::Result<()> {
    let path_tokens = path.tokens().clone();
    let mut iter = path_tokens.iter().peekable();
    if let Some(PathToken::Root) = iter.peek() {
      iter.next();
      self.match_path(&mut iter, |v, segment| {
        if let Some(PathToken::Index(index)) = segment {
          if index >= v.len() {
            v.resize(index + 1, v[0].clone());
          }
          v[index].data = value.clone();
        } else {
          v[0].data = value.clone();
        }
      })
    } else {
      Err(anyhow!("Path '{}' does not start with a root path marker ('$')", path))
    }
  }

  fn match_path<F>(
    &mut self,
    path_tokens: &mut Peekable<Iter<PathToken>>,
    callback: F
  ) -> anyhow::Result<()> where F: FnOnce(&mut Vec<ProtobufField>, Option<PathToken>) {
    let descriptors = self.descriptors.clone();
    let fields = &mut self.fields;
    if let Some(next) = path_tokens.next() {
      match next {
        PathToken::Root => {},
        PathToken::Field(name) => return if let Some(field) = find_field_values(fields, name.as_str()) {
          if path_tokens.peek().is_none() {
            callback(field, None);
            Ok(())
          } else {
            // OK to unwrap here, as if the vec was empty, find_field_values would have skipped it.
            let first_entry = field.first_mut().unwrap();
            match &mut first_entry.data {
              ProtobufFieldData::Enum(_, _) => Err(anyhow!("Support for dynamically fetching enum values is not supported yet")),
              ProtobufFieldData::Message(data, descriptor) => {
                let mut buffer = Bytes::copy_from_slice(data);
                match decode_message(&mut buffer, descriptor, &descriptors) {
                  Ok(fields) => {
                    let mut message = DynamicMessage::new(descriptor, fields.as_slice(), &descriptors);
                    message.match_path(path_tokens, callback)?;
                    data.clear();
                    message.write_to(data).map_err(|err| {
                      error!("Failed to rewrite child message: {}", err);
                      anyhow!("Failed to rewrite child message: {}", err)
                    })
                  }
                  Err(err) => {
                    Err(anyhow!("Failed to decode child message: {}", err))
                  }
                }
              },
              _ => match path_tokens.next() {
                Some(PathToken::Star) | Some(PathToken::StarIndex) => {
                  if path_tokens.peek().is_none() {
                    callback(field, None);
                    Ok(())
                  } else {
                    Err(anyhow!("Path does not match any field in the message (additional path \
                    segments can only be applied to a child message, but field type is '{}')", first_entry.data.type_name()))
                  }
                }
                Some(PathToken::Index(index)) => if first_entry.repeated_field() && path_tokens.peek().is_none() {
                  callback(field, Some(PathToken::Index(*index)));
                  Ok(())
                } else {
                  Err(anyhow!("Path segment '{}' can only be applied to repeated fields", index))
                }
                Some(segment) => Err(anyhow!("Path segment '{}' can not be applied any field in the message", segment)),
                None => Err(anyhow!("Path name '{}' does not match any field in the message", name))
              }
            }
          }
        } else {
          Err(anyhow!("Path name '{}' does not match any field in the message", name))
        },
        PathToken::Index(_) => return Err(anyhow!("Support for index paths is not supported yet")),
        PathToken::Star => return Err(anyhow!("Support for '*' in paths is not supported yet")),
        PathToken::StarIndex => return Err(anyhow!("Support for '[*]' in paths is not supported yet")),
      }
    } else {
      return Err(anyhow!("Path does not match any field in the message (end of path tokens reached)"))
    }

    Ok(())
  }

  /// Mutates the message by applying the generators to any matching message fields
  #[instrument(ret, skip(self, generators))]
  pub fn apply_generators(
    &mut self,
    generators: Option<&HashMap<DocPath, Generator>>,
    mode: &GeneratorTestMode,
    context: &HashMap<&str, Value>
  ) -> anyhow::Result<()> {
    if let Some(generators) = generators {
      let vm_boxed = DefaultVariantMatcher.boxed();

      for (path, generator) in generators {
        let value = self.fetch_field_value(&path);
        if let Some(value) = value {
          if generator.corresponds_to_mode(mode) {
            // OK to unwrap here, for if the vec was empty, fetch_field_value would have returned None.
            let first_entry = value.first().unwrap();
            match generator.generate_value(&first_entry.data, &context, &vm_boxed) {
              Ok(generated_value) => {
                self.set_field_value(&path, generated_value)?;
              }
              Err(err) => {
                warn!("Failed to apply generator '{}' for field {}: {}", path, first_entry, err);
                if let Some(GeneratorError::ProviderStateValueIsCollection(val)) = err.downcast_ref::<GeneratorError>() {
                  if first_entry.repeated_field() && val.wrapped.is_array() {
                    let array = as_array(val)?;
                    trace!("Applying a array value ({} items) to repeated field '{}'", array.len(), first_entry.field_name);
                    for (index, dv) in array.iter().enumerate() {
                      let index_path = path_join_index(path, index);
                      let pv = data_value_to_proto_value(&first_entry.data, dv)?;
                      self.set_field_value(&index_path, pv)?;
                    }
                  } else {
                    bail!(err);
                  }
                } else {
                  bail!(err);
                }
              }
            }
          }
        } else {
          warn!("No matching field found for generator '{}'", path);
        }
      }
    }

    Ok(())
  }
}

// TODO: Replace this with DocPath.join_index when pact_models 1.2.5 is released
fn path_join_index(path: &DocPath, index: usize) -> DocPath {
  let mut new_path = path.clone();
  match path.tokens().last() {
    Some(PathToken::Root) => { new_path.push_index(index); }
    Some(PathToken::Field(_)) => { new_path.push_index(index); }
    Some(PathToken::Index(_)) => { new_path.push_index(index); }
    Some(PathToken::Star) | Some(PathToken::StarIndex) => {
      let tokens = new_path.tokens().clone();
      new_path = DocPath::empty();
      for token in tokens.iter().dropping_back(1) {
        new_path.push(token.clone());
      }
      new_path.push_index(index);
    }
    None => { new_path.push_index(index); }
  }
  new_path
}

fn as_array(data: &DataValue) -> anyhow::Result<Vec<DataValue>> {
  if let Value::Array(values) = &data.wrapped {
    Ok(values.iter()
      .map(|v| DataValue {
        wrapped: v.clone(),
        data_type: data.data_type
      })
      .collect())
  } else {
    Err(anyhow!("Value {} is not an array", data.wrapped))
  }
}

fn find_field_values<'a>(
  fields: &'a mut HashMap<u32, Vec<ProtobufField>>,
  field_name: &str
) -> Option<&'a mut Vec<ProtobufField>> {
  fields.iter_mut()
    .find(|(_, fields)| fields.iter().any(|field| field.field_name == field_name))
    .map(|(_, fields)| fields)
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
      Ok(fields) => Ok(Some(DynamicMessage::new(&self.descriptor, fields.as_slice(), &self.file_descriptor_set))),
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
  use maplit::hashmap;
  use pact_models::generators::GeneratorTestMode;
  use pact_models::path_exp::DocPath;
  use pact_models::prelude::Generator::RandomInt;
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
    let descriptor = DescriptorProto::default();
    let mut message = DynamicMessage::new(&descriptor, fields.as_slice(), &descriptors);
    let path = DocPath::new("$.one.two.three").unwrap();
    expect!(message.fetch_field_value(&path)).to(be_none());
  }

  #[test]
  fn dynamic_message_fetch_value_with_no_root() {
    let field = ProtobufField {
      field_num: 1,
      field_name: "one".to_string(),
      wire_type: WireType::Varint,
      data: ProtobufFieldData::Integer64(100),
      descriptor: Default::default()
    };
    let descriptors = FileDescriptorSet {
      file: vec![]
    };
    let fields = vec![ field.clone() ];
    let descriptor = DescriptorProto::default();
    let mut message = DynamicMessage::new(&descriptor, fields.as_slice(), &descriptors);
    let path = DocPath::new("one").unwrap();
    expect!(message.fetch_field_value(&path)).to(be_some().value(fields));
  }

  #[test]
  fn dynamic_message_fetch_value_with_matching_field() {
    let field = ProtobufField {
      field_num: 1,
      field_name: "one".to_string(),
      wire_type: WireType::Varint,
      data: ProtobufFieldData::Integer64(100),
      descriptor: Default::default()
    };
    let descriptors = FileDescriptorSet {
      file: vec![]
    };
    let descriptor = DescriptorProto::default();
    let fields = vec![ field.clone() ];
    let mut message = DynamicMessage::new(&descriptor, fields.as_slice(), &descriptors);
    let path = DocPath::new("$.one").unwrap();
    expect!(message.fetch_field_value(&path)).to(be_some().value(fields));
  }

  #[test]
  fn dynamic_message_fetch_value_with_matching_child_field() {
    let child_proto_1 = FieldDescriptorProto {
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
    };
    let child_proto_2 = FieldDescriptorProto {
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
    };
    let child_descriptor = DescriptorProto {
      name: Some("child".to_string()),
      field: vec![
        child_proto_1.clone(),
        child_proto_2.clone()
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
      data: ProtobufFieldData::Integer64(100),
      descriptor: child_proto_1.clone()
    };
    let child_field2 = ProtobufField {
      field_num: 2,
      field_name: "three".to_string(),
      wire_type: WireType::Varint,
      data: ProtobufFieldData::Integer64(200),
      descriptor: child_proto_2.clone()
    };
    let descriptors = FileDescriptorSet {
      file: vec![]
    };
    let descriptor = DescriptorProto::default();
    let child_message = DynamicMessage::new(&child_descriptor, &[child_field.clone(), child_field2], &descriptors);
    let mut buffer = BytesMut::new();
    child_message.write_to(&mut buffer).unwrap();
    let field = ProtobufField {
      field_num: 1,
      field_name: "one".to_string(),
      wire_type: WireType::LengthDelimited,
      data: ProtobufFieldData::Message(buffer.to_vec(), child_descriptor),
      descriptor: child_proto_1.clone()
    };
    let fields = vec![ field.clone() ];
    let mut message = DynamicMessage::new(&descriptor, fields.as_slice(), &descriptors);
    let path = DocPath::new("$.one.two").unwrap();
    expect!(message.fetch_field_value(&path)).to(be_some().value(vec![child_field]));
  }

  #[test]
  fn dynamic_message_generate_value_with_no_fields() {
    let fields = vec![];
    let descriptors = FileDescriptorSet {
      file: vec![]
    };
    let descriptor = DescriptorProto::default();
    let mut message = DynamicMessage::new(&descriptor, fields.as_slice(), &descriptors);
    let path = DocPath::new_unwrap("$.one.two.three");
    let generators = hashmap!{
      path.clone() => RandomInt(1, 10)
    };

    expect!(message.apply_generators(Some(&generators), &GeneratorTestMode::Provider, &hashmap!{})).to(be_ok());
  }

  #[test]
  fn dynamic_message_generate_value_with_no_matching_field() {
    let field = ProtobufField {
      field_num: 1,
      field_name: "one".to_string(),
      wire_type: WireType::Varint,
      data: ProtobufFieldData::Integer64(100),
      descriptor: Default::default()
    };
    let descriptors = FileDescriptorSet {
      file: vec![]
    };
    let fields = vec![ field.clone() ];
    let descriptor = DescriptorProto::default();
    let mut message = DynamicMessage::new(&descriptor, fields.as_slice(), &descriptors);
    let generators = hashmap!{
      DocPath::new_unwrap("$.two") => RandomInt(1, 10)
    };

    expect!(message.apply_generators(Some(&generators), &GeneratorTestMode::Provider, &hashmap!{})).to(be_ok());
  }

  #[test]
  fn dynamic_message_generate_value_with_matching_field() {
    let field = ProtobufField {
      field_num: 1,
      field_name: "one".to_string(),
      wire_type: WireType::Varint,
      data: ProtobufFieldData::Integer64(100),
      descriptor: Default::default()
    };
    let descriptors = FileDescriptorSet {
      file: vec![]
    };
    let fields = vec![ field.clone() ];
    let descriptor = DescriptorProto::default();
    let mut message = DynamicMessage::new(&descriptor, fields.as_slice(), &descriptors, );
    let generators = hashmap!{
      DocPath::new_unwrap("$.one") => RandomInt(1, 10)
    };

    expect!(message.apply_generators(Some(&generators), &GeneratorTestMode::Provider, &hashmap!{})).to(be_ok());
    expect!(message.proto_fields()[0].data.as_i64().unwrap()).to_not(be_equal_to(100));
  }

  #[test]
  fn dynamic_message_generate_value_with_matching_child_field() {
    let child_proto_1 = FieldDescriptorProto {
      name: Some("two".to_string()),
      number: Some(1),
      r#type: Some(3),
      ..FieldDescriptorProto::default()
    };
    let child_proto_2 = FieldDescriptorProto {
      name: Some("three".to_string()),
      number: Some(2),
      r#type: Some(3),
      ..FieldDescriptorProto::default()
    };
    let child_descriptor = DescriptorProto {
      name: Some("child".to_string()),
      field: vec![
        child_proto_1.clone(),
        child_proto_2.clone()
      ],
      .. DescriptorProto::default()
    };
    let child_field = ProtobufField {
      field_num: 1,
      field_name: "two".to_string(),
      wire_type: WireType::Varint,
      data: ProtobufFieldData::Integer64(100),
      descriptor: child_proto_1.clone()
    };
    let child_field2 = ProtobufField {
      field_num: 2,
      field_name: "three".to_string(),
      wire_type: WireType::Varint,
      data: ProtobufFieldData::Integer64(200),
      descriptor: child_proto_2.clone()
    };
    let descriptors = FileDescriptorSet {
      file: vec![]
    };
    let child_message = DynamicMessage::new(&child_descriptor, &[child_field.clone(), child_field2], &descriptors);
    let mut buffer = BytesMut::new();
    child_message.write_to(&mut buffer).unwrap();
    let field = ProtobufField {
      field_num: 1,
      field_name: "one".to_string(),
      wire_type: WireType::LengthDelimited,
      data: ProtobufFieldData::Message(buffer.to_vec(), child_descriptor),
      descriptor: child_proto_1.clone()
    };
    let fields = vec![ field.clone() ];
    let descriptor = DescriptorProto::default();
    let mut message = DynamicMessage::new(&descriptor, fields.as_slice(), &descriptors);
    let path = DocPath::new_unwrap("$.one.two");
    let generators = hashmap!{
      path.clone() => RandomInt(1, 10)
    };

    expect!(message.apply_generators(Some(&generators), &GeneratorTestMode::Provider, &hashmap!{})).to(be_ok());
    expect!(message.fetch_field_value(&path).unwrap().first().unwrap().data.as_i64().unwrap()).to_not(be_equal_to(100));
  }
}
