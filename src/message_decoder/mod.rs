//! Decoder for encoded Protobuf messages using the descriptors

use std::fmt::{Debug, Display, Formatter};
use std::mem;
use std::str::from_utf8;

use anyhow::anyhow;
use bytes::{Buf, Bytes, BytesMut};
use itertools::Itertools;
use prost::encoding::{decode_key, decode_varint, encode_varint, WireType};
use prost_types::{DescriptorProto, EnumDescriptorProto, FieldDescriptorProto, FileDescriptorSet};
use prost_types::field_descriptor_proto::Type;
use tracing::{debug, error, trace, warn};

use crate::utils::{
  as_hex, find_enum_by_name, find_enum_by_name_in_message, find_message_descriptor_for_type, is_repeated_field, last_name, should_be_packed_type
};

pub mod generators;

/// Decoded Protobuf field
#[derive(Clone, Debug, PartialEq)]
pub struct ProtobufField {
  /// Field number
  pub field_num: u32,
  /// Field name
  pub field_name: String,
  /// Wire type for the field
  pub wire_type: WireType,
  /// Field data
  pub data: ProtobufFieldData,
  /// Additional field data (for repeated fields)
  pub additional_data: Vec<ProtobufFieldData>,
  /// Descriptor for this field
  pub descriptor: FieldDescriptorProto
}

impl ProtobufField {
  /// Create a copy of this field with the value replaced with the default
  pub fn default_field_value(&self) -> ProtobufField {
    ProtobufField {
      field_num: self.field_num,
      field_name: self.field_name.clone(),
      wire_type: self.wire_type,
      data: self.data.default_field_value(&self.descriptor),
      additional_data: vec![],
      descriptor: self.descriptor.clone()
    }
  }

  /// Configure a field with the default value
  pub fn default_field(
    field_descriptor: &FieldDescriptorProto,
    descriptor: &DescriptorProto,
    fds: &FileDescriptorSet
  ) -> Option<ProtobufField> {
    default_field_data(field_descriptor, descriptor, fds).map(|data|
      ProtobufField {
        field_num: field_descriptor.number.unwrap_or_default() as u32,
        field_name: field_descriptor.name.clone().unwrap_or_default(),
        wire_type: wire_type_for_field(field_descriptor),
        data,
        additional_data: vec![],
        descriptor: field_descriptor.clone()
      }
    )
  }

  /// If the field contains the default value for its type
  pub fn is_default_value(&self) -> bool {
    self.data.is_default_field_value()
  }

  /// If the field is a Protobuf repeated field
  pub fn repeated_field(&self) -> bool {
    is_repeated_field(&self.descriptor)
  }

  /// Creates a new field that is a clone of this one but with the data set
  pub fn clone_with_data(&self, data: &ProtobufFieldData) -> Self {
    ProtobufField {
      data: data.clone(),
      additional_data: vec![],
      .. self.clone()
    }
  }
}

fn default_field_data(
  field_descriptor: &FieldDescriptorProto,
  descriptor: &DescriptorProto,
  fds: &FileDescriptorSet
) -> Option<ProtobufFieldData> {
  match &field_descriptor.default_value {
    Some(s) => {
      // For numeric types, contains the original text representation of the value.
      // For booleans, "true" or "false".
      // For strings, contains the default text contents (not escaped in any way).
      // For bytes, contains the C escaped value.  All bytes >= 128 are escaped.
      match field_descriptor.r#type() {
        Type::Double => Some(ProtobufFieldData::Double(s.parse().unwrap_or_default())),
        Type::Float => Some(ProtobufFieldData::Float(s.parse().unwrap_or_default())),
        Type::Int64 => Some(ProtobufFieldData::Integer64(s.parse().unwrap_or_default())),
        Type::Uint64 => Some(ProtobufFieldData::UInteger64(s.parse().unwrap_or_default())),
        Type::Int32 => Some(ProtobufFieldData::Integer32(s.parse().unwrap_or_default())),
        Type::Fixed64 => Some(ProtobufFieldData::Integer64(s.parse().unwrap_or_default())),
        Type::Fixed32 => Some(ProtobufFieldData::Integer32(s.parse().unwrap_or_default())),
        Type::Bool => Some(ProtobufFieldData::Boolean(s == "true")),
        Type::String => Some(ProtobufFieldData::String(s.clone())),
        Type::Bytes => Some(ProtobufFieldData::Bytes(s.as_bytes().to_vec())),
        Type::Uint32 => Some(ProtobufFieldData::UInteger32(s.parse().unwrap_or_default())),
        Type::Enum => {
          let enum_type_name = field_descriptor.type_name.clone().unwrap_or_default();
          find_enum_by_name_in_message(&descriptor.enum_type, enum_type_name.as_str())
            .or_else(|| find_enum_by_name(fds, enum_type_name.as_str()))
            .map(|enum_proto| ProtobufFieldData::Enum(s.parse().unwrap_or_default(), enum_proto.clone()))
        },
        Type::Sfixed32 => Some(ProtobufFieldData::Integer32(s.parse().unwrap_or_default())),
        Type::Sfixed64 => Some(ProtobufFieldData::Integer64(s.parse().unwrap_or_default())),
        Type::Sint32 => Some(ProtobufFieldData::Integer32(s.parse().unwrap_or_default())),
        Type::Sint64 => Some(ProtobufFieldData::Integer64(s.parse().unwrap_or_default())),
        _ => None
      }
    }
    None => {
      // For strings, the default value is the empty string.
      // For bytes, the default value is empty bytes.
      // For bools, the default value is false.
      // For numeric types, the default value is zero.
      // For enums, the default value is the first defined enum value, which must be 0.
      // For message fields, the field is not set. Its exact value is language-dependent.
      match field_descriptor.r#type() {
        Type::Double => Some(ProtobufFieldData::Double(0.0)),
        Type::Float => Some(ProtobufFieldData::Float(0.0)),
        Type::Int64 => Some(ProtobufFieldData::Integer64(0)),
        Type::Uint64 => Some(ProtobufFieldData::UInteger64(0)),
        Type::Int32 => Some(ProtobufFieldData::Integer32(0)),
        Type::Fixed64 => Some(ProtobufFieldData::Integer64(0)),
        Type::Fixed32 => Some(ProtobufFieldData::Integer32(0)),
        Type::Bool => Some(ProtobufFieldData::Boolean(false)),
        Type::String => Some(ProtobufFieldData::String(String::default())),
        Type::Bytes => Some(ProtobufFieldData::Bytes(vec![])),
        Type::Uint32 => Some(ProtobufFieldData::UInteger32(0)),
        Type::Enum => {
          let enum_type_name = field_descriptor.type_name.clone().unwrap_or_default();
          find_enum_by_name_in_message(&descriptor.enum_type, enum_type_name.as_str())
            .or_else(|| find_enum_by_name(fds, enum_type_name.as_str()))
            .map(|enum_proto| ProtobufFieldData::Enum(0, enum_proto.clone()))
        },
        Type::Sfixed32 => Some(ProtobufFieldData::Integer32(0)),
        Type::Sfixed64 => Some(ProtobufFieldData::Integer64(0)),
        Type::Sint32 => Some(ProtobufFieldData::Integer32(0)),
        Type::Sint64 => Some(ProtobufFieldData::Integer64(0)),
        _ => None
      }
    }
  }
}

fn wire_type_for_field(descriptor: &FieldDescriptorProto) -> WireType {
  match descriptor.r#type() {
    Type::Double | Type::Fixed64 | Type::Sfixed64 => WireType::SixtyFourBit,
    Type::Float | Type::Fixed32 | Type::Sfixed32 => WireType::ThirtyTwoBit,
    Type::Int64 | Type::Uint64 | Type::Int32 | Type::Bool | Type::Uint32 | Type::Enum |
    Type::Sint32 | Type::Sint64 => WireType::Varint,
    _ => WireType::LengthDelimited
  }
}

impl Display for ProtobufField {
  fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
    write!(f, "{}:({}, {:?}, {})", self.field_num, self.field_name, self.wire_type, self.data.type_name())
  }
}

/// Decoded Protobuf field data
#[derive(Clone, Debug, PartialEq)]
pub enum ProtobufFieldData {
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
  Message(Vec<u8>, DescriptorProto),
  /// For field data that does not match the descriptor
  Unknown(Vec<u8>)
}

impl ProtobufFieldData {
  /// Returns the type name of the field.
  pub fn type_name(&self) -> &'static str {
    match self {
      ProtobufFieldData::String(_) => "String",
      ProtobufFieldData::Boolean(_) => "Boolean",
      ProtobufFieldData::UInteger32(_) => "UInteger32",
      ProtobufFieldData::Integer32(_) => "Integer32",
      ProtobufFieldData::UInteger64(_) => "UInteger64",
      ProtobufFieldData::Integer64(_) => "Integer64",
      ProtobufFieldData::Float(_) => "Float",
      ProtobufFieldData::Double(_) => "Double",
      ProtobufFieldData::Bytes(_) => "Bytes",
      ProtobufFieldData::Enum(_, _) => "Enum",
      ProtobufFieldData::Message(_, _) => "Message",
      ProtobufFieldData::Unknown(_) => "Unknown"
    }
  }

  /// Converts the data for this value into a byte array
  pub fn as_bytes(&self) -> Vec<u8> {
    match self {
      ProtobufFieldData::String(s) => s.as_bytes().to_vec(),
      ProtobufFieldData::Boolean(b) => vec![ *b as u8 ],
      ProtobufFieldData::UInteger32(n) => n.to_le_bytes().to_vec(),
      ProtobufFieldData::Integer32(n) => n.to_le_bytes().to_vec(),
      ProtobufFieldData::UInteger64(n) => n.to_le_bytes().to_vec(),
      ProtobufFieldData::Integer64(n) => n.to_le_bytes().to_vec(),
      ProtobufFieldData::Float(n) => n.to_le_bytes().to_vec(),
      ProtobufFieldData::Double(n) => n.to_le_bytes().to_vec(),
      ProtobufFieldData::Bytes(b) => b.clone(),
      ProtobufFieldData::Enum(_, _) => self.to_string().as_bytes().to_vec(),
      ProtobufFieldData::Message(b, _) => b.clone(),
      ProtobufFieldData::Unknown(data) => data.clone()
    }
  }

  /// Return the default value for this field data
  pub fn default_field_value(&self, descriptor: &FieldDescriptorProto) -> ProtobufFieldData {
    match &descriptor.default_value {
      Some(s) => {
        // For numeric types, contains the original text representation of the value.
        // For booleans, "true" or "false".
        // For strings, contains the default text contents (not escaped in any way).
        // For bytes, contains the C escaped value.  All bytes >= 128 are escaped.
        match self {
          ProtobufFieldData::String(_) => ProtobufFieldData::String(s.clone()),
          ProtobufFieldData::Boolean(_) => ProtobufFieldData::Boolean(s == "true"),
          ProtobufFieldData::UInteger32(_) => ProtobufFieldData::UInteger32(s.parse().unwrap_or_default()),
          ProtobufFieldData::Integer32(_) => ProtobufFieldData::Integer32(s.parse().unwrap_or_default()),
          ProtobufFieldData::UInteger64(_) => ProtobufFieldData::UInteger64(s.parse().unwrap_or_default()),
          ProtobufFieldData::Integer64(_) => ProtobufFieldData::Integer64(s.parse().unwrap_or_default()),
          ProtobufFieldData::Float(_) => ProtobufFieldData::Float(s.parse().unwrap_or_default()),
          ProtobufFieldData::Double(_) => ProtobufFieldData::Double(s.parse().unwrap_or_default()),
          ProtobufFieldData::Bytes(_) => ProtobufFieldData::Bytes(s.as_bytes().to_vec()),
          ProtobufFieldData::Enum(_, descriptor) => ProtobufFieldData::Enum(s.parse().unwrap_or_default(), descriptor.clone()),
          ProtobufFieldData::Message(_, descriptor) => ProtobufFieldData::Message(Default::default(), descriptor.clone()),
          ProtobufFieldData::Unknown(_) => ProtobufFieldData::Unknown(Default::default())
        }
      }
      None => {
        // For strings, the default value is the empty string.
        // For bytes, the default value is empty bytes.
        // For bools, the default value is false.
        // For numeric types, the default value is zero.
        // For enums, the default value is the first defined enum value, which must be 0.
        // For message fields, the field is not set. Its exact value is language-dependent.
        match self {
          ProtobufFieldData::String(_) => ProtobufFieldData::String(Default::default()),
          ProtobufFieldData::Boolean(_) => ProtobufFieldData::Boolean(false),
          ProtobufFieldData::UInteger32(_) => ProtobufFieldData::UInteger32(0),
          ProtobufFieldData::Integer32(_) => ProtobufFieldData::Integer32(0),
          ProtobufFieldData::UInteger64(_) => ProtobufFieldData::UInteger64(0),
          ProtobufFieldData::Integer64(_) => ProtobufFieldData::Integer64(0),
          ProtobufFieldData::Float(_) => ProtobufFieldData::Float(0.0),
          ProtobufFieldData::Double(_) => ProtobufFieldData::Double(0.0),
          ProtobufFieldData::Bytes(_) => ProtobufFieldData::Bytes(Default::default()),
          ProtobufFieldData::Enum(_, descriptor) => ProtobufFieldData::Enum(0, descriptor.clone()),
          ProtobufFieldData::Message(_, descriptor) => ProtobufFieldData::Message(Default::default(), descriptor.clone()),
          ProtobufFieldData::Unknown(_) => ProtobufFieldData::Unknown(Default::default())
        }
      }
    }
  }

  /// If the value is the default for the type
  pub fn is_default_field_value(&self) -> bool {
    // For strings, the default value is the empty string.
    // For bytes, the default value is empty bytes.
    // For bools, the default value is false.
    // For numeric types, the default value is zero.
    // For enums, the default value is the first defined enum value, which must be 0.
    // For message fields, the field is not set. Its exact value is language-dependent.
    match self {
      ProtobufFieldData::String(v) => v.is_empty(),
      ProtobufFieldData::Boolean(v) => !*v,
      ProtobufFieldData::UInteger32(v) => *v == 0,
      ProtobufFieldData::Integer32(v) => *v == 0,
      ProtobufFieldData::UInteger64(v) => *v == 0,
      ProtobufFieldData::Integer64(v) => *v == 0,
      ProtobufFieldData::Float(v) => *v == 0.0,
      ProtobufFieldData::Double(v) => *v == 0.0,
      ProtobufFieldData::Bytes(v) => v.is_empty(),
      ProtobufFieldData::Enum(v, _) => *v == 0,
      ProtobufFieldData::Message(v, _) => v.is_empty(),
      ProtobufFieldData::Unknown(v) => v.is_empty()
    }
  }

  pub fn as_u64(&self) -> Option<u64> {
    match self {
      ProtobufFieldData::UInteger64(n) => Some(*n),
      _ => None
    }
  }

  pub fn as_u32(&self) -> Option<u32> {
    match self {
      ProtobufFieldData::UInteger32(n) => Some(*n),
      _ => None
    }
  }

  pub fn as_i64(&self) -> Option<i64> {
    match self {
      ProtobufFieldData::Integer64(n) => Some(*n),
      _ => None
    }
  }

  pub fn as_i32(&self) -> Option<i32> {
    match self {
      ProtobufFieldData::Integer32(n) => Some(*n),
      _ => None
    }
  }

  pub fn as_f64(&self) -> Option<f64> {
    match self {
      ProtobufFieldData::Double(n) => Some(*n),
      _ => None
    }
  }

  pub fn as_f32(&self) -> Option<f32> {
    match self {
      ProtobufFieldData::Float(n) => Some(*n),
      _ => None
    }
  }

  pub fn as_str(&self) -> Option<&str> {
    match self {
      ProtobufFieldData::String(s) => Some(s.as_str()),
      _ => None
    }
  }
}

impl Display for ProtobufFieldData {
  fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
    match self {
      ProtobufFieldData::String(s) => write!(f, "\"{}\"", s),
      ProtobufFieldData::Boolean(b) => write!(f, "{}", b),
      ProtobufFieldData::UInteger32(n) => write!(f, "{}", n),
      ProtobufFieldData::Integer32(n) => write!(f, "{}", n),
      ProtobufFieldData::UInteger64(n) => write!(f, "{}", n),
      ProtobufFieldData::Integer64(n) => write!(f, "{}", n),
      ProtobufFieldData::Float(n) => write!(f, "{}", n),
      ProtobufFieldData::Double(n) => write!(f, "{}", n),
      ProtobufFieldData::Bytes(b) => if b.len() <= 16 {
        write!(f, "{}", as_hex(b.as_slice()))
      } else {
        write!(f, "{}... ({} bytes)", as_hex(&b[0..16]), b.len())
      },
      ProtobufFieldData::Enum(n, descriptor) => {
        let enum_value_name = descriptor.value.iter()
          .find(|v| v.number.is_some() && v.number.as_ref().unwrap() == n)
          .map(|v| v.name.clone().unwrap_or_default()).unwrap_or_else(|| "unknown".to_string());
        write!(f, "{}", enum_value_name)
      },
      ProtobufFieldData::Message(_, descriptor) => {
        write!(f, "{}", descriptor.name.clone().unwrap_or_else(|| "unknown".to_string()))
      }
      ProtobufFieldData::Unknown(b) => if b.len() <= 16 {
        write!(f, "{}", as_hex(b.as_slice()))
      } else {
        write!(f, "{}... ({} bytes)", as_hex(&b[0..16]), b.len())
      }
    }
  }
}

/// Decodes the Protobuf message using the descriptors and returns an array of ProtobufField values.
/// This will return a value for each field value in the incoming bytes in the same order, and will
/// not consolidate repeated fields.
pub fn decode_message<B>(
  buffer: &mut B,
  descriptor: &DescriptorProto,
  descriptors: &FileDescriptorSet
) -> anyhow::Result<Vec<ProtobufField>>
  where B: Buf {
  trace!("Decoding message using descriptor {:?}", descriptor);
  trace!("all descriptors available for decoding the message: {:?}", descriptors);
  trace!("Incoming buffer has {} bytes", buffer.remaining());
  let mut fields = vec![];

  while buffer.has_remaining() {
    let (field_num, wire_type) = decode_key(buffer)?;
    trace!(field_num, ?wire_type, "read field header, bytes remaining = {}", buffer.remaining());

    match &find_field_descriptor(field_num as i32, descriptor) {
      Ok(field_descriptor) => {
        let field_name = field_descriptor.name();
        trace!("field_name = {}", field_name);
        let data = match wire_type {

          // Variable Integer types
          WireType::Varint => {
            let varint = decode_varint(buffer)?;
            let t: Type = field_descriptor.r#type();
            match t {
              Type::Int64 => vec![ (ProtobufFieldData::Integer64(varint as i64), wire_type) ],
              Type::Uint64 => vec![ (ProtobufFieldData::UInteger64(varint), wire_type) ],
              Type::Int32 => vec![ (ProtobufFieldData::Integer32(varint as i32), wire_type) ],
              Type::Bool => vec![ (ProtobufFieldData::Boolean(varint > 0), wire_type) ],
              Type::Uint32 => vec![ (ProtobufFieldData::UInteger32(varint as u32), wire_type) ],
              Type::Enum => {
                vec![ (decode_enum(descriptor, descriptors, &field_descriptor, varint)?, wire_type) ]
              },
              Type::Sint32 => {
                let value = varint as u32;
                vec![ (ProtobufFieldData::Integer32(((value >> 1) as i32) ^ (-((value & 1) as i32))), wire_type) ]
              },
              Type::Sint64 => vec![ (ProtobufFieldData::Integer64(((varint >> 1) as i64) ^ (-((varint & 1) as i64))), wire_type) ],
              _ => {
                error!("Was expecting {:?} but received an unknown varint type", t);
                vec![ (ProtobufFieldData::Unknown(varint.to_le_bytes().to_vec()), wire_type) ]
              }
            }
          }

          // Fixed size 64 bit values
          WireType::SixtyFourBit => {
            let t: Type = field_descriptor.r#type();
            match t {
              Type::Double => vec![ (ProtobufFieldData::Double(buffer.get_f64_le()), wire_type) ],
              Type::Fixed64 => vec![ (ProtobufFieldData::UInteger64(buffer.get_u64_le()), wire_type) ],
              Type::Sfixed64 => vec![ (ProtobufFieldData::Integer64(buffer.get_i64_le()), wire_type) ],
              _ => {
                error!("Was expecting {:?} but received an unknown 64 bit type", t);
                let value = buffer.get_u64_le();
                vec![ (ProtobufFieldData::Unknown(value.to_le_bytes().to_vec()), wire_type) ]
              }
            }
          }

          // Length delimited types
          WireType::LengthDelimited => {
            let data_length = decode_varint(buffer)?;
            let mut data_buffer = if buffer.remaining() >= data_length as usize {
              buffer.copy_to_bytes(data_length as usize)
            } else {
              return Err(anyhow!("Insufficient data remaining ({} bytes) to read {} bytes for field {}", buffer.remaining(), data_length, field_num));
            };

            let t: Type = field_descriptor.r#type();
            trace!(field_type = ?t, data_buffer = ?data_buffer);

            match t {
              Type::String => vec![ (ProtobufFieldData::String(from_utf8(&data_buffer)?.to_string()), wire_type) ],

              Type::Message => {
                let full_type_name = field_descriptor.type_name();
                trace!(%full_type_name, "Embedded message");
                // TODO: replace with proper support for nested fields
                // this code checks fully qualified name first, if it can find it, this means the type name was a 
                // valid fully-qualified reference;
                // if it's not found, it's a nested type, so we look for it in the nested types of the current message
                // This misses the case when the type name refers to a fully-qualified nested type in another message
                // or package. This also doesn't deal with relative paths, but I don't think descriptors actually
                // contain those.
                let message_proto = find_message_descriptor_for_type(full_type_name, descriptors).map(|(d,_)|d)
                .or_else(|_| {
                  descriptor.nested_type.iter().find(
                    |message_descriptor| message_descriptor.name.as_deref() == Some(last_name(full_type_name))
                  ).cloned().ok_or_else(|| anyhow!("Did not find the message {:?} for the field {} in the Protobuf descriptor", field_descriptor.type_name, field_num))
                })?;
                vec![ (ProtobufFieldData::Message(data_buffer.to_vec(), message_proto), wire_type) ]
              }

              Type::Bytes => vec![ (ProtobufFieldData::Bytes(data_buffer.to_vec()), wire_type) ],

              _ => if should_be_packed_type(t) && is_repeated_field(&field_descriptor) {
                debug!("Reading length delimited field as a packed repeated field");
                decode_packed_field(field_descriptor, descriptor, descriptors, &mut data_buffer)?
              } else {
                error!("Was expecting {:?} but received an unknown length-delimited type", t);
                let mut buf = BytesMut::with_capacity((data_length + 8) as usize);
                encode_varint(data_length, &mut buf);
                buf.extend_from_slice(&data_buffer);
                vec![ (ProtobufFieldData::Unknown(buf.freeze().to_vec()), wire_type) ]
              }
            }
          }

          // Fixed size 32 bit values
          WireType::ThirtyTwoBit => {
            let t: Type = field_descriptor.r#type();
            match t {
              Type::Float => vec![ (ProtobufFieldData::Float(buffer.get_f32_le()), wire_type) ],
              Type::Fixed32 => vec![ (ProtobufFieldData::UInteger32(buffer.get_u32_le()), wire_type) ],
              Type::Sfixed32 => vec![ (ProtobufFieldData::Integer32(buffer.get_i32_le()), wire_type) ],
              _ => {
                error!("Was expecting {:?} but received an unknown fixed 32 bit type", t);
                let value = buffer.get_u32_le();
                vec![ (ProtobufFieldData::Unknown(value.to_le_bytes().to_vec()), wire_type) ]
              }
            }
          }
          _ => return Err(anyhow!("Messages with {:?} wire type fields are not supported", wire_type))
        };

        trace!(field_num, ?wire_type, ?data, "read field, bytes remaining = {}", buffer.remaining());
        for (data, wire_type) in data {
          fields.push(ProtobufField {
            field_num,
            field_name: field_name.to_string(),
            wire_type,
            data,
            additional_data: vec![],
            descriptor: field_descriptor.clone()
          });
        }
      }
      Err(err) => {
        warn!("Was not able to decode field: {}", err);
        let data = match wire_type {
          WireType::Varint => {
            let result = decode_varint(buffer)?;
            debug!("Unknown varint value: {}", result);
            // varints are never more than 10 bytes
            let mut buf = BytesMut::with_capacity(10);
            encode_varint(result, &mut buf);
            buf.freeze().to_vec()
          },
          WireType::SixtyFourBit => buffer.get_u64().to_le_bytes().to_vec(),
          WireType::LengthDelimited => {
            let data_length = decode_varint(buffer)?;
            let mut buf = BytesMut::with_capacity((data_length + 8) as usize);
            encode_varint(data_length, &mut buf);
            buf.extend_from_slice(&buffer.copy_to_bytes(data_length as usize));
            buf.freeze().to_vec()
          }
          WireType::ThirtyTwoBit => buffer.get_u32().to_le_bytes().to_vec(),
          _ => return Err(anyhow!("Messages with {:?} wire type fields are not supported", wire_type))
        };
        fields.push(ProtobufField {
          field_num,
          field_name: "unknown".to_string(),
          wire_type,
          data: ProtobufFieldData::Unknown(data),
          additional_data: vec![],
          descriptor: Default::default()
        });
      }
    }
  }

  let result = fields.iter()
    .sorted_by(|a, b| Ord::cmp(&a.field_num, &b.field_num))
    .cloned()
    .collect_vec();
  debug!("Decoded message has {} fields", result.len());
  trace!("Decoded message = {:?}", result);
  Ok(result)
}

fn decode_enum(
  descriptor: &DescriptorProto,
  descriptors: &FileDescriptorSet,
  field_descriptor: &FieldDescriptorProto,
  varint: u64
) -> anyhow::Result<ProtobufFieldData> {
  let enum_type_name = field_descriptor.type_name.clone().unwrap_or_default();
  let enum_proto = find_enum_by_name_in_message(&descriptor.enum_type, enum_type_name.as_str())
    .or_else(|| find_enum_by_name(descriptors, enum_type_name.as_str()))
    .ok_or_else(|| anyhow!("Did not find the enum {} for the field in the Protobuf descriptor", enum_type_name))?;
  Ok(ProtobufFieldData::Enum(varint as i32, enum_proto.clone()))
}

fn decode_packed_field(
  field: &FieldDescriptorProto,
  descriptor: &DescriptorProto,
  descriptors: &FileDescriptorSet,
  data: &mut Bytes
) -> anyhow::Result<Vec<(ProtobufFieldData, WireType)>> {
  let mut values = vec![];
  let t: Type = field.r#type();
  match t {
    Type::Double => {
      while data.remaining() >= mem::size_of::<f64>() {
        values.push((ProtobufFieldData::Double(data.get_f64_le()), WireType::SixtyFourBit));
      }
    }
    Type::Float => {
      while data.remaining() >= mem::size_of::<f32>() {
        values.push((ProtobufFieldData::Float(data.get_f32_le()), WireType::ThirtyTwoBit));
      }
    }
    Type::Int64 => {
      while data.remaining() > 0 {
        let varint = decode_varint(data)?;
        values.push((ProtobufFieldData::Integer64(varint as i64), WireType::Varint));
      }
    }
    Type::Uint64 => {
      while data.remaining() > 0 {
        let varint = decode_varint(data)?;
        values.push((ProtobufFieldData::UInteger64(varint), WireType::Varint));
      }
    }
    Type::Int32 => {
      while data.remaining() > 0 {
        let varint = decode_varint(data)?;
        values.push((ProtobufFieldData::Integer32(varint as i32), WireType::Varint));
      }
    }
    Type::Enum => {
      while data.remaining() > 0 {
        let varint = decode_varint(data)?;
        let enum_value = decode_enum(descriptor, descriptors, &field, varint)?;
        values.push((enum_value, WireType::Varint));
      }
    }
    Type::Fixed64 => {
      while data.remaining() >= mem::size_of::<u64>() {
        values.push((ProtobufFieldData::UInteger64(data.get_u64_le()), WireType::SixtyFourBit));
      }
    }
    Type::Fixed32 => {
      while data.remaining() >= mem::size_of::<u32>() {
        values.push((ProtobufFieldData::UInteger32(data.get_u32_le()), WireType::ThirtyTwoBit));
      }
    }
    Type::Uint32 => {
      while data.remaining() > 0 {
        let varint = decode_varint(data)?;
        values.push((ProtobufFieldData::UInteger32(varint as u32), WireType::Varint));
      }
    }
    Type::Sfixed32 => {
      while data.remaining() >= mem::size_of::<i32>() {
        values.push((ProtobufFieldData::Integer32(data.get_i32_le()), WireType::ThirtyTwoBit));
      }
    }
    Type::Sfixed64 => {
      while data.remaining() >= mem::size_of::<i64>() {
        values.push((ProtobufFieldData::Integer64(data.get_i64_le()), WireType::SixtyFourBit));
      }
    }
    Type::Sint32 => {
      while data.remaining() > 0 {
        let varint = decode_varint(data)?;
        let value = varint as u32;
        values.push((ProtobufFieldData::Integer32(((value >> 1) as i32) ^ (-((value & 1) as i32))), WireType::Varint));
      }
    }
    Type::Sint64 => {
      while data.remaining() > 0 {
        let varint = decode_varint(data)?;
        values.push((ProtobufFieldData::Integer64(((varint >> 1) as i64) ^ (-((varint & 1) as i64))), WireType::Varint));
      }
    }
    _ => return Err(anyhow!("Field type {:?} can not be packed", t))
  };

  if data.is_empty() {
    Ok(values)
  } else {
    Err(anyhow!("Failed to decode packed repeated field, there was still {} bytes in the buffer", data.remaining()))
  }
}

fn find_field_descriptor(field_num: i32, descriptor: &DescriptorProto) -> anyhow::Result<FieldDescriptorProto> {
  descriptor.field.iter().find(|field| {
    if let Some(num)  = field.number {
      num == field_num
    } else {
      false
    }
  })
    .cloned()
    .ok_or_else(|| anyhow!("Did not find a field with number {} in the descriptor", field_num))
}

#[cfg(test)]
mod tests {
  use base64::Engine;
  use base64::engine::general_purpose::STANDARD as BASE64;
  use bytes::{BufMut, Bytes, BytesMut};
  use expectest::prelude::*;
  use pact_plugin_driver::proto::InitPluginRequest;
  use prost::encoding::WireType;
  use prost::Message;
  use prost_types::{DescriptorProto, EnumDescriptorProto, EnumValueDescriptorProto, FileDescriptorSet};

  use crate::{
    bool_field_descriptor,
    bytes_field_descriptor,
    enum_field_descriptor,
    f32_field_descriptor,
    f64_field_descriptor,
    i32_field_descriptor,
    i64_field_descriptor,
    message_field_descriptor,
    string_field_descriptor,
    u32_field_descriptor,
    u64_field_descriptor
  };
  use crate::message_decoder::{decode_message, ProtobufFieldData};
  use crate::protobuf::tests::DESCRIPTOR_WITH_ENUM_BYTES;
  use crate::message_builder::tests::REPEATED_ENUM_DESCRIPTORS;

  const FIELD_1_MESSAGE: [u8; 2] = [8, 1];
  const FIELD_2_MESSAGE: [u8; 2] = [16, 55];
  const FIELD_5_MESSAGE: [u8; 3] = [0b101000, 0b10110011, 0b101011];

  #[test]
  fn decode_boolean() {
    let mut buffer = Bytes::from_static(&FIELD_1_MESSAGE);
    let descriptor = DescriptorProto {
      name: Some("TestMessage".to_string()),
      field: vec![ bool_field_descriptor!("bool_field", 1) ],
      extension: vec![],
      nested_type: vec![],
      enum_type: vec![],
      extension_range: vec![],
      oneof_decl: vec![],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![]
    };

    let result = decode_message(&mut buffer, &descriptor, &FileDescriptorSet{ file: vec![] }).unwrap();
    expect!(result.len()).to(be_equal_to(1));

    let field_result = result.first().unwrap();

    expect!(field_result.field_num).to(be_equal_to(1));
    expect!(field_result.wire_type).to(be_equal_to(WireType::Varint));
    expect!(&field_result.data).to(be_equal_to(&ProtobufFieldData::Boolean(true)));
  }

  #[test]
  fn decode_int32() {
    let mut buffer = Bytes::from_static(&FIELD_2_MESSAGE);
    let descriptor = DescriptorProto {
      name: Some("TestMessage".to_string()),
      field: vec![
        prost_types::FieldDescriptorProto {
          name: Some("field_1".to_string()),
          number: Some(2),
          label: Some(prost_types::field_descriptor_proto::Label::Optional as i32),
          r#type: Some(prost_types::field_descriptor_proto::Type::Int32 as i32),
          type_name: Some("Int32".to_string()),
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
      reserved_name: vec![]
    };

    let result = decode_message(&mut buffer, &descriptor, &FileDescriptorSet{ file: vec![] }).unwrap();
    expect!(result.len()).to(be_equal_to(1));

    let field_result = result.first().unwrap();

    expect!(field_result.field_num).to(be_equal_to(2));
    expect!(field_result.wire_type).to(be_equal_to(WireType::Varint));
    expect!(&field_result.data).to(be_equal_to(&ProtobufFieldData::Integer32(55)));
  }

  #[test]
  fn decode_uint64() {
    let mut buffer = Bytes::from_static(&FIELD_5_MESSAGE);
    let descriptor = DescriptorProto {
      name: Some("TestMessage".to_string()),
      field: vec![
        prost_types::FieldDescriptorProto {
          name: Some("field_1".to_string()),
          number: Some(5),
          label: Some(prost_types::field_descriptor_proto::Label::Optional as i32),
          r#type: Some(prost_types::field_descriptor_proto::Type::Uint64 as i32),
          type_name: Some("Uint64".to_string()),
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
      reserved_name: vec![]
    };

    let result = decode_message(&mut buffer, &descriptor, &FileDescriptorSet{ file: vec![] }).unwrap();
    expect!(result.len()).to(be_equal_to(1));

    let field_result = result.first().unwrap();

    expect!(field_result.field_num).to(be_equal_to(5));
    expect!(field_result.wire_type).to(be_equal_to(WireType::Varint));
    expect!(&field_result.data).to(be_equal_to(&ProtobufFieldData::UInteger64(5555)));
  }

  #[test]
  fn decode_enum() {
    let mut buffer = Bytes::from_static(&FIELD_2_MESSAGE);
    let enum_descriptor = EnumDescriptorProto {
      name: Some("ContentTypeHint".to_string()),
      value: vec![
        EnumValueDescriptorProto {
          name: Some("DEFAULT".to_string()),
          number: Some(0),
          options: None
        },
        EnumValueDescriptorProto {
          name: Some("TEXT".to_string()),
          number: Some(55),
          options: None
        },
        EnumValueDescriptorProto {
          name: Some("BINARY".to_string()),
          number: Some(66),
          options: None
        }
      ],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![]
    };
    let descriptor = DescriptorProto {
      name: Some("TestMessage".to_string()),
      field: vec![
        prost_types::FieldDescriptorProto {
          name: Some("field_1".to_string()),
          number: Some(2),
          label: Some(prost_types::field_descriptor_proto::Label::Optional as i32),
          r#type: Some(prost_types::field_descriptor_proto::Type::Enum as i32),
          type_name: Some("ContentTypeHint".to_string()),
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
      enum_type: vec![ enum_descriptor.clone() ],
      extension_range: vec![],
      oneof_decl: vec![],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![]
    };

    let result = decode_message(&mut buffer, &descriptor, &FileDescriptorSet{ file: vec![] }).unwrap();
    expect!(result.len()).to(be_equal_to(1));

    let field_result = result.first().unwrap();

    expect!(field_result.field_num).to(be_equal_to(2));
    expect!(field_result.wire_type).to(be_equal_to(WireType::Varint));
    expect!(&field_result.data).to(be_equal_to(&ProtobufFieldData::Enum(55, enum_descriptor)));
  }

  #[test]
  fn decode_f32() {
    let f_value: f32 = 12.34;
    let mut buffer = BytesMut::new();
    buffer.put_u8(21);
    buffer.put_f32_le(f_value);

    let descriptor = DescriptorProto {
      name: Some("TestMessage".to_string()),
      field: vec![
        prost_types::FieldDescriptorProto {
          name: Some("field_1".to_string()),
          number: Some(2),
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

    let result = decode_message(&mut buffer.freeze(), &descriptor, &FileDescriptorSet{ file: vec![] }).unwrap();
    expect!(result.len()).to(be_equal_to(1));

    let field_result = result.first().unwrap();

    expect!(field_result.field_num).to(be_equal_to(2));
    expect!(field_result.wire_type).to(be_equal_to(WireType::ThirtyTwoBit));
    expect!(&field_result.data).to(be_equal_to(&ProtobufFieldData::Float(12.34)));
  }

  #[test]
  fn decode_f64() {
    let f_value: f64 = 12.34;
    let mut buffer = BytesMut::new();
    buffer.put_u8(17);
    buffer.put_f64_le(f_value);

    let descriptor = DescriptorProto {
      name: Some("TestMessage".to_string()),
      field: vec![
        prost_types::FieldDescriptorProto {
          name: Some("field_1".to_string()),
          number: Some(2),
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

    let result = decode_message(&mut buffer, &descriptor, &FileDescriptorSet{ file: vec![] }).unwrap();
    expect!(result.len()).to(be_equal_to(1));

    let field_result = result.first().unwrap();

    expect!(field_result.field_num).to(be_equal_to(2));
    expect!(field_result.wire_type).to(be_equal_to(WireType::SixtyFourBit));
    expect!(&field_result.data).to(be_equal_to(&ProtobufFieldData::Double(12.34)));
  }

  #[test]
  fn decode_string() {
    let str_data = "this is a string!";
    let mut buffer = BytesMut::new();
    buffer.put_u8(10);
    buffer.put_u8(str_data.len() as u8);
    buffer.put_slice(str_data.as_bytes());

    let descriptor = DescriptorProto {
      name: Some("TestMessage".to_string()),
      field: vec![
        string_field_descriptor!("type", 1)
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

    let result = decode_message(&mut buffer, &descriptor, &FileDescriptorSet{ file: vec![] }).unwrap();
    expect!(result.len()).to(be_equal_to(1));

    let field_result = result.first().unwrap();

    expect!(field_result.field_num).to(be_equal_to(1));
    expect!(field_result.wire_type).to(be_equal_to(WireType::LengthDelimited));
    expect!(&field_result.data).to(be_equal_to(&ProtobufFieldData::String(str_data.to_string())));
  }

  #[test]
  fn decode_message_test() {
    let message = InitPluginRequest {
      implementation: "test".to_string(),
      version: "1.2.3.4".to_string()
    };

    let field1 = string_field_descriptor!("implementation", 1);
    let field2 = string_field_descriptor!("version", 2);
    let message_descriptor = DescriptorProto {
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
    let encoded = message.encode_to_vec();

    let mut buffer = BytesMut::new();
    buffer.put_u8(10);
    buffer.put_u8(encoded.len() as u8);
    buffer.put_slice(&encoded);

    let descriptor = DescriptorProto {
      name: Some("TestMessage".to_string()),
      field: vec![
        message_field_descriptor!("message", 1, "InitPluginRequest")
      ],
      extension: vec![],
      nested_type: vec![
        message_descriptor.clone()
      ],
      enum_type: vec![],
      extension_range: vec![],
      oneof_decl: vec![],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![]
    };

    let result = decode_message(&mut buffer, &descriptor, &FileDescriptorSet{ file: vec![] }).unwrap();
    expect!(result.len()).to(be_equal_to(1));

    let field_result = result.first().unwrap();

    expect!(field_result.field_num).to(be_equal_to(1));
    expect!(field_result.wire_type).to(be_equal_to(WireType::LengthDelimited));
    expect!(&field_result.data).to(be_equal_to(&ProtobufFieldData::Message(encoded, message_descriptor)));
  }

  #[test]
  fn decode_message_with_unknown_field() {
    let message = InitPluginRequest {
      implementation: "test".to_string(),
      version: "1.2.3.4".to_string()
    };

    let field1 = string_field_descriptor!("implementation", 1);
    let message_descriptor = DescriptorProto {
      name: Some("InitPluginRequest".to_string()),
      field: vec![
        field1.clone()
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

    let mut buffer = BytesMut::from(message.encode_to_vec().as_slice());
    let result = decode_message(&mut buffer, &message_descriptor, &FileDescriptorSet{ file: vec![] }).unwrap();
    expect!(result.len()).to(be_equal_to(2));

    let field_result = result.first().unwrap();
    expect!(field_result.field_num).to(be_equal_to(1));
    expect!(field_result.wire_type).to(be_equal_to(WireType::LengthDelimited));
    expect!(&field_result.data).to(be_equal_to(&ProtobufFieldData::String("test".to_string())));

    let field_result = result.get(1).unwrap();
    expect!(field_result.field_num).to(be_equal_to(2));
    expect!(field_result.wire_type).to(be_equal_to(WireType::LengthDelimited));
    expect!(field_result.data.type_name()).to(be_equal_to("Unknown"));
  }

  #[test]
  fn default_field_value_test_boolean() {
    let descriptor = bool_field_descriptor!("bool_field", 1);
    expect!(ProtobufFieldData::Boolean(true).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::Boolean(false)));

    let descriptor = prost_types::FieldDescriptorProto {
      default_value: Some("true".to_string()),
      .. bool_field_descriptor!("bool_field", 1)
    };
    expect!(ProtobufFieldData::Boolean(true).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::Boolean(true)));

    let descriptor = prost_types::FieldDescriptorProto {
      default_value: Some("false".to_string()),
      .. bool_field_descriptor!("bool_field", 1)
    };
    expect!(ProtobufFieldData::Boolean(true).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::Boolean(false)));
  }

  #[test]
  fn default_field_value_test_string() {
    let descriptor = string_field_descriptor!("field", 1);
    expect!(ProtobufFieldData::String("true".to_string()).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::String("".to_string())));

    let descriptor = prost_types::FieldDescriptorProto {
      default_value: Some("true".to_string()),
      .. string_field_descriptor!("field", 1)
    };
    expect!(ProtobufFieldData::String("other".to_string()).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::String("true".to_string())));
  }

  #[test]
  fn default_field_value_test_u32() {
    let descriptor = u32_field_descriptor!("field", 1);
    expect!(ProtobufFieldData::UInteger32(123).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::UInteger32(0)));

    let descriptor = prost_types::FieldDescriptorProto {
      default_value: Some("100".to_string()),
      .. u32_field_descriptor!("field", 1)
    };
    expect!(ProtobufFieldData::UInteger32(123).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::UInteger32(100)));

    let descriptor = prost_types::FieldDescriptorProto {
      default_value: Some("sdsd".to_string()),
      .. u32_field_descriptor!("field", 1)
    };
    expect!(ProtobufFieldData::UInteger32(123).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::UInteger32(0)));
  }

  #[test]
  fn default_field_value_test_i32() {
    let descriptor = i32_field_descriptor!("field", 1);
    expect!(ProtobufFieldData::Integer32(123).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::Integer32(0)));

    let descriptor = prost_types::FieldDescriptorProto {
      default_value: Some("100".to_string()),
      .. i32_field_descriptor!("field", 1)
    };
    expect!(ProtobufFieldData::Integer32(123).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::Integer32(100)));

    let descriptor = prost_types::FieldDescriptorProto {
      default_value: Some("sdsd".to_string()),
      .. i32_field_descriptor!("field", 1)
    };
    expect!(ProtobufFieldData::Integer32(123).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::Integer32(0)));
  }

  #[test]
  fn default_field_value_test_u64() {
    let descriptor = u64_field_descriptor!("field", 1);
    expect!(ProtobufFieldData::UInteger64(123).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::UInteger64(0)));

    let descriptor = prost_types::FieldDescriptorProto {
      default_value: Some("100".to_string()),
      .. u64_field_descriptor!("field", 1)
    };
    expect!(ProtobufFieldData::UInteger64(123).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::UInteger64(100)));

    let descriptor = prost_types::FieldDescriptorProto {
      default_value: Some("sdsd".to_string()),
      .. u64_field_descriptor!("field", 1)
    };
    expect!(ProtobufFieldData::UInteger64(123).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::UInteger64(0)));
  }

  #[test]
  fn default_field_value_test_i64() {
    let descriptor = i64_field_descriptor!("field", 1);
    expect!(ProtobufFieldData::Integer64(123).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::Integer64(0)));

    let descriptor = prost_types::FieldDescriptorProto {
      default_value: Some("100".to_string()),
      .. i64_field_descriptor!("field", 1)
    };
    expect!(ProtobufFieldData::Integer64(123).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::Integer64(100)));

    let descriptor = prost_types::FieldDescriptorProto {
      default_value: Some("sdsd".to_string()),
      .. i64_field_descriptor!("field", 1)
    };
    expect!(ProtobufFieldData::Integer64(123).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::Integer64(0)));
  }

  #[test]
  fn default_field_value_test_f32() {
    let descriptor = f32_field_descriptor!("field", 1);
    expect!(ProtobufFieldData::Float(123.0).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::Float(0.0)));

    let descriptor = prost_types::FieldDescriptorProto {
      default_value: Some("100".to_string()),
      .. f32_field_descriptor!("field", 1)
    };
    expect!(ProtobufFieldData::Float(123.0).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::Float(100.0)));

    let descriptor = prost_types::FieldDescriptorProto {
      default_value: Some("sdsd".to_string()),
      .. f32_field_descriptor!("field", 1)
    };
    expect!(ProtobufFieldData::Float(123.0).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::Float(0.0)));
  }

  #[test]
  fn default_field_value_test_f64() {
    let descriptor = f64_field_descriptor!("field", 1);
    expect!(ProtobufFieldData::Double(123.0).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::Double(0.0)));

    let descriptor = prost_types::FieldDescriptorProto {
      default_value: Some("100".to_string()),
      .. f64_field_descriptor!("field", 1)
    };
    expect!(ProtobufFieldData::Double(123.0).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::Double(100.0)));

    let descriptor = prost_types::FieldDescriptorProto {
      default_value: Some("sdsd".to_string()),
      .. f64_field_descriptor!("field", 1)
    };
    expect!(ProtobufFieldData::Double(123.0).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::Double(0.0)));
  }

  #[test]
  fn default_field_value_test_enum() {
    let enum_descriptor = prost_types::EnumDescriptorProto {
      name: Some("EnumValue".to_string()),
      value: vec![
        prost_types::EnumValueDescriptorProto {
          name: Some("OPT1".to_string()),
          number: Some(0),
          options: None
        },
        prost_types::EnumValueDescriptorProto {
          name: Some("OPT2".to_string()),
          number: Some(1),
          options: None
        },
        prost_types::EnumValueDescriptorProto {
          name: Some("OPT3".to_string()),
          number: Some(2),
          options: None
        }
      ],
      options: None,
      reserved_range: vec![],
      reserved_name: vec![]
    };
    let descriptor = enum_field_descriptor!("field", 1, "OPT1");
    expect!(ProtobufFieldData::Enum(2, enum_descriptor.clone()).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::Enum(0, enum_descriptor.clone())));

    let descriptor = prost_types::FieldDescriptorProto {
      default_value: Some("1".to_string()),
      .. enum_field_descriptor!("field", 1, "OPT2")
    };
    expect!(ProtobufFieldData::Enum(2, enum_descriptor.clone()).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::Enum(1, enum_descriptor.clone())));

    let descriptor = prost_types::FieldDescriptorProto {
      default_value: Some("sdsd".to_string()),
      .. enum_field_descriptor!("field", 1, "OPT2")
    };
    expect!(ProtobufFieldData::Enum(2, enum_descriptor.clone()).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::Enum(0, enum_descriptor.clone())));
  }

  #[test]
  fn default_field_value_test_bytes() {
    let descriptor = bytes_field_descriptor!("field", 1);
    expect!(ProtobufFieldData::Bytes(vec![1, 2, 3, 4]).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::Bytes(vec![])));

    let descriptor = prost_types::FieldDescriptorProto {
      default_value: Some("true".to_string()),
      .. bytes_field_descriptor!("field", 1)
    };
    expect!(ProtobufFieldData::Bytes(vec![1, 2, 3, 4]).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::Bytes(vec![116, 114, 117, 101])));
  }

  #[test]
  fn default_field_value_test_message() {
    let field1 = string_field_descriptor!("implementation", 1);
    let field2 = string_field_descriptor!("version", 2);
    let message_descriptor = DescriptorProto {
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
    let descriptor = message_field_descriptor!("field", 1, "InitPluginRequest");
    expect!(ProtobufFieldData::Message(vec![1, 2, 3, 4], message_descriptor.clone()).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::Message(vec![], message_descriptor.clone())));

    let descriptor = prost_types::FieldDescriptorProto {
      default_value: Some("true".to_string()),
      .. message_field_descriptor!("field", 1, "InitPluginRequest")
    };
    expect!(ProtobufFieldData::Message(vec![1, 2, 3, 4], message_descriptor.clone()).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::Message(vec![], message_descriptor.clone())));
  }

  #[test]
  fn default_field_value_test_unknown() {
    let descriptor = bytes_field_descriptor!("field", 1);
    expect!(ProtobufFieldData::Unknown(vec![1, 2, 3, 4]).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::Unknown(vec![])));

    let descriptor = prost_types::FieldDescriptorProto {
      default_value: Some("true".to_string()),
      .. bytes_field_descriptor!("field", 1)
    };
    expect!(ProtobufFieldData::Unknown(vec![1, 2, 3, 4]).default_field_value(&descriptor)).to(be_equal_to(ProtobufFieldData::Unknown(vec![])));
  }

  #[test]
  fn decode_packed_field() {
    let f_value: f32 = 12.0;
    let f_value2: f32 = 9.0;
    let mut buffer = BytesMut::new();
    buffer.put_u8(10);
    buffer.put_u8(8);
    buffer.put_f32_le(f_value);
    buffer.put_f32_le(f_value2);

    let descriptor = DescriptorProto {
      name: Some("PackedFieldMessage".to_string()),
      field: vec![
        prost_types::FieldDescriptorProto {
          name: Some("field_1".to_string()),
          number: Some(1),
          label: Some(prost_types::field_descriptor_proto::Label::Repeated as i32),
          r#type: Some(prost_types::field_descriptor_proto::Type::Float as i32),
          type_name: Some("Float".to_string()),
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
      reserved_name: vec![]
    };

    let result = decode_message(&mut buffer, &descriptor, &FileDescriptorSet{ file: vec![] }).unwrap();
    expect!(result.len()).to(be_equal_to(2));

    let field_result = result.first().unwrap();

    expect!(field_result.field_num).to(be_equal_to(1));
    expect!(field_result.wire_type).to(be_equal_to(WireType::ThirtyTwoBit));
    expect!(&field_result.data).to(be_equal_to(&ProtobufFieldData::Float(12.0)));
  }

  #[test_log::test]
  fn decode_message_with_global_enum_field() {
    let bytes: &[u8] = &DESCRIPTOR_WITH_ENUM_BYTES;
    let buffer = Bytes::from(bytes);
    let fds: FileDescriptorSet = FileDescriptorSet::decode(buffer).unwrap();
    let main_descriptor = fds.file.iter()
      .find(|fd| fd.name.clone().unwrap_or_default() == "area_calculator.proto")
      .unwrap();
    let message_descriptor = main_descriptor.message_type.iter()
      .find(|md| md.name.clone().unwrap_or_default() == "Rectangle").unwrap();
    let enum_proto = main_descriptor.enum_type.first().unwrap();

    let message_bytes: &[u8] = &[13, 0, 0, 64, 64, 21, 0, 0, 128, 64, 40, 1];
    let mut buffer = Bytes::from(message_bytes);
    let result = decode_message(&mut buffer, &message_descriptor, &fds).unwrap();
    expect!(result.len()).to(be_equal_to(3));

    let field_result = result.last().unwrap();

    expect!(field_result.field_num).to(be_equal_to(5));
    expect!(field_result.wire_type).to(be_equal_to(WireType::Varint));
    expect!(&field_result.data).to(be_equal_to(&ProtobufFieldData::Enum(1, enum_proto.clone())));
  }

  #[test_log::test]
  fn decode_message_with_repeated_enum_field() {
    let bytes = BASE64.decode(REPEATED_ENUM_DESCRIPTORS).unwrap();
    let buffer = Bytes::from(bytes);
    let fds: FileDescriptorSet = FileDescriptorSet::decode(buffer).unwrap();
    let main_descriptor = fds.file.iter()
      .find(|fd| fd.name.clone().unwrap_or_default() == "repeated_enum.proto")
      .unwrap();
    let message_descriptor = main_descriptor.message_type.iter()
      .find(|md| md.name.clone().unwrap_or_default() == "BrokenSampleRequest").unwrap();
    let enum_proto = message_descriptor.enum_type.first().unwrap();

    let message_bytes: &[u8] = &[10, 3, 2, 0, 1];
    let mut buffer = Bytes::from(message_bytes);
    let result = decode_message(&mut buffer, &message_descriptor, &fds).unwrap();
    expect!(result.len()).to(be_equal_to(3));

    expect!(result[0].field_num).to(be_equal_to(1));
    expect!(result[0].wire_type).to(be_equal_to(WireType::Varint));
    expect!(&result[0].data).to(be_equal_to(&ProtobufFieldData::Enum(2, enum_proto.clone())));
    expect!(result[1].field_num).to(be_equal_to(1));
    expect!(result[1].wire_type).to(be_equal_to(WireType::Varint));
    expect!(&result[1].data).to(be_equal_to(&ProtobufFieldData::Enum(0, enum_proto.clone())));
    expect!(result[2].field_num).to(be_equal_to(1));
    expect!(result[2].wire_type).to(be_equal_to(WireType::Varint));
    expect!(&result[2].data).to(be_equal_to(&ProtobufFieldData::Enum(1, enum_proto.clone())));
  }

  // Issue #53
  #[test_log::test]
  fn decode_message_with_unknown_fields() {
    let descriptors = "CtYCChBuZXdfZmllbGRzLnByb3RvEglwYWN0aXNzdWUiuwEKD0dldFVzZXJSZXNwb25z\
    ZRIOCgJpZBgBIAEoCVICaWQSIQoMZGlzcGxheV9uYW1lGAIgASgJUgtkaXNwbGF5TmFtZRIdCgpmaXJzdF9uYW1lGAMgA\
    SgJUglmaXJzdE5hbWUSGAoHc3VybmFtZRgEIAEoCVIHc3VybmFtZRIdCgpjcmVhdGVkX2F0GAUgASgJUgljcmVhdGVkQX\
    QSHQoKdXBkYXRlZF9hdBgGIAEoCVIJdXBkYXRlZEF0IiAKDkdldFVzZXJSZXF1ZXN0Eg4KAmlkGAEgASgJUgJpZDJPCgt\
    Vc2VyU2VydmljZRJACgdHZXRVc2VyEhkucGFjdGlzc3VlLkdldFVzZXJSZXF1ZXN0GhoucGFjdGlzc3VlLkdldFVzZXJSZ\
    XNwb25zZWIGcHJvdG8z";
    let bytes = BASE64.decode(descriptors).unwrap();
    let buffer = Bytes::from(bytes);
    let fds: FileDescriptorSet = FileDescriptorSet::decode(buffer).unwrap();
    let main_descriptor = fds.file.iter()
      .find(|fd| fd.name.clone().unwrap_or_default() == "new_fields.proto")
      .unwrap();
    let message_descriptor = main_descriptor.message_type.iter()
      .find(|md| md.name.clone().unwrap_or_default() == "GetUserResponse").unwrap();

    // buf: b"\n\x041234\x12\x0cElla Streich\x1a\x04Ella:\x07StreichB\x14Ella.Streich@test.ioH\x01", len: 59
    let message_bytes: &[u8] = &[10, 4, 49, 50, 51, 52, 18, 12, 69, 108, 108, 97, 32, 83, 116,
      114, 101, 105, 99, 104, 26, 4, 69, 108, 108, 97, 58, 7, 83, 116, 114, 101, 105, 99, 104, 66,
      20, 69, 108, 108, 97, 46, 83, 116, 114, 101, 105, 99, 104, 64, 116, 101, 115, 116, 46, 105,
      111, 72, 1];
    let mut buffer = Bytes::from(message_bytes);
    let result = decode_message(&mut buffer, &message_descriptor, &fds).unwrap();

    expect!(result.len()).to(be_equal_to(6));

    let first_field = &result[0];
    expect!(first_field.field_num).to(be_equal_to(1));
    expect!(first_field.wire_type).to(be_equal_to(WireType::LengthDelimited));
    expect!(first_field.data.clone()).to(be_equal_to(ProtobufFieldData::String("1234".to_string())));

    let field = &result[1];
    expect!(field.field_num).to(be_equal_to(2));
    expect!(field.wire_type).to(be_equal_to(WireType::LengthDelimited));
    expect!(field.data.clone()).to(be_equal_to(ProtobufFieldData::String("Ella Streich".to_string())));

    let field = &result[2];
    expect!(field.field_num).to(be_equal_to(3));
    expect!(field.wire_type).to(be_equal_to(WireType::LengthDelimited));
    expect!(field.data.clone()).to(be_equal_to(ProtobufFieldData::String("Ella".to_string())));

    // 7 bytes string "Streich"
    let field = &result[3];
    expect!(field.field_num).to(be_equal_to(7));
    expect!(field.wire_type).to(be_equal_to(WireType::LengthDelimited));
    expect!(field.data.clone()).to(be_equal_to(ProtobufFieldData::Unknown(vec![7, 83, 116, 114, 101, 105, 99, 104])));

    // 20 bytes string "Ella.Streich@test.io"
    let field = &result[4];
    expect!(field.field_num).to(be_equal_to(8));
    expect!(field.wire_type).to(be_equal_to(WireType::LengthDelimited));
    expect!(field.data.clone()).to(be_equal_to(ProtobufFieldData::Unknown(vec![
      20, 69, 108, 108, 97, 46, 83, 116, 114, 101, 105, 99, 104, 64,
      116, 101, 115, 116, 46, 105, 111])));

    // 1 byte bool true
    let field = &result[5];
    expect!(field.field_num).to(be_equal_to(9));
    expect!(field.wire_type).to(be_equal_to(WireType::Varint));
    expect!(field.data.clone()).to(be_equal_to(ProtobufFieldData::Unknown(vec![1])));
  }
}
