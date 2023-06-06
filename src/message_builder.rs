//! Builder for creating protobuf messages based on a descriptor

use std::collections::btree_map::Entry;
use std::collections::BTreeMap;

use anyhow::anyhow;
use bytes::{BufMut, Bytes, BytesMut};
use itertools::Itertools;
use maplit::btreemap;
use prost::encoding::{encode_key, encode_varint, string, WireType};
use prost::Message;
use prost_types::{DescriptorProto, EnumDescriptorProto, FieldDescriptorProto, FileDescriptorProto};
use prost_types::field_descriptor_proto::Type;
use tracing::{trace, warn};

use crate::utils::{last_name, should_be_packed_type};

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
pub(crate) struct FieldValueInner {
  /// Values for the field, only repeated and map fields will have more than one value.
  pub(crate) values: Vec<MessageFieldValue>,
  /// Descriptor for the field.
  descriptor: FieldDescriptorProto,
  /// Type of field (singular, map or repeated)
  pub(crate) field_type: MessageFieldValueType,
  /// Field data type
  pub(crate) proto_type: Type
}

/// Builder struct for a Protobuf message
#[derive(Clone, Debug, PartialEq)]
pub struct MessageBuilder {
  /// Protobuf descriptor for the file the message belongs to
  pub file_descriptor: FileDescriptorProto,
  /// Protobuf descriptor for the message
  pub descriptor: DescriptorProto,
  /// Message name
  pub message_name: String,
  pub(crate) fields: BTreeMap<String, FieldValueInner>,
}

impl MessageBuilder {
  /// Create a new message builder for the message
  pub fn new(descriptor: &DescriptorProto, message_name: &str, file_descriptor: &FileDescriptorProto) -> Self {
    MessageBuilder {
      file_descriptor: file_descriptor.clone(),
      descriptor: descriptor.clone(),
      message_name: message_name.to_string(),
      fields: btreemap!{}
    }
  }

  /// Find the field descriptor for the given name
  pub fn field_by_name(&self, name: &str) -> Option<FieldDescriptorProto> {
    self.descriptor.field.iter()
      .find(|f| f.name.clone().unwrap_or_default() == name)
      .cloned()
  }

  /// Set the field to the given value
  pub fn set_field_value(&mut self, field_descriptor: &FieldDescriptorProto, field_name: &str, field_value: MessageFieldValue) -> &mut Self {
    self.fields.insert(field_name.to_string(), FieldValueInner {
      values: vec![ field_value ],
      descriptor: field_descriptor.clone(),
      field_type: MessageFieldValueType::Normal,
      proto_type: field_descriptor.r#type()
    });
    self
  }

  /// Adds a value to a repeated field. If the field is not defined, configures it first.
  pub fn add_repeated_field_value(&mut self, field_descriptor: &FieldDescriptorProto, field_name: &str, field_value: MessageFieldValue) -> &mut Self {
    match self.fields.entry(field_name.to_string()) {
      Entry::Occupied(mut e) => {
        let value = e.get_mut();
        value.field_type = MessageFieldValueType::Repeated;
        value.values.push(field_value);
      },
      Entry::Vacant(e) => {
        e.insert(FieldValueInner {
          values: vec![ field_value ],
          descriptor: field_descriptor.clone(),
          field_type: MessageFieldValueType::Repeated,
          proto_type: field_descriptor.r#type()
        });
      }
    };
    self
  }

  /// Adds a map field value, which contains a key and value
  pub fn add_map_field_value(&mut self, field_descriptor: &FieldDescriptorProto, field_name: &str, key: MessageFieldValue, value: MessageFieldValue) -> &mut Self {
    match self.fields.entry(field_name.to_string()) {
      Entry::Occupied(mut e) => {
        e.get_mut().values.push(key);
        e.get_mut().values.push(value);
      },
      Entry::Vacant(e) => {
        e.insert(FieldValueInner {
          values: vec![ key, value ],
          descriptor: field_descriptor.clone(),
          field_type: MessageFieldValueType::Map,
          proto_type: field_descriptor.r#type()
        });
      }
    };
    self
  }

  /// Encodes the Protobuf message into a bytes buffer
  pub fn encode_message(&self) -> anyhow::Result<Bytes> {
    trace!(">> encode_message {}, {} fields", self.message_name, self.fields.len());
    let mut buffer = BytesMut::with_capacity(1024);

    for (_, field_data) in self.fields.iter()
      .sorted_by(|(_, a), (_, b)| Ord::cmp(&a.descriptor.number.unwrap_or_default(), &b.descriptor.number.unwrap_or_default())) {
      match field_data.field_type {
        MessageFieldValueType::Normal => self.encode_single_field(&mut buffer, field_data, field_data.values.first().cloned())?,
        MessageFieldValueType::Map => self.encode_map_field(&mut buffer, field_data)?,
        MessageFieldValueType::Repeated => self.encode_repeated_field(&mut buffer, field_data)?
      }
    }

    trace!("encode_message: {} bytes", buffer.len());

    Ok(buffer.freeze())
  }

  fn encode_single_field(&self, mut buffer: &mut BytesMut, field_data: &FieldValueInner, value: Option<MessageFieldValue>) -> anyhow::Result<()> {
    trace!(">> encode_single_field({:?}, {:?}, {:?})", value, field_data.descriptor.number, field_data);
    if let Some(value) = value {
      if let Some(tag) = field_data.descriptor.number {
        match field_data.proto_type {
          Type::Double => prost::encoding::double::encode(tag as u32, &value.rtype.as_f64()?, &mut buffer),
          Type::Float => prost::encoding::float::encode(tag as u32, &value.rtype.as_f32()?, &mut buffer),
          Type::Int64 => prost::encoding::int64::encode(tag as u32, &value.rtype.as_i64()?, &mut buffer),
          Type::Uint64 => prost::encoding::uint64::encode(tag as u32, &value.rtype.as_u64()?, &mut buffer),
          Type::Int32 => prost::encoding::int32::encode(tag as u32, &value.rtype.as_i32()?, &mut buffer),
          Type::Uint32 => prost::encoding::uint32::encode(tag as u32, &value.rtype.as_u32()?, &mut buffer),
          Type::Fixed64 => prost::encoding::fixed64::encode(tag as u32, &value.rtype.as_u64()?, &mut buffer),
          Type::Fixed32 => prost::encoding::fixed32::encode(tag as u32, &value.rtype.as_u32()?, &mut buffer),
          Type::Bool => prost::encoding::bool::encode(tag as u32, &value.rtype.as_bool()?, &mut buffer),
          Type::String => string::encode(tag as u32, &value.rtype.as_str()?, &mut buffer),
          Type::Message => match &value.rtype {
            RType::Message(m) => {
              let message_bytes = m.encode_message()?;
              encode_key(tag as u32, WireType::LengthDelimited, &mut buffer);
              encode_varint(message_bytes.len() as u64, &mut buffer);
              buffer.put_slice(&message_bytes);
            }
            RType::Struct(s) => {
              trace!("Encoding a Protobuf Struct");
              let mut buffer2 = BytesMut::with_capacity(s.encoded_len());
              s.encode(&mut buffer2)?;
              encode_key(tag as u32, WireType::LengthDelimited, &mut buffer);
              encode_varint(buffer2.len() as u64, &mut buffer);
              buffer.put_slice(&buffer2);
            }
            RType::Bytes(b) => {
              // Encode a google.protobuf.BytesValue
              trace!("Encoding a Protobuf BytesValue");
              let data = Bytes::from(b.clone());

              // BytesValue field 1 is a byte array
              let mut buffer2 = BytesMut::new();
              encode_key(1_u32, WireType::LengthDelimited, &mut buffer2);
              encode_varint(data.len() as u64, &mut buffer2);
              buffer2.put_slice(&data);

              // encode the BytesValue
              encode_key(tag as u32, WireType::LengthDelimited, &mut buffer);
              encode_varint(buffer2.len() as u64, &mut buffer);
              buffer.put_slice(&buffer2);
            }
            _ => {
              return Err(anyhow!("Mismatched types, expected a message builder but got {:?}", value.rtype));
            }
          }
          Type::Bytes => if let RType::Bytes(b) = &value.rtype {
            prost::encoding::bytes::encode(tag as u32, b, &mut buffer);
          } else {
            return Err(anyhow!("Mismatched types, expected a byte array but got {:?}", value.rtype));
          }
          Type::Enum => if let RType::Enum(n, desc) = &value.rtype {
            self.encode_enum_value(&field_data.descriptor, &value, tag, n, desc, buffer)?;
          } else if let RType::Integer32(i) = &value.rtype {
            encode_key(tag as u32, WireType::Varint, buffer);
            encode_varint(*i as u64, buffer);
          } else {
            return Err(anyhow!("Mismatched types, expected an enum but got {:?}", value.rtype));
          }
          Type::Sfixed32 => prost::encoding::sfixed32::encode(tag as u32, &value.rtype.as_i32()?, &mut buffer),
          Type::Sfixed64 => prost::encoding::sfixed64::encode(tag as u32, &value.rtype.as_i64()?, &mut buffer),
          Type::Sint32 => prost::encoding::sint32::encode(tag as u32, &value.rtype.as_i32()?, &mut buffer),
          Type::Sint64 => prost::encoding::sint64::encode(tag as u32, &value.rtype.as_i64()?, &mut buffer),
          _ => return Err(anyhow!("Protobuf type {:?} is not supported", field_data.proto_type))
        }
      }
    }
    Ok(())
  }

  fn encode_enum_value(
    &self,
    descriptor: &FieldDescriptorProto,
    field_value: &MessageFieldValue,
    tag: i32,
    enum_value: &i32,
    enum_proto: &EnumDescriptorProto,
    buffer: &mut BytesMut
  ) -> anyhow::Result<()> {
    trace!(">> encode_enum_value({:?}, {}, {})", field_value, tag, enum_value);
    let enum_type_name = descriptor.type_name.as_ref().ok_or_else(|| anyhow!("Type name is missing from the descriptor for enum field {}", field_value.name))?;
    let enum_value = enum_proto.value.iter().find(|enum_val| enum_val.number == Some(*enum_value))
      .ok_or_else(|| anyhow!("Did not find the enum value {} for the enum {} in the Protobuf descriptor", enum_value, enum_type_name))?;
    if let Some(enum_value_number) = enum_value.number {
      encode_key(tag as u32, WireType::Varint, buffer);
      encode_varint(enum_value_number as u64, buffer);
    } else {
      warn!("Enum value {:?} for enum {} does not have a numeric value set, will use the default", enum_value, enum_type_name);
    }
    Ok(())
  }

  fn encode_map_field(&self, buffer: &mut BytesMut, field_value: &FieldValueInner) -> anyhow::Result<()> {
    trace!(">> encode_map_field({:?})", field_value);
    if !field_value.values.is_empty() {
      if field_value.values.len() % 2 == 1 {
        return Err(anyhow!("Map fields need to have an even number of field values as key-value pairs, got {} field values", field_value.values.len()));
      }
      let entry_type_name = field_value.descriptor.type_name.as_ref().ok_or_else(|| anyhow!("Type name is missing from the descriptor for map field"))?;
      let entry_name = last_name(entry_type_name.as_str());
      let entry_proto = self.descriptor.nested_type.iter().find(|nested_type| nested_type.name.clone().unwrap_or_default() == entry_name)
        .ok_or_else(|| anyhow!("Did not find the nested type {} for the map field {} in the Protobuf descriptor", entry_name, entry_type_name))?;

      let key_proto = entry_proto.field.iter().find(|f| f.name.clone().unwrap_or_default() == "key")
        .ok_or_else(|| anyhow!("Did not find the field descriptor for the key for the map field {} in the Protobuf descriptor", entry_type_name))?;
      let value_proto = entry_proto.field.iter().find(|f| f.name.clone().unwrap_or_default() == "value")
        .ok_or_else(|| anyhow!("Did not find the field descriptor for the value for the map field {} in the Protobuf descriptor", entry_type_name))?;

      let entries = field_value.values.iter().tuples::<(_, _)>()
        .map(|(k, v)| {
          MessageFieldValue {
            name: entry_name.to_string(),
            raw_value: None,
            rtype: RType::Message(Box::new(MessageBuilder {
              file_descriptor: self.file_descriptor.clone(),
              descriptor: entry_proto.clone(),
              message_name: entry_name.to_string(),
              fields: btreemap! {
                "key".to_string() => FieldValueInner {
                  values: vec![ k.clone() ],
                  descriptor: key_proto.clone(),
                  field_type: MessageFieldValueType::Normal,
                  proto_type: key_proto.r#type()
                },
                "value".to_string() => FieldValueInner {
                  values: vec![ v.clone() ],
                  descriptor: value_proto.clone(),
                  field_type: MessageFieldValueType::Normal,
                  proto_type: value_proto.r#type()
                }
              }
            }))
          }
        }).collect();

      self.encode_repeated_field(buffer, &FieldValueInner {
        values: entries,
        descriptor: field_value.descriptor.clone(),
        field_type: MessageFieldValueType::Repeated,
        proto_type: field_value.proto_type
      })
    } else {
      Ok(())
    }
  }

  fn encode_repeated_field(&self, buffer: &mut BytesMut, field_value: &FieldValueInner) -> anyhow::Result<()> {
    trace!(">> encode_repeated_field({:?})", field_value);
    if !field_value.values.is_empty() {
      if should_be_packed_type(field_value.proto_type) {
        self.encode_packed_field(buffer, field_value)?;
      } else {
        for value in &field_value.values {
          self.encode_single_field(buffer, field_value, Some(value.clone()))?;
        }
      }
    }
    Ok(())
  }

  /// Generate a markdown representation of the message
  pub fn generate_markup(&self, indent: &str) -> anyhow::Result<String> {
    let mut buffer = String::new();

    buffer.push_str(format!("```protobuf\n{}message {} {{\n", indent, self.message_name).as_str());

    for (name, inner) in self.fields.iter()
      .sorted_by(|(_, a), (_, b)| Ord::cmp(&a.descriptor.number, &b.descriptor.number)) {
      if let Some(field_num) = inner.descriptor.number {
        match inner.field_type {
          MessageFieldValueType::Normal => buffer.push_str(format!("{}    {} {} = {};\n", indent, field_type_name(inner)?, name, field_num).as_str()),
          MessageFieldValueType::Map => buffer.push_str(format!("{}    map<{}> {} = {};\n", indent, field_type_name(inner)?, name, field_num).as_str()),
          MessageFieldValueType::Repeated => buffer.push_str(format!("{}    repeated {} {} = {};\n", indent, field_type_name(inner)?, name, field_num).as_str()),
        }
      }
    }

    buffer.push_str(format!("{}}}\n```\n", indent).as_str());

    Ok(buffer)
  }

  fn encode_packed_field(
    &self,
    buffer: &mut BytesMut,
    field_value: &FieldValueInner
  ) -> anyhow::Result<()> {
    if let Some(tag) = field_value.descriptor.number {
      match field_value.proto_type {
        Type::Double => {
          let values = field_value.values.iter()
            .map(|v| v.rtype.as_f64().unwrap_or_default())
              .collect::<Vec<f64>>();
          prost::encoding::double::encode_packed(tag as u32, &values, buffer);
          Ok(())
        }
        Type::Float => {
          let values = field_value.values.iter()
              .map(|v| v.rtype.as_f32().unwrap_or_default())
              .collect::<Vec<f32>>();
          prost::encoding::float::encode_packed(tag as u32, &values, buffer);
          Ok(())
        }
        Type::Int64 => {
          let values = field_value.values.iter()
              .map(|v| v.rtype.as_i64().unwrap_or_default())
              .collect::<Vec<i64>>();
          prost::encoding::int64::encode_packed(tag as u32, &values, buffer);
          Ok(())
        }
        Type::Uint64 => {
          let values = field_value.values.iter()
              .map(|v| v.rtype.as_u64().unwrap_or_default())
              .collect::<Vec<u64>>();
          prost::encoding::uint64::encode_packed(tag as u32, &values, buffer);
          Ok(())
        }
        Type::Int32 => {
          let values = field_value.values.iter()
              .map(|v| v.rtype.as_i32().unwrap_or_default())
              .collect::<Vec<i32>>();
          prost::encoding::int32::encode_packed(tag as u32, &values, buffer);
          Ok(())
        }
        Type::Fixed64 => {
          let values = field_value.values.iter()
              .map(|v| v.rtype.as_u64().unwrap_or_default())
              .collect::<Vec<u64>>();
          prost::encoding::fixed64::encode_packed(tag as u32, &values, buffer);
          Ok(())
        }
        Type::Fixed32 => {
          let values = field_value.values.iter()
              .map(|v| v.rtype.as_u32().unwrap_or_default())
              .collect::<Vec<u32>>();
          prost::encoding::fixed32::encode_packed(tag as u32, &values, buffer);
          Ok(())
        }
        Type::Uint32 => {
          let values = field_value.values.iter()
              .map(|v| v.rtype.as_u32().unwrap_or_default())
              .collect::<Vec<u32>>();
          prost::encoding::uint32::encode_packed(tag as u32, &values, buffer);
          Ok(())
        }
        Type::Sfixed32 => {
          let values = field_value.values.iter()
              .map(|v| v.rtype.as_i32().unwrap_or_default())
              .collect::<Vec<i32>>();
          prost::encoding::sfixed32::encode_packed(tag as u32, &values, buffer);
          Ok(())
        }
        Type::Sfixed64 => {
          let values = field_value.values.iter()
              .map(|v| v.rtype.as_i64().unwrap_or_default())
              .collect::<Vec<i64>>();
          prost::encoding::sfixed64::encode_packed(tag as u32, &values, buffer);
          Ok(())
        }
        Type::Sint32 => {
          let values = field_value.values.iter()
              .map(|v| v.rtype.as_i32().unwrap_or_default())
              .collect::<Vec<i32>>();
          prost::encoding::sint32::encode_packed(tag as u32, &values, buffer);
          Ok(())
        }
        Type::Sint64 => {
          let values = field_value.values.iter()
              .map(|v| v.rtype.as_i64().unwrap_or_default())
              .collect::<Vec<i64>>();
          prost::encoding::sint64::encode_packed(tag as u32, &values, buffer);
          Ok(())
        }
        _ => Err(anyhow!("Can not encode a {:?} field in packaged form", field_value.proto_type))
      }
    } else {
      Err(anyhow!("Unable to encode field {:?} as it has no tag", field_value.descriptor.name))
    }
  }
}

fn field_type_name(field: &FieldValueInner) -> anyhow::Result<String> {
  Ok(match field.proto_type {
    Type::Double => "double".to_string(),
    Type::Float => "float".to_string(),
    Type::Int64 => "int64".to_string(),
    Type::Uint64 => "uint64".to_string(),
    Type::Int32 => "int32".to_string(),
    Type::Fixed64 => "fixed64".to_string(),
    Type::Fixed32 => "fixed32".to_string(),
    Type::Bool => "bool".to_string(),
    Type::String => "string".to_string(),
    Type::Group => "group".to_string(),
    Type::Message => {
      let message = field.descriptor.type_name.as_ref()
        .ok_or_else(|| anyhow!("Type name is missing from the descriptor for message field"))?;
      format!("message {}", message)
    },
    Type::Bytes => "bytes".to_string(),
    Type::Uint32 => "uint32".to_string(),
    Type::Enum => {
      let enum_type_name = field.descriptor.type_name.as_ref()
        .ok_or_else(|| anyhow!("Type name is missing from the descriptor for enum field"))?;
      format!("enum {}", enum_type_name)
    }
    Type::Sfixed32 => "sfixed32".to_string(),
    Type::Sfixed64 => "sfixed64".to_string(),
    Type::Sint32 => "sint32".to_string(),
    Type::Sint64 => "sint64".to_string()
  })
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
  Enum(i32, EnumDescriptorProto),
  /// Embedded message
  Message(Box<MessageBuilder>),
  /// Embedded google.protobuf.Struct
  Struct(prost_types::Struct)
}

impl RType {
  /// Convert this value to a double
  pub fn as_f64(&self) -> anyhow::Result<f64> {
    match self {
      RType::String(s) => s.parse::<f64>().map_err(|err| anyhow!(err)),
      RType::Boolean(b) => Ok(*b as u8 as f64),
      RType::UInteger32(u) => Ok(*u as f64),
      RType::Integer32(i) => Ok(*i as f64),
      RType::UInteger64(u) => Ok(*u as f64),
      RType::Integer64(i) => Ok(*i as f64),
      RType::Float(f) => Ok(*f as f64),
      RType::Double(d) => Ok(*d),
      _ => Err(anyhow!("Can't convert {:?} to f64", self))
    }
  }

  /// Convert this value to a float
  pub fn as_f32(&self) -> anyhow::Result<f32> {
    match self {
      RType::String(s) => s.parse::<f32>().map_err(|err| anyhow!(err)),
      RType::Boolean(b) => Ok(*b as u8 as f32),
      RType::UInteger32(u) => Ok(*u as f32),
      RType::Integer32(i) => Ok(*i as f32),
      RType::UInteger64(u) => Ok(*u as f32),
      RType::Integer64(i) => Ok(*i as f32),
      RType::Float(f) => Ok(*f),
      RType::Double(d) => Ok(*d as f32),
      _ => Err(anyhow!("Can't convert {:?} to f64", self))
    }
  }

  /// Convert this value to a u64
  pub fn as_u64(&self) -> anyhow::Result<u64> {
    match self {
      RType::String(s) => s.parse::<u64>().map_err(|err| anyhow!(err)),
      RType::Boolean(b) => Ok(*b as u64),
      RType::UInteger32(u) => Ok(*u as u64),
      RType::Integer32(i) => Ok(*i as u64),
      RType::UInteger64(u) => Ok(*u),
      RType::Integer64(i) => Ok(*i as u64),
      RType::Float(f) => Ok(*f as u64),
      RType::Double(d) => Ok(*d as u64),
      _ => Err(anyhow!("Can't convert {:?} to u64", self))
    }
  }

  /// Convert this value to a i64
  pub fn as_i64(&self) -> anyhow::Result<i64> {
    match self {
      RType::String(s) => s.parse::<i64>().map_err(|err| anyhow!(err)),
      RType::Boolean(b) => Ok(*b as i64),
      RType::UInteger32(u) => Ok(*u as i64),
      RType::Integer32(i) => Ok(*i as i64),
      RType::UInteger64(u) => Ok(*u as i64),
      RType::Integer64(i) => Ok(*i),
      RType::Float(f) => Ok(*f as i64),
      RType::Double(d) => Ok(*d as i64),
      _ => Err(anyhow!("Can't convert {:?} to i64", self))
    }
  }

  /// Convert this value to a u32
  pub fn as_u32(&self) -> anyhow::Result<u32> {
    match self {
      RType::String(s) => s.parse::<u32>().map_err(|err| anyhow!(err)),
      RType::Boolean(b) => Ok(*b as u32),
      RType::UInteger32(u) => Ok(*u),
      RType::Integer32(i) => Ok(*i as u32),
      RType::UInteger64(u) => Ok(*u as u32),
      RType::Integer64(i) => Ok(*i as u32),
      RType::Float(f) => Ok(*f as u32),
      RType::Double(d) => Ok(*d as u32),
      _ => Err(anyhow!("Can't convert {:?} to u32", self))
    }
  }

  /// Convert this value to a i32
  pub fn as_i32(&self) -> anyhow::Result<i32> {
    match self {
      RType::String(s) => s.parse::<i32>().map_err(|err| anyhow!(err)),
      RType::Boolean(b) => Ok(*b as i32),
      RType::UInteger32(u) => Ok(*u as i32),
      RType::Integer32(i) => Ok(*i),
      RType::UInteger64(u) => Ok(*u as i32),
      RType::Integer64(i) => Ok(*i as i32),
      RType::Float(f) => Ok(*f as i32),
      RType::Double(d) => Ok(*d as i32),
      _ => Err(anyhow!("Can't convert {:?} to i32", self))
    }
  }

  /// Convert this value to a string
  pub fn as_str(&self) -> anyhow::Result<String> {
    match self {
      RType::String(s) => Ok(s.clone()),
      RType::Boolean(b) => Ok(b.to_string()),
      RType::UInteger32(u) => Ok(u.to_string()),
      RType::Integer32(i) => Ok(i.to_string()),
      RType::UInteger64(u) => Ok(u.to_string()),
      RType::Integer64(i) => Ok(i.to_string()),
      RType::Float(f) => Ok(f.to_string()),
      RType::Double(d) => Ok(d.to_string()),
      RType::Enum(n, _) => Ok(n.to_string()),
      _ => Err(anyhow!("Can't convert {:?} to a string", self))
    }
  }

  /// Convert this value to a bool
  pub fn as_bool(&self) -> anyhow::Result<bool> {
    match self {
      RType::String(s) => s.parse::<bool>().map_err(|err| anyhow!(err)),
      RType::Boolean(b) => Ok(*b),
      RType::UInteger32(u) => Ok(*u > 0),
      RType::Integer32(i) => Ok(*i > 0),
      RType::UInteger64(u) => Ok(*u > 0),
      RType::Integer64(i) => Ok(*i > 0),
      RType::Float(f) => Ok(*f > 0.0),
      RType::Double(d) => Ok(*d > 0.0),
      _ => Err(anyhow!("Can't convert {:?} to i64", self))
    }
  }
}

/// Value of a message field
#[derive(Clone, Debug, PartialEq)]
pub struct MessageFieldValue {
  /// Name of the field
  pub name: String,
  /// Raw value in text form
  pub raw_value: Option<String>,
  /// Rust type for the value
  pub rtype: RType
}

impl MessageFieldValue {
  /// Create a String value
  pub fn string(field_name: &str, field_value: &str) -> MessageFieldValue {
    MessageFieldValue {
      name: field_name.to_string(),
      raw_value: Some(field_value.to_string()),
      rtype: RType::String(field_value.to_string())
    }
  }

  /// Create a boolean value. This will fail with an error if the value is not a valid boolean value.
  pub fn boolean(field_name: &str, field_value: &str) -> anyhow::Result<MessageFieldValue> {
    let v: bool = field_value.parse()?;
    Ok(MessageFieldValue {
      name: field_name.to_string(),
      raw_value: Some(field_value.to_string()),
      rtype: RType::Boolean(v)
    })
  }

  /// Create an unsigned 32 bit integer value. This will fail with an error if the value is not a valid integer value.
  pub fn uinteger_32(field_name: &str, field_value: &str) -> anyhow::Result<MessageFieldValue> {
    let v: u32 = field_value.parse()?;
    Ok(MessageFieldValue {
      name: field_name.to_string(),
      raw_value: Some(field_value.to_string()),
      rtype: RType::UInteger32(v)
    })
  }

  /// Create a signed 32 bit integer value. This will fail with an error if the value is not a valid integer value.
  pub fn integer_32(field_name: &str, field_value: &str) -> anyhow::Result<MessageFieldValue> {
    let v: i32 = field_value.parse()?;
    Ok(MessageFieldValue {
      name: field_name.to_string(),
      raw_value: Some(field_value.to_string()),
      rtype: RType::Integer32(v)
    })
  }

  /// Create an unsigned 64 bit integer value. This will fail with an error if the value is not a valid integer value.
  pub fn uinteger_64(field_name: &str, field_value: &str) -> anyhow::Result<MessageFieldValue> {
    let v: u64 = field_value.parse()?;
    Ok(MessageFieldValue {
      name: field_name.to_string(),
      raw_value: Some(field_value.to_string()),
      rtype: RType::UInteger64(v)
    })
  }

  /// Create a signed 64 bit integer value. This will fail with an error if the value is not a valid integer value.
  pub fn integer_64(field_name: &str, field_value: &str) -> anyhow::Result<MessageFieldValue> {
    let v: i64 = field_value.parse()?;
    Ok(MessageFieldValue {
      name: field_name.to_string(),
      raw_value: Some(field_value.to_string()),
      rtype: RType::Integer64(v)
    })
  }

  /// Create an 32 bit floating point value. This will fail with an error if the value is not a valid float value.
  pub fn float(field_name: &str, field_value: &str) -> anyhow::Result<MessageFieldValue> {
    let v: f32 = field_value.parse()?;
    Ok(MessageFieldValue {
      name: field_name.to_string(),
      raw_value: Some(field_value.to_string()),
      rtype: RType::Float(v)
    })
  }

  /// Create an 64 bit floating point value. This will fail with an error if the value is not a valid float value.
  pub fn double(field_name: &str, field_value: &str) -> anyhow::Result<MessageFieldValue> {
    let v: f64 = field_value.parse()?;
    Ok(MessageFieldValue {
      name: field_name.to_string(),
      raw_value: Some(field_value.to_string()),
      rtype: RType::Double(v)
    })
  }

  /// Create a byte array value
  pub fn bytes(field_name: &str, field_value: &str) -> MessageFieldValue {
    MessageFieldValue {
      name: field_name.to_string(),
      raw_value: Some(field_value.to_string()),
      rtype: RType::Bytes(field_value.as_bytes().to_vec())
    }
  }
}

#[cfg(test)]
mod tests {
  use base64::Engine;
  use base64::engine::general_purpose::STANDARD as BASE64;
  use bytes::{Bytes, BytesMut};
  use expectest::prelude::*;
  use itertools::Itertools;
  use maplit::{btreemap, hashmap};
  use pact_plugin_driver::proto::{
    Body,
    CompareContentsRequest,
    MatchingRule,
    MatchingRules
  };
  use pact_plugin_driver::proto::body::ContentTypeHint;
  use prost::encoding::WireType;
  use prost::Message;
  use prost_types::{
    DescriptorProto,
    EnumDescriptorProto,
    EnumValueDescriptorProto,
    field_descriptor_proto,
    FieldDescriptorProto,
    FileDescriptorProto,
    FileDescriptorSet,
    MessageOptions,
    OneofDescriptorProto
  };
  use prost_types::field_descriptor_proto::Label::Optional;
  use prost_types::value::Kind;
  use trim_margin::MarginTrimmable;

  use crate::message_builder::{MessageBuilder, MessageFieldValue, RType};
  use crate::message_builder::MessageFieldValueType::Repeated;
  use crate::message_decoder::{decode_message, ProtobufFieldData};
  use crate::protobuf::tests::DESCRIPTOR_WITH_ENUM_BYTES;

  const ENCODED_MESSAGE: &str = "CuIFChxnb29nbGUvcHJvdG9idWYvc3RydWN0LnByb3RvEg9nb29nbGUucHJv\
  dG9idWYimAEKBlN0cnVjdBI7CgZmaWVsZHMYASADKAsyIy5nb29nbGUucHJvdG9idWYuU3RydWN0LkZpZWxkc0VudHJ5\
  UgZmaWVsZHMaUQoLRmllbGRzRW50cnkSEAoDa2V5GAEgASgJUgNrZXkSLAoFdmFsdWUYAiABKAsyFi5nb29nbGUucHJvd\
  G9idWYuVmFsdWVSBXZhbHVlOgI4ASKyAgoFVmFsdWUSOwoKbnVsbF92YWx1ZRgBIAEoDjIaLmdvb2dsZS5wcm90b2J1Zi5\
  OdWxsVmFsdWVIAFIJbnVsbFZhbHVlEiMKDG51bWJlcl92YWx1ZRgCIAEoAUgAUgtudW1iZXJWYWx1ZRIjCgxzdHJpbmdfd\
  mFsdWUYAyABKAlIAFILc3RyaW5nVmFsdWUSHwoKYm9vbF92YWx1ZRgEIAEoCEgAUglib29sVmFsdWUSPAoMc3RydWN0X3Z\
  hbHVlGAUgASgLMhcuZ29vZ2xlLnByb3RvYnVmLlN0cnVjdEgAUgtzdHJ1Y3RWYWx1ZRI7CgpsaXN0X3ZhbHVlGAYgASgLM\
  houZ29vZ2xlLnByb3RvYnVmLkxpc3RWYWx1ZUgAUglsaXN0VmFsdWVCBgoEa2luZCI7CglMaXN0VmFsdWUSLgoGdmFsdWV\
  zGAEgAygLMhYuZ29vZ2xlLnByb3RvYnVmLlZhbHVlUgZ2YWx1ZXMqGwoJTnVsbFZhbHVlEg4KCk5VTExfVkFMVUUQAEJ/C\
  hNjb20uZ29vZ2xlLnByb3RvYnVmQgtTdHJ1Y3RQcm90b1ABWi9nb29nbGUuZ29sYW5nLm9yZy9wcm90b2J1Zi90eXBlcy9r\
  bm93bi9zdHJ1Y3RwYvgBAaICA0dQQqoCHkdvb2dsZS5Qcm90b2J1Zi5XZWxsS25vd25UeXBlc2IGcHJvdG8zCoYECh5nb2\
  9nbGUvcHJvdG9idWYvd3JhcHBlcnMucHJvdG8SD2dvb2dsZS5wcm90b2J1ZiIjCgtEb3VibGVWYWx1ZRIUCgV2YWx1ZRgB\
  IAEoAVIFdmFsdWUiIgoKRmxvYXRWYWx1ZRIUCgV2YWx1ZRgBIAEoAlIFdmFsdWUiIgoKSW50NjRWYWx1ZRIUCgV2YWx1ZR\
  gBIAEoA1IFdmFsdWUiIwoLVUludDY0VmFsdWUSFAoFdmFsdWUYASABKARSBXZhbHVlIiIKCkludDMyVmFsdWUSFAoFdmFs\
  dWUYASABKAVSBXZhbHVlIiMKC1VJbnQzMlZhbHVlEhQKBXZhbHVlGAEgASgNUgV2YWx1ZSIhCglCb29sVmFsdWUSFAoFdm\
  FsdWUYASABKAhSBXZhbHVlIiMKC1N0cmluZ1ZhbHVlEhQKBXZhbHVlGAEgASgJUgV2YWx1ZSIiCgpCeXRlc1ZhbHVlEhQKB\
  XZhbHVlGAEgASgMUgV2YWx1ZUKDAQoTY29tLmdvb2dsZS5wcm90b2J1ZkINV3JhcHBlcnNQcm90b1ABWjFnb29nbGUuZ29\
  sYW5nLm9yZy9wcm90b2J1Zi90eXBlcy9rbm93bi93cmFwcGVyc3Bi+AEBogIDR1BCqgIeR29vZ2xlLlByb3RvYnVmLldlb\
  GxLbm93blR5cGVzYgZwcm90bzMKvgEKG2dvb2dsZS9wcm90b2J1Zi9lbXB0eS5wcm90bxIPZ29vZ2xlLnByb3RvYnVmIg\
  cKBUVtcHR5Qn0KE2NvbS5nb29nbGUucHJvdG9idWZCCkVtcHR5UHJvdG9QAVouZ29vZ2xlLmdvbGFuZy5vcmcvcHJvdG9\
  idWYvdHlwZXMva25vd24vZW1wdHlwYvgBAaICA0dQQqoCHkdvb2dsZS5Qcm90b2J1Zi5XZWxsS25vd25UeXBlc2IGcHJv\
  dG8zCv0iCgxwbHVnaW4ucHJvdG8SDmlvLnBhY3QucGx1Z2luGhxnb29nbGUvcHJvdG9idWYvc3RydWN0LnByb3RvGh5nb2\
  9nbGUvcHJvdG9idWYvd3JhcHBlcnMucHJvdG8aG2dvb2dsZS9wcm90b2J1Zi9lbXB0eS5wcm90byJVChFJbml0UGx1Z2l\
  uUmVxdWVzdBImCg5pbXBsZW1lbnRhdGlvbhgBIAEoCVIOaW1wbGVtZW50YXRpb24SGAoHdmVyc2lvbhgCIAEoCVIHdmVy\
  c2lvbiLHAgoOQ2F0YWxvZ3VlRW50cnkSPAoEdHlwZRgBIAEoDjIoLmlvLnBhY3QucGx1Z2luLkNhdGFsb2d1ZUVudHJ5Lk\
  VudHJ5VHlwZVIEdHlwZRIQCgNrZXkYAiABKAlSA2tleRJCCgZ2YWx1ZXMYAyADKAsyKi5pby5wYWN0LnBsdWdpbi5DYXRh\
  bG9ndWVFbnRyeS5WYWx1ZXNFbnRyeVIGdmFsdWVzGjkKC1ZhbHVlc0VudHJ5EhAKA2tleRgBIAEoCVIDa2V5EhQKBXZhbH\
  VlGAIgASgJUgV2YWx1ZToCOAEiZgoJRW50cnlUeXBlEhMKD0NPTlRFTlRfTUFUQ0hFUhAAEhUKEUNPTlRFTlRfR0VORVJ\
  BVE9SEAESDwoLTU9DS19TRVJWRVIQAhILCgdNQVRDSEVSEAMSDwoLSU5URVJBQ1RJT04QBCJSChJJbml0UGx1Z2luUmVz\
  cG9uc2USPAoJY2F0YWxvZ3VlGAEgAygLMh4uaW8ucGFjdC5wbHVnaW4uQ2F0YWxvZ3VlRW50cnlSCWNhdGFsb2d1ZSJJC\
  glDYXRhbG9ndWUSPAoJY2F0YWxvZ3VlGAEgAygLMh4uaW8ucGFjdC5wbHVnaW4uQ2F0YWxvZ3VlRW50cnlSCWNhdGFsb2\
  d1ZSLlAQoEQm9keRIgCgtjb250ZW50VHlwZRgBIAEoCVILY29udGVudFR5cGUSNQoHY29udGVudBgCIAEoCzIbLmdvb2d\
  sZS5wcm90b2J1Zi5CeXRlc1ZhbHVlUgdjb250ZW50Ek4KD2NvbnRlbnRUeXBlSGludBgDIAEoDjIkLmlvLnBhY3QucGx1Z\
  2luLkJvZHkuQ29udGVudFR5cGVIaW50Ug9jb250ZW50VHlwZUhpbnQiNAoPQ29udGVudFR5cGVIaW50EgsKB0RFRkFVTF\
  QQABIICgRURVhUEAESCgoGQklOQVJZEAIipQMKFkNvbXBhcmVDb250ZW50c1JlcXVlc3QSMAoIZXhwZWN0ZWQYASABKAs\
  yFC5pby5wYWN0LnBsdWdpbi5Cb2R5UghleHBlY3RlZBIsCgZhY3R1YWwYAiABKAsyFC5pby5wYWN0LnBsdWdpbi5Cb2R5\
  UgZhY3R1YWwSMgoVYWxsb3dfdW5leHBlY3RlZF9rZXlzGAMgASgIUhNhbGxvd1VuZXhwZWN0ZWRLZXlzEkcKBXJ1bGVzG\
  AQgAygLMjEuaW8ucGFjdC5wbHVnaW4uQ29tcGFyZUNvbnRlbnRzUmVxdWVzdC5SdWxlc0VudHJ5UgVydWxlcxJVChNwb\
  HVnaW5Db25maWd1cmF0aW9uGAUgASgLMiMuaW8ucGFjdC5wbHVnaW4uUGx1Z2luQ29uZmlndXJhdGlvblITcGx1Z2luQ2\
  9uZmlndXJhdGlvbhpXCgpSdWxlc0VudHJ5EhAKA2tleRgBIAEoCVIDa2V5EjMKBXZhbHVlGAIgASgLMh0uaW8ucGFjdC5\
  wbHVnaW4uTWF0Y2hpbmdSdWxlc1IFdmFsdWU6AjgBIkkKE0NvbnRlbnRUeXBlTWlzbWF0Y2gSGgoIZXhwZWN0ZWQYASAB\
  KAlSCGV4cGVjdGVkEhYKBmFjdHVhbBgCIAEoCVIGYWN0dWFsIsMBCg9Db250ZW50TWlzbWF0Y2gSNwoIZXhwZWN0ZWQYA\
  SABKAsyGy5nb29nbGUucHJvdG9idWYuQnl0ZXNWYWx1ZVIIZXhwZWN0ZWQSMwoGYWN0dWFsGAIgASgLMhsuZ29vZ2xlLn\
  Byb3RvYnVmLkJ5dGVzVmFsdWVSBmFjdHVhbBIaCghtaXNtYXRjaBgDIAEoCVIIbWlzbWF0Y2gSEgoEcGF0aBgEIAEoCVI\
  EcGF0aBISCgRkaWZmGAUgASgJUgRkaWZmIlQKEUNvbnRlbnRNaXNtYXRjaGVzEj8KCm1pc21hdGNoZXMYASADKAsyHy5p\
  by5wYWN0LnBsdWdpbi5Db250ZW50TWlzbWF0Y2hSCm1pc21hdGNoZXMipwIKF0NvbXBhcmVDb250ZW50c1Jlc3BvbnNlE\
  hQKBWVycm9yGAEgASgJUgVlcnJvchJHCgx0eXBlTWlzbWF0Y2gYAiABKAsyIy5pby5wYWN0LnBsdWdpbi5Db250ZW50VH\
  lwZU1pc21hdGNoUgx0eXBlTWlzbWF0Y2gSTgoHcmVzdWx0cxgDIAMoCzI0LmlvLnBhY3QucGx1Z2luLkNvbXBhcmVDb25\
  0ZW50c1Jlc3BvbnNlLlJlc3VsdHNFbnRyeVIHcmVzdWx0cxpdCgxSZXN1bHRzRW50cnkSEAoDa2V5GAEgASgJUgNrZXkS\
  NwoFdmFsdWUYAiABKAsyIS5pby5wYWN0LnBsdWdpbi5Db250ZW50TWlzbWF0Y2hlc1IFdmFsdWU6AjgBIoABChtDb25ma\
  Wd1cmVJbnRlcmFjdGlvblJlcXVlc3QSIAoLY29udGVudFR5cGUYASABKAlSC2NvbnRlbnRUeXBlEj8KDmNvbnRlbnRzQ2\
  9uZmlnGAIgASgLMhcuZ29vZ2xlLnByb3RvYnVmLlN0cnVjdFIOY29udGVudHNDb25maWciUwoMTWF0Y2hpbmdSdWxlEhI\
  KBHR5cGUYASABKAlSBHR5cGUSLwoGdmFsdWVzGAIgASgLMhcuZ29vZ2xlLnByb3RvYnVmLlN0cnVjdFIGdmFsdWVzIkEK\
  DU1hdGNoaW5nUnVsZXMSMAoEcnVsZRgBIAMoCzIcLmlvLnBhY3QucGx1Z2luLk1hdGNoaW5nUnVsZVIEcnVsZSJQCglHZ\
  W5lcmF0b3ISEgoEdHlwZRgBIAEoCVIEdHlwZRIvCgZ2YWx1ZXMYAiABKAsyFy5nb29nbGUucHJvdG9idWYuU3RydWN0U\
  gZ2YWx1ZXMisQEKE1BsdWdpbkNvbmZpZ3VyYXRpb24SUwoYaW50ZXJhY3Rpb25Db25maWd1cmF0aW9uGAEgASgLMhcuZ2\
  9vZ2xlLnByb3RvYnVmLlN0cnVjdFIYaW50ZXJhY3Rpb25Db25maWd1cmF0aW9uEkUKEXBhY3RDb25maWd1cmF0aW9uGAI\
  gASgLMhcuZ29vZ2xlLnByb3RvYnVmLlN0cnVjdFIRcGFjdENvbmZpZ3VyYXRpb24iiAYKE0ludGVyYWN0aW9uUmVzcG9u\
  c2USMAoIY29udGVudHMYASABKAsyFC5pby5wYWN0LnBsdWdpbi5Cb2R5Ughjb250ZW50cxJECgVydWxlcxgCIAMoCzIuL\
  mlvLnBhY3QucGx1Z2luLkludGVyYWN0aW9uUmVzcG9uc2UuUnVsZXNFbnRyeVIFcnVsZXMSUwoKZ2VuZXJhdG9ycxgDIA\
  MoCzIzLmlvLnBhY3QucGx1Z2luLkludGVyYWN0aW9uUmVzcG9uc2UuR2VuZXJhdG9yc0VudHJ5UgpnZW5lcmF0b3JzEkE\
  KD21lc3NhZ2VNZXRhZGF0YRgEIAEoCzIXLmdvb2dsZS5wcm90b2J1Zi5TdHJ1Y3RSD21lc3NhZ2VNZXRhZGF0YRJVChNw\
  bHVnaW5Db25maWd1cmF0aW9uGAUgASgLMiMuaW8ucGFjdC5wbHVnaW4uUGx1Z2luQ29uZmlndXJhdGlvblITcGx1Z2luQ\
  29uZmlndXJhdGlvbhIsChFpbnRlcmFjdGlvbk1hcmt1cBgGIAEoCVIRaW50ZXJhY3Rpb25NYXJrdXASZAoVaW50ZXJhY3\
  Rpb25NYXJrdXBUeXBlGAcgASgOMi4uaW8ucGFjdC5wbHVnaW4uSW50ZXJhY3Rpb25SZXNwb25zZS5NYXJrdXBUeXBlUhV\
  pbnRlcmFjdGlvbk1hcmt1cFR5cGUSGgoIcGFydE5hbWUYCCABKAlSCHBhcnROYW1lGlcKClJ1bGVzRW50cnkSEAoDa2V5\
  GAEgASgJUgNrZXkSMwoFdmFsdWUYAiABKAsyHS5pby5wYWN0LnBsdWdpbi5NYXRjaGluZ1J1bGVzUgV2YWx1ZToCOAEaW\
  AoPR2VuZXJhdG9yc0VudHJ5EhAKA2tleRgBIAEoCVIDa2V5Ei8KBXZhbHVlGAIgASgLMhkuaW8ucGFjdC5wbHVnaW4uR\
  2VuZXJhdG9yUgV2YWx1ZToCOAEiJwoKTWFya3VwVHlwZRIPCgtDT01NT05fTUFSSxAAEggKBEhUTUwQASLSAQocQ29uZ\
  mlndXJlSW50ZXJhY3Rpb25SZXNwb25zZRIUCgVlcnJvchgBIAEoCVIFZXJyb3ISRQoLaW50ZXJhY3Rpb24YAiADKAsyI\
  y5pby5wYWN0LnBsdWdpbi5JbnRlcmFjdGlvblJlc3BvbnNlUgtpbnRlcmFjdGlvbhJVChNwbHVnaW5Db25maWd1cmF0a\
  W9uGAMgASgLMiMuaW8ucGFjdC5wbHVnaW4uUGx1Z2luQ29uZmlndXJhdGlvblITcGx1Z2luQ29uZmlndXJhdGlvbiLTA\
  goWR2VuZXJhdGVDb250ZW50UmVxdWVzdBIwCghjb250ZW50cxgBIAEoCzIULmlvLnBhY3QucGx1Z2luLkJvZHlSCGNvb\
  nRlbnRzElYKCmdlbmVyYXRvcnMYAiADKAsyNi5pby5wYWN0LnBsdWdpbi5HZW5lcmF0ZUNvbnRlbnRSZXF1ZXN0Lkdlb\
  mVyYXRvcnNFbnRyeVIKZ2VuZXJhdG9ycxJVChNwbHVnaW5Db25maWd1cmF0aW9uGAMgASgLMiMuaW8ucGFjdC5wbHVna\
  W4uUGx1Z2luQ29uZmlndXJhdGlvblITcGx1Z2luQ29uZmlndXJhdGlvbhpYCg9HZW5lcmF0b3JzRW50cnkSEAoDa2V5G\
  AEgASgJUgNrZXkSLwoFdmFsdWUYAiABKAsyGS5pby5wYWN0LnBsdWdpbi5HZW5lcmF0b3JSBXZhbHVlOgI4ASJLChdHZ\
  W5lcmF0ZUNvbnRlbnRSZXNwb25zZRIwCghjb250ZW50cxgBIAEoCzIULmlvLnBhY3QucGx1Z2luLkJvZHlSCGNvbnRlb\
  nRzMuIDCgpQYWN0UGx1Z2luElMKCkluaXRQbHVnaW4SIS5pby5wYWN0LnBsdWdpbi5Jbml0UGx1Z2luUmVxdWVzdBoiL\
  mlvLnBhY3QucGx1Z2luLkluaXRQbHVnaW5SZXNwb25zZRJECg9VcGRhdGVDYXRhbG9ndWUSGS5pby5wYWN0LnBsdWdpb\
  i5DYXRhbG9ndWUaFi5nb29nbGUucHJvdG9idWYuRW1wdHkSYgoPQ29tcGFyZUNvbnRlbnRzEiYuaW8ucGFjdC5wbHVna\
  W4uQ29tcGFyZUNvbnRlbnRzUmVxdWVzdBonLmlvLnBhY3QucGx1Z2luLkNvbXBhcmVDb250ZW50c1Jlc3BvbnNlEnEKF\
  ENvbmZpZ3VyZUludGVyYWN0aW9uEisuaW8ucGFjdC5wbHVnaW4uQ29uZmlndXJlSW50ZXJhY3Rpb25SZXF1ZXN0Giwua\
  W8ucGFjdC5wbHVnaW4uQ29uZmlndXJlSW50ZXJhY3Rpb25SZXNwb25zZRJiCg9HZW5lcmF0ZUNvbnRlbnQSJi5pby5w\
  YWN0LnBsdWdpbi5HZW5lcmF0ZUNvbnRlbnRSZXF1ZXN0GicuaW8ucGFjdC5wbHVnaW4uR2VuZXJhdGVDb250ZW50UmV\
  zcG9uc2VCEFoOaW8ucGFjdC5wbHVnaW5iBnByb3RvMw==";

  #[macro_export]
  macro_rules! string_field_descriptor {
    ($name:expr, $n:expr) => (
        prost_types::FieldDescriptorProto {
          name: Some($name.to_string()),
          number: Some($n),
          label: Some(prost_types::field_descriptor_proto::Label::Optional as i32),
          r#type: Some(prost_types::field_descriptor_proto::Type::String as i32),
          type_name: Some("String".to_string()),
          extendee: None,
          default_value: None,
          oneof_index: None,
          json_name: None,
          options: None,
          proto3_optional: None
        }
    );
  }

  #[macro_export]
  macro_rules! bool_field_descriptor {
    ($name:expr, $n:expr) => (
        prost_types::FieldDescriptorProto {
          name: Some($name.to_string()),
          number: Some($n),
          label: Some(prost_types::field_descriptor_proto::Label::Optional as i32),
          r#type: Some(prost_types::field_descriptor_proto::Type::Bool as i32),
          type_name: Some("Bool".to_string()),
          extendee: None,
          default_value: None,
          oneof_index: None,
          json_name: None,
          options: None,
          proto3_optional: None
        }
    );
  }

  #[macro_export]
  macro_rules! message_field_descriptor {
    ($name:expr, $n:expr, $t:expr) => (
      prost_types::FieldDescriptorProto {
        name: Some($name.to_string()),
        number: Some($n),
        label: Some(prost_types::field_descriptor_proto::Label::Optional as i32),
        r#type: Some(prost_types::field_descriptor_proto::Type::Message as i32),
        type_name: Some($t.to_string()),
        extendee: None,
        default_value: None,
        oneof_index: None,
        json_name: None,
        options: None,
        proto3_optional: None
      }
    );
  }

  #[macro_export]
  macro_rules! enum_field_descriptor {
    ($name:expr, $n:expr, $t:expr) => (
      prost_types::FieldDescriptorProto {
        name: Some($name.to_string()),
        number: Some($n),
        label: Some(prost_types::field_descriptor_proto::Label::Optional as i32),
        r#type: Some(prost_types::field_descriptor_proto::Type::Enum as i32),
        type_name: Some($t.to_string()),
        extendee: None,
        default_value: None,
        oneof_index: None,
        json_name: None,
        options: None,
        proto3_optional: None
      }
    );
  }

  #[macro_export]
  macro_rules! bytes_field_descriptor {
    ($name:expr, $n:expr) => (
        prost_types::FieldDescriptorProto {
          name: Some($name.to_string()),
          number: Some($n),
          label: Some(prost_types::field_descriptor_proto::Label::Optional as i32),
          r#type: Some(prost_types::field_descriptor_proto::Type::Bytes as i32),
          type_name: Some("Bytes".to_string()),
          extendee: None,
          default_value: None,
          oneof_index: None,
          json_name: None,
          options: None,
          proto3_optional: None
        }
    );
  }

  #[macro_export]
  macro_rules! u32_field_descriptor {
    ($name:expr, $n:expr) => (
        prost_types::FieldDescriptorProto {
          name: Some($name.to_string()),
          number: Some($n),
          label: Some(prost_types::field_descriptor_proto::Label::Optional as i32),
          r#type: Some(prost_types::field_descriptor_proto::Type::Uint32 as i32),
          type_name: Some("UInteger32".to_string()),
          extendee: None,
          default_value: None,
          oneof_index: None,
          json_name: None,
          options: None,
          proto3_optional: None
        }
    );
  }

  #[macro_export]
  macro_rules! i32_field_descriptor {
    ($name:expr, $n:expr) => (
        prost_types::FieldDescriptorProto {
          name: Some($name.to_string()),
          number: Some($n),
          label: Some(prost_types::field_descriptor_proto::Label::Optional as i32),
          r#type: Some(prost_types::field_descriptor_proto::Type::Int32 as i32),
          type_name: Some("Integer32".to_string()),
          extendee: None,
          default_value: None,
          oneof_index: None,
          json_name: None,
          options: None,
          proto3_optional: None
        }
    );
  }

  #[macro_export]
  macro_rules! u64_field_descriptor {
    ($name:expr, $n:expr) => (
        prost_types::FieldDescriptorProto {
          name: Some($name.to_string()),
          number: Some($n),
          label: Some(prost_types::field_descriptor_proto::Label::Optional as i32),
          r#type: Some(prost_types::field_descriptor_proto::Type::Uint64 as i32),
          type_name: Some("UInteger64".to_string()),
          extendee: None,
          default_value: None,
          oneof_index: None,
          json_name: None,
          options: None,
          proto3_optional: None
        }
    );
  }

  #[macro_export]
  macro_rules! i64_field_descriptor {
    ($name:expr, $n:expr) => (
        prost_types::FieldDescriptorProto {
          name: Some($name.to_string()),
          number: Some($n),
          label: Some(prost_types::field_descriptor_proto::Label::Optional as i32),
          r#type: Some(prost_types::field_descriptor_proto::Type::Int64 as i32),
          type_name: Some("Integer64".to_string()),
          extendee: None,
          default_value: None,
          oneof_index: None,
          json_name: None,
          options: None,
          proto3_optional: None
        }
    );
  }

  #[macro_export]
  macro_rules! f32_field_descriptor {
    ($name:expr, $n:expr) => (
        prost_types::FieldDescriptorProto {
          name: Some($name.to_string()),
          number: Some($n),
          label: Some(prost_types::field_descriptor_proto::Label::Optional as i32),
          r#type: Some(prost_types::field_descriptor_proto::Type::Float as i32),
          type_name: Some("Float".to_string()),
          extendee: None,
          default_value: None,
          oneof_index: None,
          json_name: None,
          options: None,
          proto3_optional: None
        }
    );
  }

  #[macro_export]
  macro_rules! f64_field_descriptor {
    ($name:expr, $n:expr) => (
        prost_types::FieldDescriptorProto {
          name: Some($name.to_string()),
          number: Some($n),
          label: Some(prost_types::field_descriptor_proto::Label::Optional as i32),
          r#type: Some(prost_types::field_descriptor_proto::Type::Double as i32),
          type_name: Some("Double".to_string()),
          extendee: None,
          default_value: None,
          oneof_index: None,
          json_name: None,
          options: None,
          proto3_optional: None
        }
    );
  }

  #[test_log::test]
  fn encode_simple_message_test() {
    // message InitPluginRequest {
    //   // Implementation calling the plugin
    //   string implementation = 1;
    //   // Version of the implementation
    //   string version = 2;
    // }

    let file_descriptor = get_file_descriptor("plugin.proto", ENCODED_MESSAGE).unwrap();

    let field1 = string_field_descriptor!("implementation", 1);
    let field2 = string_field_descriptor!("version", 2);

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
    let mut message = MessageBuilder::new(&descriptor, "InitPluginRequest", &file_descriptor);
    message.set_field_value(&field1, "implementation", MessageFieldValue {
      name: "implementation".to_string(),
      raw_value: Some("plugin-driver-rust".to_string()),
      rtype: RType::String("plugin-driver-rust".to_string())
    });
    message.set_field_value(&field2, "version", MessageFieldValue {
      name: "version".to_string(),
      raw_value: Some("0.0.0".to_string()),
      rtype: RType::String("0.0.0".to_string())
    });

    let result = message.encode_message().unwrap();
    expect!(result.to_vec()).to(be_equal_to(BASE64.decode("ChJwbHVnaW4tZHJpdmVyLXJ1c3QSBTAuMC4w").unwrap()));

    expect!(message.generate_markup("")).to(be_ok().value(
      "|```protobuf
         |message InitPluginRequest {
         |    string implementation = 1;
         |    string version = 2;
         |}
         |```
         |
         ".trim_margin().unwrap()));
  }

  #[test_log::test]
  fn encode_message_bytes_test() {
    // message Body {
    //   // The content type of the body in MIME format (i.e. application/json)
    //   string contentType = 1;
    //   // Bytes of the actual content
    //   google.protobuf.BytesValue content = 2;
    //   // Enum of content type override. This is a hint on how the content type should be treated.
    //   enum ContentTypeHint {
    //     // Determine the form of the content using the default rules of the Pact implementation
    //     DEFAULT = 0;
    //     // Contents must always be treated as a text form
    //     TEXT = 1;
    //     // Contents must always be treated as a binary form
    //     BINARY = 2;
    //   }
    //   // Content type override to apply (if required). If omitted, the default rules of the Pact implementation
    //   // will be used
    //   ContentTypeHint contentTypeHint = 3;
    // }

    let file_descriptor = get_file_descriptor("plugin.proto", ENCODED_MESSAGE).unwrap();

    let body = Body {
      content_type: "application/json".to_string(),
      content: Some("{\"test\": true}".as_bytes().to_vec()),
      content_type_hint: ContentTypeHint::Text as i32
    };
    let encoded = body.encode_to_vec();

    let field1 = string_field_descriptor!("contentType".to_string(), 1);
    let field2 = message_field_descriptor!("content", 2, ".google.protobuf.BytesValue");
    let field3 = enum_field_descriptor!("contentTypeHint", 3, ".io.pact.plugin.Body.ContentTypeHint");
    let enum_proto = EnumDescriptorProto {
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
      enum_type: vec![ enum_proto.clone() ],
      extension_range: vec![],
      oneof_decl: vec![],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![]
    };
    let mut message = MessageBuilder::new(&descriptor, "Body", &file_descriptor);
    message.set_field_value(&field1, "contentType", MessageFieldValue {
      name: "contentType".to_string(),
      raw_value: Some("application/json".to_string()),
      rtype: RType::String("application/json".to_string())
    });

    let bytes_field = bytes_field_descriptor!("value", 1);
    let content_descriptor = DescriptorProto {
      name: Some("BytesValue".to_string()),
      field: vec![
        bytes_field.clone()
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
    let mut bytes_message = MessageBuilder::new(&content_descriptor, "BytesValue", &file_descriptor);
    bytes_message.set_field_value(&bytes_field, "value", MessageFieldValue {
      name: "value".to_string(),
      raw_value: Some("{\"test\": true}".to_string()),
      rtype: RType::Bytes("{\"test\": true}".as_bytes().to_vec())
    });

    message.set_field_value(&field2, "content", MessageFieldValue {
      name: "content".to_string(),
      raw_value: Some("{\"test\": true}".to_string()),
      rtype: RType::Message(Box::new(bytes_message))
    });
    message.set_field_value(&field3, "contentTypeHint", MessageFieldValue {
      name: "contentTypeHint".to_string(),
      raw_value: Some("TEXT".to_string()),
      rtype: RType::Enum(1, enum_proto)
    });

    let result = message.encode_message().unwrap();
    expect!(result.to_vec()).to(be_equal_to(encoded));

    expect!(message.generate_markup("")).to(be_ok().value(
      "|```protobuf
         |message Body {
         |    string contentType = 1;
         |    message .google.protobuf.BytesValue content = 2;
         |    enum .io.pact.plugin.Body.ContentTypeHint contentTypeHint = 3;
         |}
         |```
         |
         ".trim_margin().unwrap()));
  }

  fn get_file_descriptor(file_name: &str, descriptor: &str) -> Option<FileDescriptorProto> {
    let bytes = BASE64.decode(descriptor).unwrap();
    let bytes1 = Bytes::from(bytes);
    let fds = FileDescriptorSet::decode(bytes1).unwrap();
    fds.file.iter().find(|fd| fd.name.clone().unwrap_or_default() == file_name).cloned()
  }

  #[test_log::test]
  fn encode_message_with_map_field_test() {
    // message CompareContentsRequest {
    //   // Expected body from the Pact interaction
    //   Body expected = 1;
    //   // Actual received body
    //   Body actual = 2;
    //   // If unexpected keys or attributes should be allowed. Setting this to false results in additional keys or fields
    //   // will cause a mismatch
    //   bool allow_unexpected_keys = 3;
    //   // Map of expressions to matching rules. The expressions follow the documented Pact matching rule expressions
    //   map<string, MatchingRules> rules = 4;
    //   // Additional data added to the Pact/Interaction by the plugin
    //   PluginConfiguration pluginConfiguration = 5;
    // }

    let file_descriptor = get_file_descriptor("plugin.proto", ENCODED_MESSAGE).unwrap();
    let file_descriptor_set = FileDescriptorSet {
      file: vec![ file_descriptor.clone() ]
    };

    let compare_message = CompareContentsRequest {
      allow_unexpected_keys: true,
      rules: hashmap! {
        "$.one".to_string() => MatchingRules {
          rule: vec![
            MatchingRule {
              r#type: "Type".to_string(),
              values: None
            }
          ]
        },
        "$.two".to_string() => MatchingRules {
          rule: vec![
            MatchingRule {
              r#type: "Regex".to_string(),
              values: Some(::prost_types::Struct {
                fields: btreemap! {
                  "regex".to_string() => ::prost_types::Value {
                    kind: Some(Kind::StringValue(".*".to_string()))
                  }
                }
              })
            }
          ]
        }
      },
      .. CompareContentsRequest::default()
    };
    let mut encoded_buf = BytesMut::with_capacity(compare_message.encoded_len());
    compare_message.encode(&mut encoded_buf).unwrap();

    let field1 = bool_field_descriptor!("allowUnexpectedKeys", 3);
    let field2 = FieldDescriptorProto {
      name: Some("rules".to_string()),
      number: Some(4),
      label: Some(Repeated as i32),
      r#type: Some(field_descriptor_proto::Type::Message as i32),
      type_name: Some(".io.pact.plugin.CompareContentsRequest.RulesEntry".to_string()),
      extendee: None,
      default_value: None,
      oneof_index: None,
      json_name: None,
      options: None,
      proto3_optional: None
    };
    let descriptor = DescriptorProto {
      name: Some("CompareContentsRequest".to_string()),
      field: vec![
        field1.clone(),
        field2.clone()
      ],
      extension: vec![],
      nested_type: vec![
        DescriptorProto {
          name: Some("RulesEntry".to_string()),
          field: vec![
            string_field_descriptor!("key", 1),
            message_field_descriptor!("value", 2, ".io.pact.plugin.MatchingRules")
          ],
          extension: vec![],
          nested_type: vec![],
          enum_type: vec![],
          extension_range: vec![],
          oneof_decl: vec![],
          options: Some(MessageOptions {
            message_set_wire_format: None,
            no_standard_descriptor_accessor: None,
            deprecated: None,
            map_entry: Some(true),
            uninterpreted_option: vec![]
          }),
          reserved_range: vec![],
          reserved_name: vec![]
        }
      ],
      enum_type: vec![],
      extension_range: vec![],
      oneof_decl: vec![],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![]
    };

    let mut message = MessageBuilder::new(&descriptor, "CompareContentsRequest", &file_descriptor);
    message.set_field_value(&field1, "allowUnexpectedKeys", MessageFieldValue {
      name: "allowUnexpectedKeys".to_string(),
      raw_value: Some("true".to_string()),
      rtype: RType::Boolean(true)
    });

    let matching_rule_field = FieldDescriptorProto {
      name: Some("rule".to_string()),
      number: Some(1),
      label: Some(Repeated as i32),
      r#type: Some(field_descriptor_proto::Type::Message as i32),
      type_name: Some(".io.pact.plugin.MatchingRule".to_string()),
      extendee: None,
      default_value: None,
      oneof_index: None,
      json_name: Some("rule".to_string()),
      options: None,
      proto3_optional: None,
    };
    let rule_descriptor = DescriptorProto {
      name: Some("MatchingRules".to_string()),
      field: vec![
        matching_rule_field.clone()
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
    let mut matching_rules = MessageBuilder::new(&rule_descriptor, "MatchingRules", &file_descriptor);

    let type_field_descriptor = string_field_descriptor!("type", 1);
    let values_field_descriptor = message_field_descriptor!("values", 2, ".google.protobuf.Struct");
    let matching_rule_descriptor = DescriptorProto {
      name: Some("MatchingRule".to_string()),
      field: vec![
        type_field_descriptor.clone(),
        values_field_descriptor.clone()
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
    let mut matching_rule_1 = MessageBuilder::new(&matching_rule_descriptor, "MatchingRule", &file_descriptor);
    matching_rule_1.set_field_value(&type_field_descriptor, "type", MessageFieldValue {
      name: "type".to_string(),
      raw_value: Some("Type".to_string()),
      rtype: RType::String("Type".to_string())
    });

    matching_rules.add_repeated_field_value(&matching_rule_field, "rule", MessageFieldValue {
      name: "".to_string(),
      raw_value: None,
      rtype: RType::Message(Box::new(matching_rule_1))
    });

    let mut rule2 = MessageBuilder::new(&rule_descriptor, "MatchingRules", &file_descriptor);
    let mut matching_rule_2 = MessageBuilder::new(&matching_rule_descriptor, "MatchingRule", &file_descriptor);
    matching_rule_2.set_field_value(&type_field_descriptor, "type", MessageFieldValue {
      name: "type".to_string(),
      raw_value: Some("Regex".to_string()),
      rtype: RType::String("Regex".to_string())
    });

    let struct_fields_descriptor = FieldDescriptorProto {
      name: Some("fields".to_string()),
      number: Some(1),
      label: Some(Repeated as i32),
      r#type: Some(field_descriptor_proto::Type::Message as i32),
      type_name: Some(".google.protobuf.Struct.FieldsEntry".to_string()),
      extendee: None,
      default_value: None,
      oneof_index: None,
      json_name: Some("fields".to_string()),
      options: None,
      proto3_optional: None,
    };
    let value_descriptor = FieldDescriptorProto {
      name: Some("value".to_string()),
      number: Some(2),
      label: Some(Optional as i32),
      r#type: Some(field_descriptor_proto::Type::Message as i32),
      type_name: Some(".google.protobuf.Value".to_string()),
      extendee: None,
      default_value: None,
      oneof_index: None,
      json_name: Some("value".to_string()),
      options: None,
      proto3_optional: None
    };
    let struct_descriptor = DescriptorProto {
      name: Some("Struct".to_string()),
      field: vec![
        struct_fields_descriptor.clone()
      ],
      extension: vec![],
      nested_type: vec![
        DescriptorProto {
          name: Some("FieldsEntry".to_string()),
          field: vec![
            FieldDescriptorProto {
              name: Some("key".to_string()),
              number: Some(1),
              label: Some(Optional as i32),
              r#type: Some(field_descriptor_proto::Type::String as i32),
              type_name: None,
              extendee: None,
              default_value: None,
              oneof_index: None,
              json_name: Some("key".to_string()),
              options: None,
              proto3_optional: None,
            },
            value_descriptor.clone()
          ],
          extension: vec![],
          nested_type: vec![],
          enum_type: vec![],
          extension_range: vec![],
          oneof_decl: vec![],
          options: Some(
            MessageOptions {
              message_set_wire_format: None,
              no_standard_descriptor_accessor: None,
              deprecated: None,
              map_entry: Some(true),
              uninterpreted_option: vec![]
            },
          ),
          reserved_range: vec![],
          reserved_name: vec![]
        },
      ],
      enum_type: vec![],
      extension_range: vec![],
      oneof_decl: vec![],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![]
    };

    let value_string_field = FieldDescriptorProto {
      name: Some("string_value".to_string()),
      number: Some(3),
      label: Some(Optional as i32),
      r#type: Some(field_descriptor_proto::Type::String as i32),
      type_name: None,
      extendee: None,
      default_value: None,
      oneof_index: Some(0),
      json_name: Some("stringValue".to_string()),
      options: None,
      proto3_optional: None,
    };
    let value_descriptor = DescriptorProto {
      name: Some("Value".to_string()),
      field: vec![
        value_string_field.clone()
      ],
      extension: vec![],
      nested_type: vec![],
      enum_type: vec![],
      extension_range: vec![],
      oneof_decl: vec![
        OneofDescriptorProto {
          name: Some("kind".to_string()),
          options: None,
        },
      ],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![]
    };
    let mut regex_values = MessageBuilder::new(&value_descriptor, "Value", &file_descriptor);
    regex_values.set_field_value(&value_string_field, "string_value", MessageFieldValue {
      name: "string_value".to_string(),
      raw_value: None,
      rtype: RType::String(".*".to_string())
    });

    let mut matching_rule_values = MessageBuilder::new(&struct_descriptor, "Struct", &file_descriptor);
    matching_rule_values.add_map_field_value(&struct_fields_descriptor, "fields", MessageFieldValue {
      name: "key".to_string(),
      raw_value: None,
      rtype: RType::String("regex".to_string())
    }, MessageFieldValue {
      name: "value".to_string(),
      raw_value: None,
      rtype: RType::Message(Box::new(regex_values))
    });

    matching_rule_2.set_field_value(&values_field_descriptor, "values", MessageFieldValue {
      name: "values".to_string(),
      raw_value: None,
      rtype: RType::Message(Box::new(matching_rule_values))
    });

    rule2.add_repeated_field_value(&matching_rule_field, "rule", MessageFieldValue {
      name: "".to_string(),
      raw_value: None,
      rtype: RType::Message(Box::new(matching_rule_2))
    });

    message.add_map_field_value(&field2, "rules", MessageFieldValue {
      name: "key".to_string(),
      raw_value: None,
      rtype: RType::String("$.one".to_string())
    }, MessageFieldValue {
      name: "value".to_string(),
      raw_value: Some("".to_string()),
      rtype: RType::Message(Box::new(matching_rules))
    });
    message.add_map_field_value(&field2, "rules", MessageFieldValue {
      name: "key".to_string(),
      raw_value: None,
      rtype: RType::String("$.two".to_string())
    }, MessageFieldValue {
      name: "value".to_string(),
      raw_value: None,
      rtype: RType::Message(Box::new(rule2))
    });

    let encoded_fields = decode_message(&mut encoded_buf, &descriptor, &file_descriptor_set).unwrap();
    let mut bytes = message.encode_message().unwrap();
    let result = decode_message(&mut bytes, &descriptor, &file_descriptor_set).unwrap();

    expect!(result.len()).to(be_equal_to(encoded_fields.len()));

    let actual_fields = result.iter().map(|f| f.field_num)
      .sorted()
      .collect_vec();
    let expected_fields = result.iter().map(|f| f.field_num)
      .sorted()
      .collect_vec();
    expect!(actual_fields).to(be_equal_to(expected_fields));

    expect!(result.iter().find(|f| f.field_num == 3)).to(be_equal_to(encoded_fields.iter().find(|f| f.field_num == 3)));

    let expected_map_values = encoded_fields.iter().filter(|f| f.field_num == 4).collect_vec();
    let actual_map_values = result.iter().filter(|f| f.field_num == 4).collect_vec();
    expect!(actual_map_values.len()).to(be_equal_to(2));
    let actual_data = actual_map_values.iter().map(|f| {
      expect!(f.wire_type).to(be_equal_to(WireType::LengthDelimited));
      match &f.data {
        ProtobufFieldData::Message(d, _) => d.clone(),
        _ => panic!("Got an unexpected field type {:?}", f.data)
      }
    }).sorted_by(|a, b| Ord::cmp(&a.len(), &b.len()))
      .collect_vec();
    let expected_data = expected_map_values.iter().map(|f| {
      match &f.data {
        ProtobufFieldData::Message(d, _) => d.clone(),
        _ => panic!("Got an unexpected field type {:?}", f.data)
      }
    }).sorted_by(|a, b| Ord::cmp(&a.len(), &b.len()))
      .collect_vec();
    expect!(actual_data).to(be_equal_to(expected_data));

    expect!(message.generate_markup("")).to(be_ok().value(
      "|```protobuf
         |message CompareContentsRequest {
         |    bool allowUnexpectedKeys = 3;
         |    map<message .io.pact.plugin.CompareContentsRequest.RulesEntry> rules = 4;
         |}
         |```
         |
         ".trim_margin().unwrap()));
  }

  // #[test_log::test]
  // TODO: replace with a test that uses oneOf
  // fn encode_message_for_interaction_response_test() {
  //   // message InteractionResponse {
  //   //   // Contents for the interaction
  //   //   Body contents = 1;
  //   //   // All matching rules to apply
  //   //   map<string, MatchingRules> rules = 2;
  //   //   // Generators to apply
  //   //   map<string, Generator> generators = 3;
  //   //   // For message interactions, any metadata to be applied
  //   //   google.protobuf.Struct messageMetadata = 4;
  //   //   // Plugin specific data to be persisted in the pact file
  //   //   PluginConfiguration pluginConfiguration = 5;
  //   //   // Markdown/HTML formatted text representation of the interaction
  //   //   string interactionMarkup = 6;
  //   //   // Type of markup used
  //   //   enum MarkupType {
  //   //     // CommonMark format
  //   //     COMMON_MARK = 0;
  //   //     // HTML format
  //   //     HTML = 1;
  //   //   }
  //   //   MarkupType interactionMarkupType = 7;
  //   //   // Description of what part this interaction belongs to (in the case of there being more than one, for instance,
  //   //   // request/response messages)
  //   //   string partName = 8;
  //   // }
  //
  //   let interaction_response = InteractionResponse {
  //     contents: Some(Body {
  //       content_type: "application/json".to_string(),
  //       content: Some("{}".as_bytes().to_vec()),
  //       content_type_hint: ContentTypeHint::Text as i32
  //     }),
  //     rules: hashmap! {
  //       "$.test.one".to_string() => MatchingRules {
  //         rule: vec![
  //           MatchingRule {
  //             r#type: "regex".to_string(),
  //             values: None
  //           }
  //         ]
  //       }
  //     },
  //     generators: hashmap! {
  //       "$.test.one".to_string() => Generator {
  //         r#type: "DateTime".to_string(),
  //         values: Some(::prost_types::Struct {
  //           fields: btreemap! {
  //             "format".to_string() => ::prost_types::Value {
  //               kind: Some(::prost_types::value::Kind::StringValue("YYYY-MM-DD".to_string()))
  //             }
  //           }
  //         })
  //       },
  //       "$.test.two".to_string() => Generator {
  //         r#type: "DateTime".to_string(),
  //         values: Some(::prost_types::Struct {
  //           fields: btreemap! {
  //             "format".to_string() => ::prost_types::Value {
  //               kind: Some(::prost_types::value::Kind::StringValue("YYYY-MM-DD".to_string()))
  //             }
  //           }
  //         })
  //       }
  //     },
  //     .. InteractionResponse::default()
  //   };
  //
  //   let mut encoded_buf = BytesMut::with_capacity(interaction_response.encoded_len());
  //   interaction_response.encode(&mut encoded_buf).unwrap();
  //   dbg!(format!("{:0x}", encoded_buf));
  //
  //   expect!(true).to(be_false());
  // }

  const AREA_CALCULATOR_DESCRIPTOR: &str = "CsMHChVhcmVhX2NhbGN1bGF0b3IucHJvdG8SD2FyZWFfY2FsY3VsYX\
  RvciK6AgoMU2hhcGVNZXNzYWdlEjEKBnNxdWFyZRgBIAEoCzIXLmFyZWFfY2FsY3VsYXRvci5TcXVhcmVIAFIGc3F1YXJlEj\
  oKCXJlY3RhbmdsZRgCIAEoCzIaLmFyZWFfY2FsY3VsYXRvci5SZWN0YW5nbGVIAFIJcmVjdGFuZ2xlEjEKBmNpcmNsZRgDIA\
  EoCzIXLmFyZWFfY2FsY3VsYXRvci5DaXJjbGVIAFIGY2lyY2xlEjcKCHRyaWFuZ2xlGAQgASgLMhkuYXJlYV9jYWxjdWxhdG\
  9yLlRyaWFuZ2xlSABSCHRyaWFuZ2xlEkYKDXBhcmFsbGVsb2dyYW0YBSABKAsyHi5hcmVhX2NhbGN1bGF0b3IuUGFyYWxsZW\
  xvZ3JhbUgAUg1wYXJhbGxlbG9ncmFtQgcKBXNoYXBlIikKBlNxdWFyZRIfCgtlZGdlX2xlbmd0aBgBIAEoAlIKZWRnZUxlbm\
  d0aCI5CglSZWN0YW5nbGUSFgoGbGVuZ3RoGAEgASgCUgZsZW5ndGgSFAoFd2lkdGgYAiABKAJSBXdpZHRoIiAKBkNpcmNsZR\
  IWCgZyYWRpdXMYASABKAJSBnJhZGl1cyJPCghUcmlhbmdsZRIVCgZlZGdlX2EYASABKAJSBWVkZ2VBEhUKBmVkZ2VfYhgCIA\
  EoAlIFZWRnZUISFQoGZWRnZV9jGAMgASgCUgVlZGdlQyJICg1QYXJhbGxlbG9ncmFtEh8KC2Jhc2VfbGVuZ3RoGAEgASgCUg\
  piYXNlTGVuZ3RoEhYKBmhlaWdodBgCIAEoAlIGaGVpZ2h0IkQKC0FyZWFSZXF1ZXN0EjUKBnNoYXBlcxgBIAMoCzIdLmFyZW\
  FfY2FsY3VsYXRvci5TaGFwZU1lc3NhZ2VSBnNoYXBlcyIkCgxBcmVhUmVzcG9uc2USFAoFdmFsdWUYASADKAJSBXZhbHVlMq\
  YBCgpDYWxjdWxhdG9yEksKCWNhbGN1bGF0ZRIdLmFyZWFfY2FsY3VsYXRvci5TaGFwZU1lc3NhZ2UaHS5hcmVhX2NhbGN1bG\
  F0b3IuQXJlYVJlc3BvbnNlIgASSwoKY2FsY3VsYXRlMhIcLmFyZWFfY2FsY3VsYXRvci5BcmVhUmVxdWVzdBodLmFyZWFfY2\
  FsY3VsYXRvci5BcmVhUmVzcG9uc2UiAEIcWhdpby5wYWN0L2FyZWFfY2FsY3VsYXRvctACAWIGcHJvdG8z";

  #[test_log::test]
  fn test_packed_repeated_fields() {
    let file_descriptor = get_file_descriptor("area_calculator.proto", AREA_CALCULATOR_DESCRIPTOR).unwrap();
    let area_response_descriptor = file_descriptor.message_type.iter()
        .find(|desc| desc.name.clone().unwrap_or_default() == "AreaResponse")
        .unwrap();
    let values_field_descriptor = area_response_descriptor.field.iter()
        .find(|desc| desc.name.clone().unwrap_or_default() == "value")
        .unwrap();
    let mut builder = MessageBuilder::new(area_response_descriptor, "AreaResponse", &file_descriptor);
    let message_field_value = MessageFieldValue {
      name: "value".to_string(),
      raw_value: Some("12.0".to_string()),
      rtype: RType::Float(12.0)
    };
    let message_field_value2 = MessageFieldValue {
      name: "value".to_string(),
      raw_value: Some("9.0".to_string()),
      rtype: RType::Float(9.0)
    };
    builder.add_repeated_field_value(values_field_descriptor, "value", message_field_value);
    builder.add_repeated_field_value(values_field_descriptor, "value", message_field_value2);

    let expected = vec![10, 8, 0, 0, 64, 65, 0, 0, 16, 65];
    let result = builder.encode_message().unwrap();
    expect!(result.to_vec()).to(be_equal_to(expected));
  }

  #[test_log::test]
  fn test_field_with_global_enum() {
    let bytes: &[u8] = &DESCRIPTOR_WITH_ENUM_BYTES;
    let buffer = Bytes::from(bytes);
    let fds: FileDescriptorSet = FileDescriptorSet::decode(buffer).unwrap();

    let main_descriptor = fds.file.iter()
      .find(|fd| fd.name.clone().unwrap_or_default() == "area_calculator.proto")
      .unwrap();
    let message_descriptor = main_descriptor.message_type.iter()
      .find(|md| md.name.clone().unwrap_or_default() == "Rectangle").unwrap();
    let mut message_builder = MessageBuilder::new(&message_descriptor, "Rectangle", main_descriptor);

    let value_field_descriptor = message_descriptor.field.iter()
      .find(|desc| desc.name.clone().unwrap_or_default() == "ad_break_type")
      .unwrap();
    let field_value = MessageFieldValue {
      name: "ad_break_type".to_string(),
      raw_value: Some("AUDIO_AD_BREAK".to_string()),
      rtype: RType::Enum(1, main_descriptor.enum_type.first().cloned().unwrap())
    };
    message_builder.set_field_value(value_field_descriptor, "ad_break_type", field_value);

    let expected = vec![
      40, // Field 5, VARINT
      1   // Value 1
    ];
    let result = message_builder.encode_message().unwrap();
    expect!(result.to_vec()).to(be_equal_to(expected));
  }
}
