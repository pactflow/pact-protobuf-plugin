//! Builder for creating protobuf messages based on a descriptor

use std::collections::HashMap;

use maplit::hashmap;
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
}

/// Type of message field
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MessageFieldType {
  /// Represents an absent field
  Absent,
  /// Singular field
  Singular
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
  pub value: Option<ProtoValue>,
  /// Type of the field in the message
  pub field_type: MessageFieldType,
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
      }),
      field_type: MessageFieldType::Singular
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
      }),
      field_type: MessageFieldType::Singular
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
      }),
      field_type: MessageFieldType::Singular
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
      }),
      field_type: MessageFieldType::Singular
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
      }),
      field_type: MessageFieldType::Singular
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
      }),
      field_type: MessageFieldType::Singular
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
      }),
      field_type: MessageFieldType::Singular
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
      }),
      field_type: MessageFieldType::Singular
    })
  }

  /// Create a field value that represents NULL
  pub fn null(name: &str) -> MessageFieldValue {
    MessageFieldValue {
      name: name.to_string(),
      value: None,
      field_type: MessageFieldType::Absent
    }
  }
}
