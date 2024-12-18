//! gRPC codec that used a Pact interaction

use std::collections::HashMap;
use std::iter::Peekable;
use std::slice::Iter;

use anyhow::{anyhow, bail};
use bytes::{Buf, BufMut, Bytes};
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

/// Dynamic message support based on a vector of ProtobufField field values. Internally, it will
/// consolidate all fields with the same field number.
#[derive(Debug, Clone)]
pub struct DynamicMessage {
  fields: HashMap<u32, ProtobufField>,
  descriptors: FileDescriptorSet
}

impl DynamicMessage {
  /// Create a new message from the slice of fields
  pub fn new(
    field_data: &[ProtobufField],
    descriptors: &FileDescriptorSet
  ) -> DynamicMessage {
    let fields = field_data.iter()
      .map(|f| (f.field_num, f.clone()))
      .into_group_map();
    let fields = fields.iter()
      .map(|(field_num, fields)| {
        let mut fields = fields.clone();
        let field = fields.iter_mut()
          .reduce(|field, f| {
            field.additional_data.push(f.data.clone());
            field
          } )
          .unwrap(); // safe to unwrap, the group by above can't create an empty vector.
        (*field_num, field.clone())
      })
      .collect();
    DynamicMessage {
      fields,
      descriptors: descriptors.clone()
    }
  }

  /// Return a vector of the fields
  pub fn proto_fields(&self) -> Vec<ProtobufField> {
    self.fields.values().cloned().collect()
  }

  /// Return a flattened vector of the fields. This will expand repeated fields.
  pub fn flatten_fields(&self) -> Vec<ProtobufField> {
    self.fields.values()
      .flat_map(|f| {
        let mut result = vec![ f.clone() ];
        if f.repeated_field() && !f.additional_data.is_empty() {
          result.extend(f.additional_data.iter()
            .map(|d| f.clone_with_data(d)));
        }
        result
      })
      .collect()
  }

  /// Encode this message to the provided buffer
  pub fn write_to<B>(&self, buffer: &mut B) -> anyhow::Result<()> where B: BufMut {
    for (field_num, field) in self.fields.iter()
      .sorted_by(|(a, _), (b, _)| Ord::cmp(a, b)) {
      Self::write_field(buffer, *field_num, field, &field.data)?;
      if field.repeated_field() && !field.additional_data.is_empty() {
        for data in &field.additional_data {
          Self::write_field(buffer, *field_num, field, data)?;
        }
      }
    }
    Ok(())
  }

  fn write_field<B>(
    buffer: &mut B,
    field_num: u32,
    field: &ProtobufField,
    data: &ProtobufFieldData
  ) -> anyhow::Result<()> where B: BufMut {
    trace!(%field_num, %field, %data, "Writing field data");
    encode_key(field.field_num, field.wire_type, buffer);
    match field.wire_type {
      WireType::Varint => match data {
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
      WireType::SixtyFourBit => match data {
        ProtobufFieldData::UInteger64(n) => buffer.put_u64_le(*n),
        ProtobufFieldData::Integer64(n) => buffer.put_i64_le(*n),
        ProtobufFieldData::Double(n) => buffer.put_f64_le(*n),
        ProtobufFieldData::Unknown(b) => {
          debug!("Writing unknown field {}", field.data);
          buffer.put_slice(b.as_slice());
        }
        _ => return Err(anyhow!("Expected a 64 bit value, but field is {}", field.data))
      }
      WireType::LengthDelimited => match data {
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
      WireType::ThirtyTwoBit => match data {
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
    Ok(())
  }

  /// Retrieve the value for a message field using the given path
  #[instrument(ret, skip(self), fields(path = %path))]
  pub fn fetch_field_value(&mut self, path: &DocPath) -> Option<ProtobufField> {
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
          if index == 0 {
            v.data = value.clone();
          } else {
            let additional_index = index - 1;
            if additional_index >= v.additional_data.len() {
              v.additional_data.resize(additional_index + 1, v.data.clone());
            }
            v.additional_data[additional_index] = value.clone();
          }
        } else {
          v.data = value.clone();
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
  ) -> anyhow::Result<()> where F: FnOnce(&mut ProtobufField, Option<PathToken>) {
    let descriptors = self.descriptors.clone();
    let fields = &mut self.fields;
    if let Some(next) = path_tokens.next() {
      match next {
        PathToken::Root => {},
        PathToken::Field(name) => return if let Some(field) = find_field_value(fields, name.as_str()) {
          if path_tokens.peek().is_none() {
            callback(field, None);
            Ok(())
          } else {
            match &mut field.data {
              ProtobufFieldData::Enum(_, _) => Err(anyhow!("Support for dynamically fetching enum values is not supported yet")),
              ProtobufFieldData::Message(data, descriptor) => {
                let mut buffer = Bytes::copy_from_slice(data);
                match decode_message(&mut buffer, descriptor, &descriptors) {
                  Ok(fields) => {
                    let mut message = DynamicMessage::new(fields.as_slice(), &descriptors);
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
                    segments can only be applied to a child message, but field type is '{}')", field.data.type_name()))
                  }
                }
                Some(PathToken::Index(index)) => if field.repeated_field() && path_tokens.peek().is_none() {
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
        if let Some(value) = &value {
          if generator.corresponds_to_mode(mode) {
            match generator.generate_value(&value.data, &context, &vm_boxed) {
              Ok(generated_value) => {
                self.set_field_value(&path, generated_value)?;
              }
              Err(err) => {
                warn!("Failed to apply generator '{}' for field {}: {}", path, value, err);
                if let Some(GeneratorError::ProviderStateValueIsCollection(val)) = err.downcast_ref::<GeneratorError>() {
                  if value.repeated_field() && val.wrapped.is_array() {
                    let array = as_array(val)?;
                    trace!("Applying a array value ({} items) to repeated field '{}'", array.len(), value.field_name);
                    for (index, dv) in array.iter().enumerate() {
                      let index_path = path.join_index(index);
                      let pv = data_value_to_proto_value(&value.data, dv)?;
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

fn find_field_value<'a>(
  fields: &'a mut HashMap<u32, ProtobufField>,
  field_name: &str
) -> Option<&'a mut ProtobufField> {
  fields.iter_mut()
    .find(|(_, field)| field.field_name == field_name)
    .map(|(_, field)| field)
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

  #[instrument(skip_all, fields(bytes = src.remaining()))]
  fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Self::Error> {
    trace!("Incoming bytes = {:?}", src);
    match decode_message(src, &self.descriptor, &self.file_descriptor_set) {
      Ok(fields) => Ok(Some(DynamicMessage::new(fields.as_slice(), &self.file_descriptor_set))),
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
  use pact_models::generators::Generator::ProviderStateGenerator;
  use pact_models::generators::GeneratorTestMode;
  use pact_models::path_exp::DocPath;
  use pact_models::prelude::Generator::RandomInt;
  use pretty_assertions::assert_eq;
  use prost::encoding::WireType;
  use prost_types::{DescriptorProto, field_descriptor_proto, FieldDescriptorProto, FileDescriptorProto, FileDescriptorSet};
  use serde_json::json;

  use crate::dynamic_message::DynamicMessage;
  use crate::message_decoder::{decode_message, ProtobufField, ProtobufFieldData};

  #[test]
  fn dynamic_message_fetch_value_with_no_fields() {
    let fields = vec![];
    let descriptors = FileDescriptorSet {
      file: vec![]
    };
    let mut message = DynamicMessage::new(fields.as_slice(), &descriptors);
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
      additional_data: vec![],
      descriptor: Default::default()
    };
    let descriptors = FileDescriptorSet {
      file: vec![]
    };
    let fields = vec![ field.clone() ];
    let mut message = DynamicMessage::new(fields.as_slice(), &descriptors);
    let path = DocPath::new("one").unwrap();
    expect!(message.fetch_field_value(&path)).to(be_some().value(field));
  }

  #[test]
  fn dynamic_message_fetch_value_with_matching_field() {
    let field = ProtobufField {
      field_num: 1,
      field_name: "one".to_string(),
      wire_type: WireType::Varint,
      data: ProtobufFieldData::Integer64(100),
      additional_data: vec![],
      descriptor: Default::default()
    };
    let descriptors = FileDescriptorSet {
      file: vec![]
    };
    let fields = vec![ field.clone() ];
    let mut message = DynamicMessage::new(fields.as_slice(), &descriptors);
    let path = DocPath::new("$.one").unwrap();
    expect!(message.fetch_field_value(&path)).to(be_some().value(field));
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
      additional_data: vec![],
      descriptor: child_proto_1.clone()
    };
    let child_field2 = ProtobufField {
      field_num: 2,
      field_name: "three".to_string(),
      wire_type: WireType::Varint,
      data: ProtobufFieldData::Integer64(200),
      additional_data: vec![],
      descriptor: child_proto_2.clone()
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
      data: ProtobufFieldData::Message(buffer.to_vec(), child_descriptor),
      additional_data: vec![],
      descriptor: child_proto_1.clone()
    };
    let fields = vec![ field.clone() ];
    let mut message = DynamicMessage::new(fields.as_slice(), &descriptors);
    let path = DocPath::new("$.one.two").unwrap();
    expect!(message.fetch_field_value(&path)).to(be_some().value(child_field));
  }

  #[test]
  fn dynamic_message_generate_value_with_no_fields() {
    let fields = vec![];
    let descriptors = FileDescriptorSet {
      file: vec![]
    };
    let mut message = DynamicMessage::new(fields.as_slice(), &descriptors);
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
      additional_data: vec![],
      descriptor: Default::default()
    };
    let descriptors = FileDescriptorSet {
      file: vec![]
    };
    let fields = vec![ field.clone() ];
    let mut message = DynamicMessage::new(fields.as_slice(), &descriptors);
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
      additional_data: vec![],
      descriptor: Default::default()
    };
    let descriptors = FileDescriptorSet {
      file: vec![]
    };
    let fields = vec![ field.clone() ];
    let mut message = DynamicMessage::new(fields.as_slice(), &descriptors);
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
      .. FieldDescriptorProto::default()
    };
    let child_proto_2 = FieldDescriptorProto {
      name: Some("three".to_string()),
      number: Some(2),
      r#type: Some(3),
      .. FieldDescriptorProto::default()
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
      additional_data: vec![],
      descriptor: child_proto_1.clone()
    };
    let child_field2 = ProtobufField {
      field_num: 2,
      field_name: "three".to_string(),
      wire_type: WireType::Varint,
      data: ProtobufFieldData::Integer64(200),
      additional_data: vec![],
      descriptor: child_proto_2.clone()
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
      data: ProtobufFieldData::Message(buffer.to_vec(), child_descriptor),
      additional_data: vec![],
      descriptor: child_proto_1.clone()
    };
    let fields = vec![ field.clone() ];
    let mut message = DynamicMessage::new(fields.as_slice(), &descriptors);
    let path = DocPath::new_unwrap("$.one.two");
    let generators = hashmap!{
      path.clone() => RandomInt(1, 10)
    };

    expect!(message.apply_generators(Some(&generators), &GeneratorTestMode::Provider, &hashmap!{})).to(be_ok());
    expect!(message.fetch_field_value(&path).unwrap().data.as_i64().unwrap()).to_not(be_equal_to(100));
  }

  #[test]
  fn dynamic_message_inject_array_into_repeated_field() {
    let field_descriptor = FieldDescriptorProto {
      label: Some(field_descriptor_proto::Label::Repeated as i32),
      .. FieldDescriptorProto::default()
    };
    let field = ProtobufField {
      field_num: 1,
      field_name: "one".to_string(),
      wire_type: WireType::Varint,
      data: ProtobufFieldData::Integer64(100),
      additional_data: vec![],
      descriptor: field_descriptor.clone()
    };
    let descriptors = FileDescriptorSet {
      file: vec![]
    };
    let fields = vec![ field.clone() ];
    let mut message = DynamicMessage::new(fields.as_slice(), &descriptors);
    let generators = hashmap!{
      DocPath::new_unwrap("$.one") => ProviderStateGenerator("a".to_string(), None)
    };

    let context = hashmap!{
      "a" => json!([1, 2, "3", 4])
    };
    expect!(message.apply_generators(Some(&generators), &GeneratorTestMode::Provider, &context)).to(be_ok());

    let generated_fields = message.proto_fields();
    expect!(generated_fields.len()).to(be_equal_to(1));

    let resulting_field = &generated_fields[0];
    expect!(resulting_field.data.as_i64().unwrap()).to(be_equal_to(1));
    expect!(resulting_field.additional_data.len()).to(be_equal_to(3));
    expect!(resulting_field.additional_data[0].as_i64().unwrap()).to(be_equal_to(2));
    expect!(resulting_field.additional_data[1].as_i64().unwrap()).to(be_equal_to(3));
    expect!(resulting_field.additional_data[2].as_i64().unwrap()).to(be_equal_to(4));
  }

  #[test]
  fn dynamic_message_inject_array_into_non_repeated_field() {
    let field_descriptor = FieldDescriptorProto {
      label: None,
      .. FieldDescriptorProto::default()
    };
    let field = ProtobufField {
      field_num: 1,
      field_name: "one".to_string(),
      wire_type: WireType::Varint,
      data: ProtobufFieldData::Integer64(100),
      additional_data: vec![],
      descriptor: field_descriptor.clone()
    };
    let descriptors = FileDescriptorSet {
      file: vec![]
    };
    let fields = vec![ field.clone() ];
    let mut message = DynamicMessage::new(fields.as_slice(), &descriptors);
    let generators = hashmap!{
      DocPath::new_unwrap("$.one") => ProviderStateGenerator("a".to_string(), None)
    };

    let context = hashmap!{
      "a" => json!([1, 2, 3, 4])
    };
    let result = message.apply_generators(Some(&generators), &GeneratorTestMode::Provider, &context);
    expect!(result.as_ref()).to(be_err());
    expect!(result.unwrap_err().to_string()).to(be_equal_to(
      "Provider state value is a collection (Array or Object), and can not be injected into a single field"));
  }

  #[test]
  fn dynamic_message_inject_array_with_incorrect_type() {
    let field_descriptor = FieldDescriptorProto {
      label: Some(field_descriptor_proto::Label::Repeated as i32),
      .. FieldDescriptorProto::default()
    };
    let field = ProtobufField {
      field_num: 1,
      field_name: "one".to_string(),
      wire_type: WireType::Varint,
      data: ProtobufFieldData::Integer64(100),
      additional_data: vec![],
      descriptor: field_descriptor.clone()
    };
    let descriptors = FileDescriptorSet {
      file: vec![]
    };
    let fields = vec![ field.clone() ];
    let mut message = DynamicMessage::new(fields.as_slice(), &descriptors);
    let generators = hashmap!{
      DocPath::new_unwrap("$.one") => ProviderStateGenerator("a".to_string(), None)
    };

    let context = hashmap!{
      "a" => json!([1, 2, "sss", 4])
    };
    let result = message.apply_generators(Some(&generators), &GeneratorTestMode::Provider, &context);
    expect!(result.as_ref()).to(be_err());
    expect!(result.unwrap_err().to_string()).to(be_equal_to(
      "i64 can not be generated from 'sss' - invalid digit found in string"));
  }

  #[test]
  fn dynamic_message_write_to_test() {
    let field_descriptor = FieldDescriptorProto {
      name: Some("one".to_string()),
      number: Some(1),
      r#type: Some(field_descriptor_proto::Type::Int64 as i32),
      label: None,
      .. FieldDescriptorProto::default()
    };
    let field = ProtobufField {
      field_num: 1,
      field_name: "one".to_string(),
      wire_type: WireType::Varint,
      data: ProtobufFieldData::Integer64(100),
      additional_data: vec![],
      descriptor: field_descriptor.clone()
    };
    let descriptors = FileDescriptorSet {
      file: vec![]
    };
    let fields = vec![ field.clone() ];
    let descriptor = DescriptorProto {
      field: vec![
        field_descriptor.clone()
      ],
      .. DescriptorProto::default()
    };
    let message = DynamicMessage::new(fields.as_slice(), &descriptors);

    let mut buffer = BytesMut::new();
    message.write_to(&mut buffer).unwrap();

    let result = decode_message(&mut buffer.freeze(), &descriptor, &descriptors).unwrap();
    expect!(result).to(be_equal_to(vec![ field ]));
  }

  #[test]
  fn dynamic_message_write_to_test_with_multiple_fields() {
    let field_descriptor_1 = FieldDescriptorProto {
      name: Some("one".to_string()),
      number: Some(1),
      r#type: Some(field_descriptor_proto::Type::Int64 as i32),
      label: None,
      .. FieldDescriptorProto::default()
    };
    let field_1 = ProtobufField {
      field_num: 1,
      field_name: "one".to_string(),
      wire_type: WireType::Varint,
      data: ProtobufFieldData::Integer64(100),
      additional_data: vec![],
      descriptor: field_descriptor_1.clone()
    };

    let field_descriptor_2 = FieldDescriptorProto {
      name: Some("two".to_string()),
      number: Some(2),
      r#type: Some(field_descriptor_proto::Type::String as i32),
      label: None,
      .. FieldDescriptorProto::default()
    };
    let field_2 = ProtobufField {
      field_num: 2,
      field_name: "two".to_string(),
      wire_type: WireType::LengthDelimited,
      data: ProtobufFieldData::String("test".to_string()),
      additional_data: vec![],
      descriptor: field_descriptor_2.clone()
    };

    let field_descriptor_3 = FieldDescriptorProto {
      name: Some("three".to_string()),
      number: Some(3),
      r#type: Some(field_descriptor_proto::Type::Bool as i32),
      label: None,
      .. FieldDescriptorProto::default()
    };
    let field_3 = ProtobufField {
      field_num: 3,
      field_name: "three".to_string(),
      wire_type: WireType::Varint,
      data: ProtobufFieldData::Boolean(true),
      additional_data: vec![],
      descriptor: field_descriptor_3.clone()
    };

    let descriptors = FileDescriptorSet {
      file: vec![]
    };
    let fields = vec![ field_1.clone(), field_3.clone(), field_2.clone() ];
    let descriptor = DescriptorProto {
      field: vec![
        field_descriptor_1.clone(),
        field_descriptor_2.clone(),
        field_descriptor_3.clone()
      ],
      .. DescriptorProto::default()
    };
    let message = DynamicMessage::new(fields.as_slice(), &descriptors);

    let mut buffer = BytesMut::new();
    message.write_to(&mut buffer).unwrap();

    let result = decode_message(&mut buffer.freeze(), &descriptor, &descriptors).unwrap();
    expect!(result).to(be_equal_to(vec![ field_1, field_2, field_3 ]));
  }

  #[test]
  fn dynamic_message_write_to_test_with_child_field() {
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
      additional_data: vec![],
      descriptor: child_proto_1.clone()
    };
    let child_field2 = ProtobufField {
      field_num: 2,
      field_name: "three".to_string(),
      wire_type: WireType::Varint,
      data: ProtobufFieldData::Integer64(200),
      additional_data: vec![],
      descriptor: child_proto_2.clone()
    };

    let field_descriptor = FieldDescriptorProto {
      name: Some("one".to_string()),
      number: Some(1),
      r#type: Some(field_descriptor_proto::Type::Message as i32),
      type_name: Some("child".to_string()),
      label: None,
      .. FieldDescriptorProto::default()
    };
    let descriptor = DescriptorProto {
      name: Some("parent".to_string()),
      field: vec![
        field_descriptor.clone()
      ],
      .. DescriptorProto::default()
    };
    let descriptors = FileDescriptorSet {
      file: vec![
        FileDescriptorProto {
          message_type: vec![
            descriptor.clone(), child_descriptor.clone()
          ],
          .. FileDescriptorProto::default()
        }
      ]
    };

    let child_message = DynamicMessage::new(&[child_field.clone(), child_field2], &descriptors);
    let mut child_buffer = BytesMut::new();
    child_message.write_to(&mut child_buffer).unwrap();

    let field = ProtobufField {
      field_num: 1,
      field_name: "one".to_string(),
      wire_type: WireType::LengthDelimited,
      data: ProtobufFieldData::Message(child_buffer.to_vec(), child_descriptor),
      additional_data: vec![],
      descriptor: field_descriptor.clone()
    };
    let fields = vec![ field.clone() ];
    let message = DynamicMessage::new(fields.as_slice(), &descriptors);

    let mut buffer = BytesMut::new();
    message.write_to(&mut buffer).unwrap();

    let result = decode_message(&mut buffer.freeze(), &descriptor, &descriptors).unwrap();
    assert_eq!(result, vec![ field ]);
  }

  #[test]
  fn dynamic_message_write_to_test_with_repeated_fields() {
    let field_descriptor_1 = FieldDescriptorProto {
      name: Some("one".to_string()),
      number: Some(1),
      r#type: Some(field_descriptor_proto::Type::Int64 as i32),
      label: Some(field_descriptor_proto::Label::Repeated as i32),
      .. FieldDescriptorProto::default()
    };
    let field_1_1 = ProtobufField {
      field_num: 1,
      field_name: "one".to_string(),
      wire_type: WireType::Varint,
      data: ProtobufFieldData::Integer64(100),
      additional_data: vec![],
      descriptor: field_descriptor_1.clone()
    };
    let field_1_2 = ProtobufField {
      field_num: 1,
      field_name: "one".to_string(),
      wire_type: WireType::Varint,
      data: ProtobufFieldData::Integer64(101),
      additional_data: vec![],
      descriptor: field_descriptor_1.clone()
    };
    let field_1_3 = ProtobufField {
      field_num: 1,
      field_name: "one".to_string(),
      wire_type: WireType::Varint,
      data: ProtobufFieldData::Integer64(102),
      additional_data: vec![],
      descriptor: field_descriptor_1.clone()
    };

    let field_descriptor_2 = FieldDescriptorProto {
      name: Some("two".to_string()),
      number: Some(2),
      r#type: Some(field_descriptor_proto::Type::String as i32),
      label: None,
      .. FieldDescriptorProto::default()
    };
    let field_2 = ProtobufField {
      field_num: 2,
      field_name: "two".to_string(),
      wire_type: WireType::LengthDelimited,
      data: ProtobufFieldData::String("test".to_string()),
      additional_data: vec![],
      descriptor: field_descriptor_2.clone()
    };

    let field_descriptor_3 = FieldDescriptorProto {
      name: Some("three".to_string()),
      number: Some(3),
      r#type: Some(field_descriptor_proto::Type::Bool as i32),
      label: None,
      .. FieldDescriptorProto::default()
    };
    let field_3 = ProtobufField {
      field_num: 3,
      field_name: "three".to_string(),
      wire_type: WireType::Varint,
      data: ProtobufFieldData::Boolean(true),
      additional_data: vec![],
      descriptor: field_descriptor_3.clone()
    };

    let descriptors = FileDescriptorSet {
      file: vec![]
    };
    let fields = vec![
      field_1_1.clone(),
      field_3.clone(),
      field_1_2.clone(),
      field_2.clone(),
      field_1_3.clone()
    ];
    let descriptor = DescriptorProto {
      field: vec![
        field_descriptor_1.clone(),
        field_descriptor_2.clone(),
        field_descriptor_3.clone()
      ],
      .. DescriptorProto::default()
    };
    let message = DynamicMessage::new(fields.as_slice(), &descriptors);

    let mut buffer = BytesMut::new();
    message.write_to(&mut buffer).unwrap();

    let result = decode_message(&mut buffer.freeze(), &descriptor, &descriptors).unwrap();
    expect!(result).to(be_equal_to(vec![ field_1_1, field_1_2, field_1_3, field_2, field_3 ]));
  }

  #[test]
  fn dynamic_message_write_to_test_with_repeated_field_with_additional_values() {
    let field_descriptor_1 = FieldDescriptorProto {
      name: Some("one".to_string()),
      number: Some(1),
      r#type: Some(field_descriptor_proto::Type::Int64 as i32),
      label: Some(field_descriptor_proto::Label::Repeated as i32),
      .. FieldDescriptorProto::default()
    };
    let field_1 = ProtobufField {
      field_num: 1,
      field_name: "one".to_string(),
      wire_type: WireType::Varint,
      data: ProtobufFieldData::Integer64(100),
      additional_data: vec![ ProtobufFieldData::Integer64(101), ProtobufFieldData::Integer64(102) ],
      descriptor: field_descriptor_1.clone()
    };

    let field_descriptor_2 = FieldDescriptorProto {
      name: Some("two".to_string()),
      number: Some(2),
      r#type: Some(field_descriptor_proto::Type::String as i32),
      label: None,
      .. FieldDescriptorProto::default()
    };
    let field_2 = ProtobufField {
      field_num: 2,
      field_name: "two".to_string(),
      wire_type: WireType::LengthDelimited,
      data: ProtobufFieldData::String("test".to_string()),
      additional_data: vec![],
      descriptor: field_descriptor_2.clone()
    };

    let field_descriptor_3 = FieldDescriptorProto {
      name: Some("three".to_string()),
      number: Some(3),
      r#type: Some(field_descriptor_proto::Type::Bool as i32),
      label: None,
      .. FieldDescriptorProto::default()
    };
    let field_3 = ProtobufField {
      field_num: 3,
      field_name: "three".to_string(),
      wire_type: WireType::Varint,
      data: ProtobufFieldData::Boolean(true),
      additional_data: vec![],
      descriptor: field_descriptor_3.clone()
    };

    let descriptors = FileDescriptorSet {
      file: vec![]
    };
    let fields = vec![
      field_1.clone(),
      field_3.clone(),
      field_2.clone()
    ];
    let descriptor = DescriptorProto {
      field: vec![
        field_descriptor_1.clone(),
        field_descriptor_2.clone(),
        field_descriptor_3.clone()
      ],
      .. DescriptorProto::default()
    };
    let message = DynamicMessage::new(fields.as_slice(), &descriptors);

    let mut buffer = BytesMut::new();
    message.write_to(&mut buffer).unwrap();

    let result = decode_message(&mut buffer.freeze(), &descriptor, &descriptors).unwrap();
    let field_1_1 = ProtobufField {
      field_num: 1,
      field_name: "one".to_string(),
      wire_type: WireType::Varint,
      data: ProtobufFieldData::Integer64(100),
      additional_data: vec![],
      descriptor: field_descriptor_1.clone()
    };
    let field_1_2 = ProtobufField {
      field_num: 1,
      field_name: "one".to_string(),
      wire_type: WireType::Varint,
      data: ProtobufFieldData::Integer64(101),
      additional_data: vec![],
      descriptor: field_descriptor_1.clone()
    };
    let field_1_3 = ProtobufField {
      field_num: 1,
      field_name: "one".to_string(),
      wire_type: WireType::Varint,
      data: ProtobufFieldData::Integer64(102),
      additional_data: vec![],
      descriptor: field_descriptor_1.clone()
    };
    assert_eq!(result, vec![ field_1_1, field_1_2, field_1_3, field_2, field_3 ]);
  }
}
