//! Decoder for encoded Protobuf messages using the descriptors

use std::str::from_utf8;

use anyhow::anyhow;
use bytes::{Buf, BytesMut};
use prost::encoding::{decode_key, decode_varint, WireType};
use prost_types::{DescriptorProto, EnumDescriptorProto, FieldDescriptorProto};
use prost_types::field_descriptor_proto::Type;

/// Decoded Protobuf field
#[derive(Clone, Debug, PartialEq)]
pub struct ProtobufField {
  /// Field number
  pub field_num: u32,
  /// Wire type for the field
  pub wire_type: WireType,
  /// Field data
  pub data: ProtobufFieldData
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
  Message(Vec<u8>, DescriptorProto)
}

/// Decodes the Protobuf message using the descriptors
pub fn decode_message<B>(buffer: &mut B, descriptor: &DescriptorProto) -> anyhow::Result<Vec<ProtobufField>>
  where B: Buf {
  let mut fields = vec![];

  while buffer.has_remaining() {
    let (field_num, wire_type) = decode_key(buffer)?;
    let field_descriptor = find_field_descriptor(field_num as i32, descriptor)?;

    let data = match wire_type {
      WireType::Varint => {
        let varint = decode_varint(buffer)?;
        let t: Type = field_descriptor.r#type();
        match t {
          Type::Int64 => ProtobufFieldData::Integer64(varint as i64),
          Type::Uint64 => ProtobufFieldData::UInteger64(varint),
          Type::Int32 => ProtobufFieldData::Integer32(varint as i32),
          Type::Bool => ProtobufFieldData::Boolean(varint > 0),
          Type::Uint32 => ProtobufFieldData::UInteger32(varint as u32),
          Type::Enum => {
            let enum_proto = descriptor.enum_type.iter()
              .find(|enum_type| enum_type.name.clone() == field_descriptor.type_name)
              .ok_or_else(|| anyhow!("Did not find the enum {:?} for the field {} in the Protobuf descriptor", field_descriptor.type_name, field_num))?;
            ProtobufFieldData::Enum(varint as i32, enum_proto.clone())
          },
          Type::Sint32 => {
            let value = varint as u32;
            ProtobufFieldData::Integer32(((value >> 1) as i32) ^ (-((value & 1) as i32)))
          },
          Type::Sint64 => ProtobufFieldData::Integer64(((varint >> 1) as i64) ^ (-((varint & 1) as i64))),
          _ => return Err(anyhow!("Field type {:?} is not a valid varint type", t))
        }
      }
      WireType::SixtyFourBit => {
        let t: Type = field_descriptor.r#type();
        match t {
          Type::Double => ProtobufFieldData::Double(buffer.get_f64_le()),
          Type::Fixed64 => ProtobufFieldData::UInteger64(buffer.get_u64_le()),
          Type::Sfixed64 => ProtobufFieldData::Integer64(buffer.get_i64_le()),
          _ => return Err(anyhow!("Field type {:?} is not a valid fixed 64 bit type", t))
        }
      }
      WireType::LengthDelimited => {
        let data_length = decode_varint(buffer)?;
        let data_buffer = if buffer.remaining() >= data_length as usize {
          buffer.copy_to_bytes(data_length as usize)
        } else {
          return Err(anyhow!("Insufficient data remaining to read {} bytes for field {}", data_length, field_num));
        };
        let t: Type = field_descriptor.r#type();
        match t {
          Type::String => ProtobufFieldData::String(from_utf8(&data_buffer)?.to_string()),
          Type::Message => {
            let message_proto = descriptor.nested_type.iter()
              .find(|message_descriptor|  message_descriptor.name == field_descriptor.type_name)
              .ok_or_else(|| anyhow!("Did not find the embedded message {:?} for the field {} in the Protobuf descriptor", field_descriptor.type_name, field_num))?;
            ProtobufFieldData::Message(data_buffer.to_vec(), message_proto.clone())
          }
          Type::Bytes => ProtobufFieldData::Bytes(data_buffer.to_vec()),
          _ => return Err(anyhow!("Field type {:?} is not a valid length-delimited type", t))
        }
      }
      WireType::ThirtyTwoBit => {
        let t: Type = field_descriptor.r#type();
        match t {
          Type::Float => ProtobufFieldData::Float(buffer.get_f32_le()),
          Type::Fixed32 => ProtobufFieldData::UInteger32(buffer.get_u32_le()),
          Type::Sfixed32 => ProtobufFieldData::Integer32(buffer.get_i32_le()),
          _ => return Err(anyhow!("Field type {:?} is not a valid fixed 32 bit type", t))
        }
      }
      _ => return Err(anyhow!("Messages with {:?} wire type fields are not supported", wire_type))
    };

    fields.push(ProtobufField {
      field_num,
      wire_type,
      data
    });
  }

  Ok(fields)
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
  use bytes::{BufMut, Bytes, BytesMut};
  use expectest::prelude::*;
  use pact_plugin_driver::proto::InitPluginRequest;
  use prost::encoding::WireType;
  use prost::Message;
  use prost_types::{DescriptorProto, EnumDescriptorProto, EnumValueDescriptorProto};

  use crate::{bool_field_descriptor, message_field_descriptor, string_field_descriptor};
  use crate::message_decoder::{decode_message, ProtobufFieldData};

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

    let result = decode_message(&mut buffer, &descriptor).unwrap();
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

    let result = decode_message(&mut buffer, &descriptor).unwrap();
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

    let result = decode_message(&mut buffer, &descriptor).unwrap();
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

    let result = decode_message(&mut buffer, &descriptor).unwrap();
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

    let result = decode_message(&mut buffer.freeze(), &descriptor).unwrap();
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

    let result = decode_message(&mut buffer, &descriptor).unwrap();
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

    let result = decode_message(&mut buffer, &descriptor).unwrap();
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

    let result = decode_message(&mut buffer, &descriptor).unwrap();
    expect!(result.len()).to(be_equal_to(1));

    let field_result = result.first().unwrap();

    expect!(field_result.field_num).to(be_equal_to(1));
    expect!(field_result.wire_type).to(be_equal_to(WireType::LengthDelimited));
    expect!(&field_result.data).to(be_equal_to(&ProtobufFieldData::Message(encoded, message_descriptor)));
  }
}
