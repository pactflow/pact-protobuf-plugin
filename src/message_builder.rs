//! Builder for creating protobuf messages based on a descriptor

use std::collections::HashMap;

use anyhow::anyhow;
use bytes::{Buf, Bytes, BytesMut};
use itertools::Itertools;
use maplit::hashmap;
use prost::encoding::{encode_key, encode_varint, string, WireType};
use prost::encoding::int32::encode_packed;
use prost::encoding::int32::encode;
use prost::Message;
use prost_types::{DescriptorProto, FieldDescriptorProto};
use prost_types::field_descriptor_proto::Type;

/// Enum to set what type of field the value is for
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MessageFieldValueType {
  /// Normal field value
  Normal,
  /// Map field value
  Map,
  /// Repeated field value
  Repeated
}

/// Inner struct to store the values for a field
#[derive(Clone, Debug, PartialEq)]
struct FieldValueInner {
  /// Values for the field, only repeated fields will have more than one value.
  values: Vec<MessageFieldValue>,
  /// Descriptor for the field.
  descriptor: FieldDescriptorProto,
  /// Type of field
  field_type: MessageFieldValueType
}

/// Builder struct for a Protobuf message
#[derive(Clone, Debug)]
pub struct MessageBuilder {
  /// Protobuf descriptor for the message
  pub descriptor: DescriptorProto,
  /// Message name
  pub message_name: String,

  fields: HashMap<String, FieldValueInner>
}

impl MessageBuilder {
  /// Create a new message builder for the message
  pub fn new(descriptor: &DescriptorProto, message_name: &str) -> Self {
    MessageBuilder {
      descriptor: descriptor.clone(),
      message_name: message_name.to_string(),
      fields: hashmap!{}
    }
  }

  /// Set the field to the given value
  pub fn set_field(&mut self, field_descriptor: &FieldDescriptorProto, field_name: &str, field_value: MessageFieldValue) -> &mut Self {
    self.fields.insert(field_name.to_string(), FieldValueInner {
      values: vec![ field_value ],
      descriptor: field_descriptor.clone(),
      field_type: MessageFieldValueType::Normal
    });
    self
  }

  /// Adds a value to a repeated field. If the field is not defined, configures it first.
  pub fn add_repeated_field_value(&mut self, field_descriptor: &FieldDescriptorProto, field_name: &str, field_value: MessageFieldValue) -> &mut Self {
    self
  }

  /// Encodes the Protobuf message into a bytes buffer
  pub fn encode_message(&self) -> anyhow::Result<Bytes> {
    let mut buffer = BytesMut::with_capacity(1024);

    for (_, field_data) in self.fields.iter()
      .sorted_by(|(_, a), (_, b)| Ord::cmp(&a.descriptor.number.unwrap_or_default(), &b.descriptor.number.unwrap_or_default())) {
      match field_data.field_type {
        MessageFieldValueType::Normal => if let Some(value) = field_data.values.first() {
          if let Some(tag) = field_data.descriptor.number {
            match &value.proto_type {
              Type::Double => {}
              Type::Float => {}
              Type::Int64 => {}
              Type::Uint64 => {}
              Type::Int32 => {}
              Type::Fixed64 => {}
              Type::Fixed32 => {}
              Type::Bool => {}
              Type::String => if let RType::String(s) = &value.rtype {
                string::encode(tag as u32, s, &mut buffer);
              } else {
                return Err(anyhow!("Mismatched types, expected a string but got {:?}", value.rtype));
              }
              Type::Group => {}
              Type::Message => {}
              Type::Bytes => if let RType::Bytes(b) = &value.rtype {
                prost::encoding::bytes::encode(tag as u32, b, &mut buffer);
              } else {
                return Err(anyhow!("Mismatched types, expected a byte array but got {:?}", value.rtype));
              }
              Type::Uint32 => {}
              Type::Enum => if let RType::Enum(name) = &value.rtype {
                self.encode_enum_value(&field_data.descriptor, value, tag, name, &mut buffer)?;
              } else {
                return Err(anyhow!("Mismatched types, expected an enum but got {:?}", value.rtype));
              }
              Type::Sfixed32 => {}
              Type::Sfixed64 => {}
              Type::Sint32 => {}
              Type::Sint64 => {}
            }
          }
        }
        MessageFieldValueType::Map => {}
        MessageFieldValueType::Repeated => {}
      }
    }

    Ok(buffer.freeze())
  }

  fn encode_enum_value(
    &self,
    descriptor: &FieldDescriptorProto,
    field_value: &MessageFieldValue,
    tag: i32,
    enum_value_name: &String,
    buffer: &mut BytesMut
  ) -> anyhow::Result<()> {
    let enum_type_name = descriptor.type_name.as_ref().ok_or_else(|| anyhow!("Type name is missing from the descriptor for enum field {}", field_value.name))?;
    let enum_name = enum_type_name.split('.').last().unwrap_or_else(|| enum_type_name.as_str());
    let enum_proto = self.descriptor.enum_type.iter().find(|enum_type| enum_type.name.clone().unwrap_or_default() == enum_name)
      .ok_or_else(|| anyhow!("Did not find the enum {} for the type {} in the Protobuf descriptor", enum_name, enum_type_name))?;
    let enum_value = enum_proto.value.iter().find(|enum_val| enum_val.name.clone().unwrap_or_default() == enum_value_name.as_str())
      .ok_or_else(|| anyhow!("Did not find the enum value {} for the enum {} in the Protobuf descriptor", enum_value_name, enum_type_name))?;
    if let Some(enum_value_number) = enum_value.number {
      encode_key(tag as u32, WireType::Varint, buffer);
      encode_varint(enum_value_number as u64, buffer);
      Ok(())
    } else {
      Err(anyhow!("Enum value {} for enum {} does not have a numeric value set", enum_value_name, enum_type_name))
    }
  }
}

/// Rust type to use for a protobuf type
#[derive(Clone, Debug, PartialEq)]
pub enum RType {
  /// String value
  String(String),
  /// Boolean value
  Boolean(bool),
  /// Unsigned 32 bit integer
  UInteger32(u32),
  /// Signed 32 bit integer
  Integer32(i32),
  /// Unsigned 64 bit integer
  UInteger64(u64),
  /// Signed 64 bit integer
  Integer64(i64),
  /// 32 bit floating point number
  Float(f32),
  /// 64 bit floating point number
  Double(f64),
  /// Array of bytes
  Bytes(Vec<u8>),
  /// Enum value
  Enum(String)
}

/// Value of a message field
#[derive(Clone, Debug, PartialEq)]
pub struct MessageFieldValue {
  /// Name of the field
  pub name: String,
  /// Raw value in text form
  pub raw_value: Option<String>,
  /// Rust type for the value
  pub rtype: RType,
  /// Protobuf type for the value
  pub proto_type: Type
}

impl MessageFieldValue {
  /// Create a String value
  pub fn string(field_name: &str, field_value: &str) -> MessageFieldValue {
    MessageFieldValue {
      name: field_name.to_string(),
      raw_value: Some(field_value.to_string()),
      rtype: RType::String(field_value.to_string()),
      proto_type: Type::String
    }
  }

  /// Create a boolean value. This will fail with an error if the value is not a valid boolean value.
  pub fn boolean(field_name: &str, field_value: &str) -> anyhow::Result<MessageFieldValue> {
    let v: bool = field_value.parse()?;
    Ok(MessageFieldValue {
      name: field_name.to_string(),
      raw_value: Some(field_value.to_string()),
      rtype: RType::Boolean(v),
      proto_type: Type::Bool
    })
  }

  /// Create an unsigned 32 bit integer value. This will fail with an error if the value is not a valid integer value.
  pub fn uinteger_32(field_name: &str, field_value: &str, proto_type: Type) -> anyhow::Result<MessageFieldValue> {
    let v: u32 = field_value.parse()?;
    Ok(MessageFieldValue {
      name: field_name.to_string(),
      raw_value: Some(field_value.to_string()),
      rtype: RType::UInteger32(v),
      proto_type
    })
  }

  /// Create a signed 32 bit integer value. This will fail with an error if the value is not a valid integer value.
  pub fn integer_32(field_name: &str, field_value: &str, proto_type: Type) -> anyhow::Result<MessageFieldValue> {
    let v: i32 = field_value.parse()?;
    Ok(MessageFieldValue {
      name: field_name.to_string(),
      raw_value: Some(field_value.to_string()),
      rtype: RType::Integer32(v),
      proto_type
    })
  }

  /// Create an unsigned 64 bit integer value. This will fail with an error if the value is not a valid integer value.
  pub fn uinteger_64(field_name: &str, field_value: &str, proto_type: Type) -> anyhow::Result<MessageFieldValue> {
    let v: u64 = field_value.parse()?;
    Ok(MessageFieldValue {
      name: field_name.to_string(),
      raw_value: Some(field_value.to_string()),
      rtype: RType::UInteger64(v),
      proto_type
    })
  }

  /// Create a signed 64 bit integer value. This will fail with an error if the value is not a valid integer value.
  pub fn integer_64(field_name: &str, field_value: &str, proto_type: Type) -> anyhow::Result<MessageFieldValue> {
    let v: i64 = field_value.parse()?;
    Ok(MessageFieldValue {
      name: field_name.to_string(),
      raw_value: Some(field_value.to_string()),
      rtype: RType::Integer64(v),
      proto_type
    })
  }

  /// Create an 32 bit floating point value. This will fail with an error if the value is not a valid float value.
  pub fn float(field_name: &str, field_value: &str, proto_type: Type) -> anyhow::Result<MessageFieldValue> {
    let v: f32 = field_value.parse()?;
    Ok(MessageFieldValue {
      name: field_name.to_string(),
      raw_value: Some(field_value.to_string()),
      rtype: RType::Float(v),
      proto_type
    })
  }

  /// Create an 64 bit floating point value. This will fail with an error if the value is not a valid float value.
  pub fn double(field_name: &str, field_value: &str, proto_type: Type) -> anyhow::Result<MessageFieldValue> {
    let v: f64 = field_value.parse()?;
    Ok(MessageFieldValue {
      name: field_name.to_string(),
      raw_value: Some(field_value.to_string()),
      rtype: RType::Double(v),
      proto_type
    })
  }

  /// Create a byte array value
  pub fn bytes(field_name: &str, field_value: &str) -> MessageFieldValue {
    MessageFieldValue {
      name: field_name.to_string(),
      raw_value: Some(field_value.to_string()),
      rtype: RType::Bytes(field_value.as_bytes().to_vec()),
      proto_type: Type::Bytes
    }
  }
}

#[cfg(test)]
mod tests {
  use bytes::Bytes;
  use expectest::prelude::*;
  use prost_types::{DescriptorProto, EnumDescriptorProto, EnumValueDescriptorProto, field_descriptor_proto, FieldDescriptorProto, FileDescriptorSet};
  use prost_types::field_descriptor_proto::Type;
  use pact_plugin_driver::proto::Body;
  use pact_plugin_driver::proto::body::ContentTypeHint;
  use prost::Message;

  use crate::message_builder::{MessageBuilder, MessageFieldValue, RType};

  #[test]
  fn encode_simple_message_test() {
    let field1 = FieldDescriptorProto {
      name: Some("implementation".to_string()),
      number: Some(1),
      label: None,
      r#type: Some(field_descriptor_proto::Type::String as i32),
      type_name: Some("String".to_string()),
      extendee: None,
      default_value: None,
      oneof_index: None,
      json_name: None,
      options: None,
      proto3_optional: None
    };
    let field2 = FieldDescriptorProto {
      name: Some("version".to_string()),
      number: Some(2),
      label: None,
      r#type: Some(field_descriptor_proto::Type::String as i32),
      type_name: Some("String".to_string()),
      extendee: None,
      default_value: None,
      oneof_index: None,
      json_name: None,
      options: None,
      proto3_optional: None
    };
    let descriptor = DescriptorProto {
      name: Some("InitPluginRequest".to_string()),
      field: vec![
        field1.clone(),
        field2.clone()
      ],
      extension: vec![],
      nested_type: vec![],
      enum_type: vec![],
      extension_range: vec![],
      oneof_decl: vec![],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![]
    };
    let mut message = MessageBuilder::new(&descriptor, "InitPluginRequest");
    message.set_field(&field1, "implementation", MessageFieldValue {
      name: "implementation".to_string(),
      raw_value: Some("plugin-driver-rust".to_string()),
      rtype: RType::String("plugin-driver-rust".to_string()),
      proto_type: Type::String
    });
    message.set_field(&field2, "version", MessageFieldValue {
      name: "version".to_string(),
      raw_value: Some("0.0.0".to_string()),
      rtype: RType::String("0.0.0".to_string()),
      proto_type: Type::String
    });

    let result = message.encode_message().unwrap();
    expect!(result.to_vec()).to(be_equal_to(base64::decode("ChJwbHVnaW4tZHJpdmVyLXJ1c3QSBTAuMC4w").unwrap()));
  }

  #[test]
  fn encode_message_bytes_test() {
    // let bytes = base64::decode("CuIFChxnb29nbGUvcHJvdG9idWYvc3RydWN0LnByb3RvEg9nb29nbGUucHJvdG9idWYimAEKBlN0cnVjdBI7CgZmaWVsZHMYASADKAsyIy5nb29nbGUucHJvdG9idWYuU3RydWN0LkZpZWxkc0VudHJ5UgZmaWVsZHMaUQoLRmllbGRzRW50cnkSEAoDa2V5GAEgASgJUgNrZXkSLAoFdmFsdWUYAiABKAsyFi5nb29nbGUucHJvdG9idWYuVmFsdWVSBXZhbHVlOgI4ASKyAgoFVmFsdWUSOwoKbnVsbF92YWx1ZRgBIAEoDjIaLmdvb2dsZS5wcm90b2J1Zi5OdWxsVmFsdWVIAFIJbnVsbFZhbHVlEiMKDG51bWJlcl92YWx1ZRgCIAEoAUgAUgtudW1iZXJWYWx1ZRIjCgxzdHJpbmdfdmFsdWUYAyABKAlIAFILc3RyaW5nVmFsdWUSHwoKYm9vbF92YWx1ZRgEIAEoCEgAUglib29sVmFsdWUSPAoMc3RydWN0X3ZhbHVlGAUgASgLMhcuZ29vZ2xlLnByb3RvYnVmLlN0cnVjdEgAUgtzdHJ1Y3RWYWx1ZRI7CgpsaXN0X3ZhbHVlGAYgASgLMhouZ29vZ2xlLnByb3RvYnVmLkxpc3RWYWx1ZUgAUglsaXN0VmFsdWVCBgoEa2luZCI7CglMaXN0VmFsdWUSLgoGdmFsdWVzGAEgAygLMhYuZ29vZ2xlLnByb3RvYnVmLlZhbHVlUgZ2YWx1ZXMqGwoJTnVsbFZhbHVlEg4KCk5VTExfVkFMVUUQAEJ/ChNjb20uZ29vZ2xlLnByb3RvYnVmQgtTdHJ1Y3RQcm90b1ABWi9nb29nbGUuZ29sYW5nLm9yZy9wcm90b2J1Zi90eXBlcy9rbm93bi9zdHJ1Y3RwYvgBAaICA0dQQqoCHkdvb2dsZS5Qcm90b2J1Zi5XZWxsS25vd25UeXBlc2IGcHJvdG8zCoYECh5nb29nbGUvcHJvdG9idWYvd3JhcHBlcnMucHJvdG8SD2dvb2dsZS5wcm90b2J1ZiIjCgtEb3VibGVWYWx1ZRIUCgV2YWx1ZRgBIAEoAVIFdmFsdWUiIgoKRmxvYXRWYWx1ZRIUCgV2YWx1ZRgBIAEoAlIFdmFsdWUiIgoKSW50NjRWYWx1ZRIUCgV2YWx1ZRgBIAEoA1IFdmFsdWUiIwoLVUludDY0VmFsdWUSFAoFdmFsdWUYASABKARSBXZhbHVlIiIKCkludDMyVmFsdWUSFAoFdmFsdWUYASABKAVSBXZhbHVlIiMKC1VJbnQzMlZhbHVlEhQKBXZhbHVlGAEgASgNUgV2YWx1ZSIhCglCb29sVmFsdWUSFAoFdmFsdWUYASABKAhSBXZhbHVlIiMKC1N0cmluZ1ZhbHVlEhQKBXZhbHVlGAEgASgJUgV2YWx1ZSIiCgpCeXRlc1ZhbHVlEhQKBXZhbHVlGAEgASgMUgV2YWx1ZUKDAQoTY29tLmdvb2dsZS5wcm90b2J1ZkINV3JhcHBlcnNQcm90b1ABWjFnb29nbGUuZ29sYW5nLm9yZy9wcm90b2J1Zi90eXBlcy9rbm93bi93cmFwcGVyc3Bi+AEBogIDR1BCqgIeR29vZ2xlLlByb3RvYnVmLldlbGxLbm93blR5cGVzYgZwcm90bzMKvgEKG2dvb2dsZS9wcm90b2J1Zi9lbXB0eS5wcm90bxIPZ29vZ2xlLnByb3RvYnVmIgcKBUVtcHR5Qn0KE2NvbS5nb29nbGUucHJvdG9idWZCCkVtcHR5UHJvdG9QAVouZ29vZ2xlLmdvbGFuZy5vcmcvcHJvdG9idWYvdHlwZXMva25vd24vZW1wdHlwYvgBAaICA0dQQqoCHkdvb2dsZS5Qcm90b2J1Zi5XZWxsS25vd25UeXBlc2IGcHJvdG8zCv0iCgxwbHVnaW4ucHJvdG8SDmlvLnBhY3QucGx1Z2luGhxnb29nbGUvcHJvdG9idWYvc3RydWN0LnByb3RvGh5nb29nbGUvcHJvdG9idWYvd3JhcHBlcnMucHJvdG8aG2dvb2dsZS9wcm90b2J1Zi9lbXB0eS5wcm90byJVChFJbml0UGx1Z2luUmVxdWVzdBImCg5pbXBsZW1lbnRhdGlvbhgBIAEoCVIOaW1wbGVtZW50YXRpb24SGAoHdmVyc2lvbhgCIAEoCVIHdmVyc2lvbiLHAgoOQ2F0YWxvZ3VlRW50cnkSPAoEdHlwZRgBIAEoDjIoLmlvLnBhY3QucGx1Z2luLkNhdGFsb2d1ZUVudHJ5LkVudHJ5VHlwZVIEdHlwZRIQCgNrZXkYAiABKAlSA2tleRJCCgZ2YWx1ZXMYAyADKAsyKi5pby5wYWN0LnBsdWdpbi5DYXRhbG9ndWVFbnRyeS5WYWx1ZXNFbnRyeVIGdmFsdWVzGjkKC1ZhbHVlc0VudHJ5EhAKA2tleRgBIAEoCVIDa2V5EhQKBXZhbHVlGAIgASgJUgV2YWx1ZToCOAEiZgoJRW50cnlUeXBlEhMKD0NPTlRFTlRfTUFUQ0hFUhAAEhUKEUNPTlRFTlRfR0VORVJBVE9SEAESDwoLTU9DS19TRVJWRVIQAhILCgdNQVRDSEVSEAMSDwoLSU5URVJBQ1RJT04QBCJSChJJbml0UGx1Z2luUmVzcG9uc2USPAoJY2F0YWxvZ3VlGAEgAygLMh4uaW8ucGFjdC5wbHVnaW4uQ2F0YWxvZ3VlRW50cnlSCWNhdGFsb2d1ZSJJCglDYXRhbG9ndWUSPAoJY2F0YWxvZ3VlGAEgAygLMh4uaW8ucGFjdC5wbHVnaW4uQ2F0YWxvZ3VlRW50cnlSCWNhdGFsb2d1ZSLlAQoEQm9keRIgCgtjb250ZW50VHlwZRgBIAEoCVILY29udGVudFR5cGUSNQoHY29udGVudBgCIAEoCzIbLmdvb2dsZS5wcm90b2J1Zi5CeXRlc1ZhbHVlUgdjb250ZW50Ek4KD2NvbnRlbnRUeXBlSGludBgDIAEoDjIkLmlvLnBhY3QucGx1Z2luLkJvZHkuQ29udGVudFR5cGVIaW50Ug9jb250ZW50VHlwZUhpbnQiNAoPQ29udGVudFR5cGVIaW50EgsKB0RFRkFVTFQQABIICgRURVhUEAESCgoGQklOQVJZEAIipQMKFkNvbXBhcmVDb250ZW50c1JlcXVlc3QSMAoIZXhwZWN0ZWQYASABKAsyFC5pby5wYWN0LnBsdWdpbi5Cb2R5UghleHBlY3RlZBIsCgZhY3R1YWwYAiABKAsyFC5pby5wYWN0LnBsdWdpbi5Cb2R5UgZhY3R1YWwSMgoVYWxsb3dfdW5leHBlY3RlZF9rZXlzGAMgASgIUhNhbGxvd1VuZXhwZWN0ZWRLZXlzEkcKBXJ1bGVzGAQgAygLMjEuaW8ucGFjdC5wbHVnaW4uQ29tcGFyZUNvbnRlbnRzUmVxdWVzdC5SdWxlc0VudHJ5UgVydWxlcxJVChNwbHVnaW5Db25maWd1cmF0aW9uGAUgASgLMiMuaW8ucGFjdC5wbHVnaW4uUGx1Z2luQ29uZmlndXJhdGlvblITcGx1Z2luQ29uZmlndXJhdGlvbhpXCgpSdWxlc0VudHJ5EhAKA2tleRgBIAEoCVIDa2V5EjMKBXZhbHVlGAIgASgLMh0uaW8ucGFjdC5wbHVnaW4uTWF0Y2hpbmdSdWxlc1IFdmFsdWU6AjgBIkkKE0NvbnRlbnRUeXBlTWlzbWF0Y2gSGgoIZXhwZWN0ZWQYASABKAlSCGV4cGVjdGVkEhYKBmFjdHVhbBgCIAEoCVIGYWN0dWFsIsMBCg9Db250ZW50TWlzbWF0Y2gSNwoIZXhwZWN0ZWQYASABKAsyGy5nb29nbGUucHJvdG9idWYuQnl0ZXNWYWx1ZVIIZXhwZWN0ZWQSMwoGYWN0dWFsGAIgASgLMhsuZ29vZ2xlLnByb3RvYnVmLkJ5dGVzVmFsdWVSBmFjdHVhbBIaCghtaXNtYXRjaBgDIAEoCVIIbWlzbWF0Y2gSEgoEcGF0aBgEIAEoCVIEcGF0aBISCgRkaWZmGAUgASgJUgRkaWZmIlQKEUNvbnRlbnRNaXNtYXRjaGVzEj8KCm1pc21hdGNoZXMYASADKAsyHy5pby5wYWN0LnBsdWdpbi5Db250ZW50TWlzbWF0Y2hSCm1pc21hdGNoZXMipwIKF0NvbXBhcmVDb250ZW50c1Jlc3BvbnNlEhQKBWVycm9yGAEgASgJUgVlcnJvchJHCgx0eXBlTWlzbWF0Y2gYAiABKAsyIy5pby5wYWN0LnBsdWdpbi5Db250ZW50VHlwZU1pc21hdGNoUgx0eXBlTWlzbWF0Y2gSTgoHcmVzdWx0cxgDIAMoCzI0LmlvLnBhY3QucGx1Z2luLkNvbXBhcmVDb250ZW50c1Jlc3BvbnNlLlJlc3VsdHNFbnRyeVIHcmVzdWx0cxpdCgxSZXN1bHRzRW50cnkSEAoDa2V5GAEgASgJUgNrZXkSNwoFdmFsdWUYAiABKAsyIS5pby5wYWN0LnBsdWdpbi5Db250ZW50TWlzbWF0Y2hlc1IFdmFsdWU6AjgBIoABChtDb25maWd1cmVJbnRlcmFjdGlvblJlcXVlc3QSIAoLY29udGVudFR5cGUYASABKAlSC2NvbnRlbnRUeXBlEj8KDmNvbnRlbnRzQ29uZmlnGAIgASgLMhcuZ29vZ2xlLnByb3RvYnVmLlN0cnVjdFIOY29udGVudHNDb25maWciUwoMTWF0Y2hpbmdSdWxlEhIKBHR5cGUYASABKAlSBHR5cGUSLwoGdmFsdWVzGAIgASgLMhcuZ29vZ2xlLnByb3RvYnVmLlN0cnVjdFIGdmFsdWVzIkEKDU1hdGNoaW5nUnVsZXMSMAoEcnVsZRgBIAMoCzIcLmlvLnBhY3QucGx1Z2luLk1hdGNoaW5nUnVsZVIEcnVsZSJQCglHZW5lcmF0b3ISEgoEdHlwZRgBIAEoCVIEdHlwZRIvCgZ2YWx1ZXMYAiABKAsyFy5nb29nbGUucHJvdG9idWYuU3RydWN0UgZ2YWx1ZXMisQEKE1BsdWdpbkNvbmZpZ3VyYXRpb24SUwoYaW50ZXJhY3Rpb25Db25maWd1cmF0aW9uGAEgASgLMhcuZ29vZ2xlLnByb3RvYnVmLlN0cnVjdFIYaW50ZXJhY3Rpb25Db25maWd1cmF0aW9uEkUKEXBhY3RDb25maWd1cmF0aW9uGAIgASgLMhcuZ29vZ2xlLnByb3RvYnVmLlN0cnVjdFIRcGFjdENvbmZpZ3VyYXRpb24iiAYKE0ludGVyYWN0aW9uUmVzcG9uc2USMAoIY29udGVudHMYASABKAsyFC5pby5wYWN0LnBsdWdpbi5Cb2R5Ughjb250ZW50cxJECgVydWxlcxgCIAMoCzIuLmlvLnBhY3QucGx1Z2luLkludGVyYWN0aW9uUmVzcG9uc2UuUnVsZXNFbnRyeVIFcnVsZXMSUwoKZ2VuZXJhdG9ycxgDIAMoCzIzLmlvLnBhY3QucGx1Z2luLkludGVyYWN0aW9uUmVzcG9uc2UuR2VuZXJhdG9yc0VudHJ5UgpnZW5lcmF0b3JzEkEKD21lc3NhZ2VNZXRhZGF0YRgEIAEoCzIXLmdvb2dsZS5wcm90b2J1Zi5TdHJ1Y3RSD21lc3NhZ2VNZXRhZGF0YRJVChNwbHVnaW5Db25maWd1cmF0aW9uGAUgASgLMiMuaW8ucGFjdC5wbHVnaW4uUGx1Z2luQ29uZmlndXJhdGlvblITcGx1Z2luQ29uZmlndXJhdGlvbhIsChFpbnRlcmFjdGlvbk1hcmt1cBgGIAEoCVIRaW50ZXJhY3Rpb25NYXJrdXASZAoVaW50ZXJhY3Rpb25NYXJrdXBUeXBlGAcgASgOMi4uaW8ucGFjdC5wbHVnaW4uSW50ZXJhY3Rpb25SZXNwb25zZS5NYXJrdXBUeXBlUhVpbnRlcmFjdGlvbk1hcmt1cFR5cGUSGgoIcGFydE5hbWUYCCABKAlSCHBhcnROYW1lGlcKClJ1bGVzRW50cnkSEAoDa2V5GAEgASgJUgNrZXkSMwoFdmFsdWUYAiABKAsyHS5pby5wYWN0LnBsdWdpbi5NYXRjaGluZ1J1bGVzUgV2YWx1ZToCOAEaWAoPR2VuZXJhdG9yc0VudHJ5EhAKA2tleRgBIAEoCVIDa2V5Ei8KBXZhbHVlGAIgASgLMhkuaW8ucGFjdC5wbHVnaW4uR2VuZXJhdG9yUgV2YWx1ZToCOAEiJwoKTWFya3VwVHlwZRIPCgtDT01NT05fTUFSSxAAEggKBEhUTUwQASLSAQocQ29uZmlndXJlSW50ZXJhY3Rpb25SZXNwb25zZRIUCgVlcnJvchgBIAEoCVIFZXJyb3ISRQoLaW50ZXJhY3Rpb24YAiADKAsyIy5pby5wYWN0LnBsdWdpbi5JbnRlcmFjdGlvblJlc3BvbnNlUgtpbnRlcmFjdGlvbhJVChNwbHVnaW5Db25maWd1cmF0aW9uGAMgASgLMiMuaW8ucGFjdC5wbHVnaW4uUGx1Z2luQ29uZmlndXJhdGlvblITcGx1Z2luQ29uZmlndXJhdGlvbiLTAgoWR2VuZXJhdGVDb250ZW50UmVxdWVzdBIwCghjb250ZW50cxgBIAEoCzIULmlvLnBhY3QucGx1Z2luLkJvZHlSCGNvbnRlbnRzElYKCmdlbmVyYXRvcnMYAiADKAsyNi5pby5wYWN0LnBsdWdpbi5HZW5lcmF0ZUNvbnRlbnRSZXF1ZXN0LkdlbmVyYXRvcnNFbnRyeVIKZ2VuZXJhdG9ycxJVChNwbHVnaW5Db25maWd1cmF0aW9uGAMgASgLMiMuaW8ucGFjdC5wbHVnaW4uUGx1Z2luQ29uZmlndXJhdGlvblITcGx1Z2luQ29uZmlndXJhdGlvbhpYCg9HZW5lcmF0b3JzRW50cnkSEAoDa2V5GAEgASgJUgNrZXkSLwoFdmFsdWUYAiABKAsyGS5pby5wYWN0LnBsdWdpbi5HZW5lcmF0b3JSBXZhbHVlOgI4ASJLChdHZW5lcmF0ZUNvbnRlbnRSZXNwb25zZRIwCghjb250ZW50cxgBIAEoCzIULmlvLnBhY3QucGx1Z2luLkJvZHlSCGNvbnRlbnRzMuIDCgpQYWN0UGx1Z2luElMKCkluaXRQbHVnaW4SIS5pby5wYWN0LnBsdWdpbi5Jbml0UGx1Z2luUmVxdWVzdBoiLmlvLnBhY3QucGx1Z2luLkluaXRQbHVnaW5SZXNwb25zZRJECg9VcGRhdGVDYXRhbG9ndWUSGS5pby5wYWN0LnBsdWdpbi5DYXRhbG9ndWUaFi5nb29nbGUucHJvdG9idWYuRW1wdHkSYgoPQ29tcGFyZUNvbnRlbnRzEiYuaW8ucGFjdC5wbHVnaW4uQ29tcGFyZUNvbnRlbnRzUmVxdWVzdBonLmlvLnBhY3QucGx1Z2luLkNvbXBhcmVDb250ZW50c1Jlc3BvbnNlEnEKFENvbmZpZ3VyZUludGVyYWN0aW9uEisuaW8ucGFjdC5wbHVnaW4uQ29uZmlndXJlSW50ZXJhY3Rpb25SZXF1ZXN0GiwuaW8ucGFjdC5wbHVnaW4uQ29uZmlndXJlSW50ZXJhY3Rpb25SZXNwb25zZRJiCg9HZW5lcmF0ZUNvbnRlbnQSJi5pby5wYWN0LnBsdWdpbi5HZW5lcmF0ZUNvbnRlbnRSZXF1ZXN0GicuaW8ucGFjdC5wbHVnaW4uR2VuZXJhdGVDb250ZW50UmVzcG9uc2VCEFoOaW8ucGFjdC5wbHVnaW5iBnByb3RvMw==").unwrap();
    // let bytes1 = Bytes::copy_from_slice(bytes.as_slice());
    // let fds = FileDescriptorSet::decode(bytes1);
    // dbg!(fds);

    let body = Body {
      content_type: "application/json".to_string(),
      content: Some("{\"test\": true}".as_bytes().to_vec()),
      content_type_hint: ContentTypeHint::Text as i32
    };
    let encoded = body.encode_to_vec();


    //                             FieldDescriptorProto {
    //                                 name: Some(
    //                                     "content",
    //                                 ),
    //                                 number: Some(
    //                                     2,
    //                                 ),
    //                                 label: Some(
    //                                     Optional,
    //                                 ),
    //                                 r#type: Some(
    //                                     Message,
    //                                 ),
    //                                 type_name: Some(
    //                                     ".google.protobuf.BytesValue",
    //                                 ),
    //                                 extendee: None,
    //                                 default_value: None,
    //                                 oneof_index: None,
    //                                 json_name: Some(
    //                                     "content",
    //                                 ),
    //                                 options: None,
    //                                 proto3_optional: None,
    //                             },

    let field1 = FieldDescriptorProto {
      name: Some("contentType".to_string()),
      number: Some(1),
      label: None,
      r#type: Some(field_descriptor_proto::Type::String as i32),
      type_name: Some("String".to_string()),
      extendee: None,
      default_value: None,
      oneof_index: None,
      json_name: None,
      options: None,
      proto3_optional: None
    };
    let field2 = FieldDescriptorProto {
      name: Some("content".to_string()),
      number: Some(2),
      label: None,
      r#type: Some(field_descriptor_proto::Type::Bytes as i32),
      type_name: Some("bytes".to_string()),
      extendee: None,
      default_value: None,
      oneof_index: None,
      json_name: None,
      options: None,
      proto3_optional: None
    };
    let field3 = FieldDescriptorProto {
      name: Some("contentTypeHint".to_string()),
      number: Some(3),
      label: None,
      r#type: Some(field_descriptor_proto::Type::Enum as i32),
      type_name: Some(".io.pact.plugin.Body.ContentTypeHint".to_string()),
      extendee: None,
      default_value: None,
      oneof_index: None,
      json_name: None,
      options: None,
      proto3_optional: None
    };
    let descriptor = DescriptorProto {
      name: Some("Body".to_string()),
      field: vec![
        field1.clone(),
        field2.clone(),
        field3.clone()
      ],
      extension: vec![],
      nested_type: vec![],
      enum_type: vec![
        EnumDescriptorProto {
          name: Some("ContentTypeHint".to_string()),
          value: vec![
              EnumValueDescriptorProto {
                  name: Some("DEFAULT".to_string()),
                  number: Some(0),
                  options: None
              },
              EnumValueDescriptorProto {
                  name: Some("TEXT".to_string()),
                  number: Some(1),
                  options: None
              },
              EnumValueDescriptorProto {
                  name: Some("BINARY".to_string()),
                  number: Some(2),
                  options: None
              }
          ],
          options: None,
          reserved_range: vec![],
          reserved_name: vec![]
        },
      ],
      extension_range: vec![],
      oneof_decl: vec![],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![]
    };
    let mut message = MessageBuilder::new(&descriptor, "Body");
    message.set_field(&field1, "contentType", MessageFieldValue {
      name: "contentType".to_string(),
      raw_value: Some("application/json".to_string()),
      rtype: RType::String("application/json".to_string()),
      proto_type: Type::String
    });
    message.set_field(&field2, "content", MessageFieldValue {
      name: "content".to_string(),
      raw_value: Some("{\"test\": true}".to_string()),
      rtype: RType::Bytes("{\"test\": true}".as_bytes().to_vec()),
      proto_type: Type::Bytes
    });
    message.set_field(&field3, "contentTypeHint", MessageFieldValue {
      name: "contentTypeHint".to_string(),
      raw_value: Some("TEXT".to_string()),
      rtype: RType::Enum("TEXT".to_string()),
      proto_type: Type::Enum
    });

    let result = message.encode_message().unwrap();
    expect!(result.to_vec()).to(be_equal_to(encoded));

    //<[10, 16, 97, 112, 112, 108, 105, 99, 97, 116, 105, 111, 110, 47, 106, 115, 111, 110, 18, 16, 10, 14, 123, 34, 116, 101, 115, 116, 34, 58, 32, 116, 114, 117, 101, 125, 24, 1]>
    //<[10, 16, 97, 112, 112, 108, 105, 99, 97, 116, 105, 111, 110, 47, 106, 115, 111, 110, 18,         14, 123, 34, 116, 101, 115, 116, 34, 58, 32, 116, 114, 117, 101, 125, 16, 1]>`, src/message_builder.rs:499:5
  }
}
