//! Builder for creating protobuf messages based on a descriptor

use std::collections::HashMap;

use anyhow::anyhow;
use bytes::{Bytes, BytesMut};
use itertools::Itertools;
use maplit::hashmap;
use prost::encoding::{encode_key, string, WireType};
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
            if let Some(val) = &value.value {
              match &val.proto_type {
                Type::Double => {}
                Type::Float => {}
                Type::Int64 => {}
                Type::Uint64 => {}
                Type::Int32 => {}
                Type::Fixed64 => {}
                Type::Fixed32 => {}
                Type::Bool => {}
                Type::String => if let RType::String(s) = &val.rtype {
                  string::encode(tag as u32, s, &mut buffer);
                } else {
                  return Err(anyhow!("Mismatched types, expected a string but got {:?}", val.rtype));
                }
                Type::Group => {}
                Type::Message => {}
                Type::Bytes => {}
                Type::Uint32 => {}
                Type::Enum => {}
                Type::Sfixed32 => {}
                Type::Sfixed64 => {}
                Type::Sint32 => {}
                Type::Sint64 => {}
              }
            }
          }
        }
        MessageFieldValueType::Map => {}
        MessageFieldValueType::Repeated => {}
      }
    }

    Ok(buffer.freeze())
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
  Double(f64)
}

/// Value for the protobuf field
#[derive(Clone, Debug, PartialEq)]
pub struct ProtoValue {
  /// Raw value
  pub value: String,
  /// Rust type for the value
  pub rtype: RType,
  /// Protobuf type for the value
  pub proto_type: Type
}

/// Value of a message field
#[derive(Clone, Debug, PartialEq)]
pub struct MessageFieldValue {
  /// Name of the field
  pub name: String,
  /// Field value
  pub value: Option<ProtoValue>
}

impl MessageFieldValue {
  /// Create a String value
  pub fn string(field_name: &str, field_value: &str) -> MessageFieldValue {
    MessageFieldValue {
      name: field_name.to_string(),
      value: Some(ProtoValue {
        value: field_value.to_string(),
        rtype: RType::String(field_value.to_string()),
        proto_type: Type::String
      })
    }
  }

  /// Create a boolean value. This will fail with an error if the value is not a valid boolean value.
  pub fn boolean(field_name: &str, field_value: &str) -> anyhow::Result<MessageFieldValue> {
    let v: bool = field_value.parse()?;
    Ok(MessageFieldValue {
      name: field_name.to_string(),
      value: Some(ProtoValue {
        value: field_value.to_string(),
        rtype: RType::Boolean(v),
        proto_type: Type::Bool
      })
    })
  }

  /// Create an unsigned 32 bit integer value. This will fail with an error if the value is not a valid integer value.
  pub fn uinteger_32(field_name: &str, field_value: &str, proto_type: Type) -> anyhow::Result<MessageFieldValue> {
    let v: u32 = field_value.parse()?;
    Ok(MessageFieldValue {
      name: field_name.to_string(),
      value: Some(ProtoValue {
        value: field_value.to_string(),
        rtype: RType::UInteger32(v),
        proto_type
      })
    })
  }

  /// Create a signed 32 bit integer value. This will fail with an error if the value is not a valid integer value.
  pub fn integer_32(field_name: &str, field_value: &str, proto_type: Type) -> anyhow::Result<MessageFieldValue> {
    let v: i32 = field_value.parse()?;
    Ok(MessageFieldValue {
      name: field_name.to_string(),
      value: Some(ProtoValue {
        value: field_value.to_string(),
        rtype: RType::Integer32(v),
        proto_type
      })
    })
  }

  /// Create an unsigned 64 bit integer value. This will fail with an error if the value is not a valid integer value.
  pub fn uinteger_64(field_name: &str, field_value: &str, proto_type: Type) -> anyhow::Result<MessageFieldValue> {
    let v: u64 = field_value.parse()?;
    Ok(MessageFieldValue {
      name: field_name.to_string(),
      value: Some(ProtoValue {
        value: field_value.to_string(),
        rtype: RType::UInteger64(v),
        proto_type
      })
    })
  }

  /// Create a signed 64 bit integer value. This will fail with an error if the value is not a valid integer value.
  pub fn integer_64(field_name: &str, field_value: &str, proto_type: Type) -> anyhow::Result<MessageFieldValue> {
    let v: i64 = field_value.parse()?;
    Ok(MessageFieldValue {
      name: field_name.to_string(),
      value: Some(ProtoValue {
        value: field_value.to_string(),
        rtype: RType::Integer64(v),
        proto_type
      })
    })
  }

  /// Create an 32 bit floating point value. This will fail with an error if the value is not a valid float value.
  pub fn float(field_name: &str, field_value: &str, proto_type: Type) -> anyhow::Result<MessageFieldValue> {
    let v: f32 = field_value.parse()?;
    Ok(MessageFieldValue {
      name: field_name.to_string(),
      value: Some(ProtoValue {
        value: field_value.to_string(),
        rtype: RType::Float(v),
        proto_type
      })
    })
  }

  /// Create an 64 bit floating point value. This will fail with an error if the value is not a valid float value.
  pub fn double(field_name: &str, field_value: &str, proto_type: Type) -> anyhow::Result<MessageFieldValue> {
    let v: f64 = field_value.parse()?;
    Ok(MessageFieldValue {
      name: field_name.to_string(),
      value: Some(ProtoValue {
        value: field_value.to_string(),
        rtype: RType::Double(v),
        proto_type
      })
    })
  }

  /// Create a field value that represents NULL
  pub fn null(name: &str) -> MessageFieldValue {
    MessageFieldValue {
      name: name.to_string(),
      value: None
    }
  }
}

#[cfg(test)]
mod tests {
  use expectest::prelude::*;
  use prost_types::{DescriptorProto, field_descriptor_proto, FieldDescriptorProto};
  use prost_types::field_descriptor_proto::Type;

  use crate::message_builder::{MessageBuilder, MessageFieldValue, ProtoValue, RType};

  #[test]
  fn encode_message_test() {
    let field1 = FieldDescriptorProto {
      name: Some("implementation".to_string()),
      number: Some(1),
      label: None,
      r#type: Some(field_descriptor_proto::Type::String as i32),
      type_name: Some("string".to_string()),
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
      type_name: Some("string".to_string()),
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
      value: Some(ProtoValue {
        value: "plugin-driver-rust".to_string(),
        rtype: RType::String("plugin-driver-rust".to_string()),
        proto_type: Type::String
      })
    });
    message.set_field(&field2, "version", MessageFieldValue {
      name: "version".to_string(),
      value: Some(ProtoValue {
        value: "0.0.0".to_string(),
        rtype: RType::String("0.0.0".to_string()),
        proto_type: Type::String
      })
    });

    let result = message.encode_message().unwrap();
    expect!(result.to_vec()).to(be_equal_to(base64::decode("ChJwbHVnaW4tZHJpdmVyLXJ1c3QSBTAuMC4w").unwrap()));
  }
}
