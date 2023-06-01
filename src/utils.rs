//! Shared utilities

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;
use std::panic::RefUnwindSafe;

use anyhow::anyhow;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use bytes::{Bytes, BytesMut};
use field_descriptor_proto::Type;
use maplit::hashmap;
use pact_models::json_utils::json_to_string;
use pact_models::pact::load_pact_from_json;
use pact_models::prelude::v4::V4Pact;
use pact_models::v4::interaction::V4Interaction;
use prost::Message;
use prost_types::{
  DescriptorProto,
  EnumDescriptorProto,
  field_descriptor_proto,
  FieldDescriptorProto,
  FileDescriptorProto,
  FileDescriptorSet,
  MethodDescriptorProto,
  ServiceDescriptorProto,
  Value
};
use prost_types::field_descriptor_proto::Label;
use prost_types::value::Kind;
use serde_json::json;
use tracing::{debug, error, trace, warn};

use crate::message_decoder::{decode_message, ProtobufField, ProtobufFieldData};

/// Return the last name in a dot separated string
pub fn last_name(entry_type_name: &str) -> &str {
  entry_type_name.split('.').last().unwrap_or(entry_type_name)
}

/// Search for a message by type name in all the descriptors
pub fn find_message_type_by_name(message_name: &str, descriptors: &FileDescriptorSet) -> anyhow::Result<(DescriptorProto, FileDescriptorProto)> {
  descriptors.file.iter()
    .map(|descriptor| {
      find_message_type_in_file_descriptor(message_name, descriptor).map(|ds| (ds, descriptor)).ok()
    })
    .find(|result| result.is_some())
    .flatten()
    .map(|(m, f)| (m, f.clone()))
    .ok_or_else(|| anyhow!("Did not find a message type '{}' in the descriptors", message_name))
}

/// Search for a message by type name in the file descriptor
pub fn find_message_type_in_file_descriptor(message_name: &str, descriptor: &FileDescriptorProto) -> anyhow::Result<DescriptorProto> {
  descriptor.message_type.iter()
    .find(|message| message.name.clone().unwrap_or_default() == message_name)
    .cloned()
    .ok_or_else(|| anyhow!("Did not find a message type '{}' in the file descriptor '{}'",
      message_name, descriptor.name.as_deref().unwrap_or("unknown")))
}

/// Search for a message by type name in the file descriptor, and if not found, search in all the
/// descriptors
pub fn find_message_type_in_file_descriptors(
  message_type: &str,
  file_descriptor: &FileDescriptorProto,
  all_descriptors: &HashMap<String, &FileDescriptorProto>
) -> anyhow::Result<DescriptorProto> {
  find_message_type_in_file_descriptor(message_type, file_descriptor)
    .or_else(|_| {
      all_descriptors.values()
        .find_map(|fd| find_message_type_in_file_descriptor(message_type, fd).ok())
        .ok_or_else(|| anyhow!("Did not find a message type '{}' in any of the file descriptors", message_type))
    })
}

/// If the field is a map field. A field will be a map field if it is a repeated field, the field
/// type is a message and the nested type has the map flag set on the message options.
pub fn is_map_field(message_descriptor: &DescriptorProto, field: &FieldDescriptorProto) -> bool {
  if field.label() == Label::Repeated && field.r#type() == Type::Message {
    match find_nested_type(message_descriptor, field) {
      Some(nested) => match nested.options {
        None => false,
        Some(options) => options.map_entry.unwrap_or(false)
      },
      None => false
    }
  } else {
    false
  }
}

/// Returns the nested descriptor for this field.
pub fn find_nested_type(message_descriptor: &DescriptorProto, field: &FieldDescriptorProto) -> Option<DescriptorProto> {
  trace!(">> find_nested_type({:?}, {:?}, {:?}, {:?})", message_descriptor.name, field.name, field.r#type(), field.type_name);
  if field.r#type() == Type::Message {
    let type_name = field.type_name.clone().unwrap_or_default();
    let message_type = last_name(type_name.as_str());
    trace!("find_nested_type: Looking for nested type '{}'", message_type);
    message_descriptor.nested_type.iter().find(|nested| {
      trace!("find_nested_type: type = '{:?}'", nested.name);
      nested.name.clone().unwrap_or_default() == message_type
    }).cloned()
  } else {
    None
  }
}

/// Return the hexadecimal representation for the bytes
pub(crate) fn as_hex(data: &[u8]) -> String {
  let mut buffer = String::with_capacity(data.len() * 2);

  for b in data {
    let _ = write!(&mut buffer, "{:02x}", b);
  }

  buffer
}

/// Create a string from the byte array for rendering/displaying
pub(crate) fn display_bytes(data: &[u8]) -> String {
  if data.len() <= 16 {
    as_hex(data)
  } else {
    format!("{}... ({} bytes)", as_hex(&data[0..16]), data.len())
  }
}

/// Look for the message field data with the given name
pub fn find_message_field_by_name(descriptor: &DescriptorProto, field_data: Vec<ProtobufField>, field_name: &str) -> Option<ProtobufField> {
  let field_num = match descriptor.field.iter()
    .find(|f| f.name.clone().unwrap_or_default() == field_name)
    .map(|f| f.number.unwrap_or(-1)) {
    Some(n) => n,
    None => return None
  };

  field_data.iter().find(|d| d.field_num == field_num as u32).cloned()
}

/// If the field is a repeated field
pub fn is_repeated_field(descriptor: &FieldDescriptorProto) -> bool {
  descriptor.label() == Label::Repeated
}

/// Get the name of the enum value
pub fn enum_name(enum_value: i32, descriptor: &EnumDescriptorProto) -> String {
  descriptor.value.iter().find(|v| v.number.unwrap_or(-1) == enum_value)
    .map(|v| v.name.clone().unwrap_or_else(|| format!("enum {}", enum_value)))
    .unwrap_or_else(|| format!("Unknown enum {}", enum_value))
}

/// Find the integer value of the given enum type and name in the message descriptor.
#[tracing::instrument(ret, skip_all, fields(%enum_name, %enum_value))]
pub fn find_enum_value_by_name_in_message(
  enum_types: &[EnumDescriptorProto],
  enum_name: &str,
  enum_value: &str
) -> Option<(i32, EnumDescriptorProto)> {
  trace!(">> find_enum_value_by_name_in_message({}, {})",enum_name, enum_value);
  enum_types.iter()
    .find_map(|enum_descriptor| {
      trace!("find_enum_value_by_name_in_message: enum type = {:?}", enum_descriptor.name);
      if let Some(name) = &enum_descriptor.name {
        if name == last_name(enum_name) {
          enum_descriptor.value.iter().find_map(|val| {
            if let Some(name) = &val.name {
              if name == enum_value {
                val.number.map(|n| (n, enum_descriptor.clone()))
              } else {
                None
              }
            } else {
              None
            }
          })
        } else {
          None
        }
      } else {
        None
      }
    })
}

/// Find the enum type by name in the message descriptor.
#[tracing::instrument(ret, skip_all, fields(%enum_name))]
pub fn find_enum_by_name_in_message(
  enum_types: &[EnumDescriptorProto],
  enum_name: &str
) -> Option<EnumDescriptorProto> {
  trace!(">> find_enum_value_by_name_in_message({})",enum_name);
  enum_types.iter()
    .find_map(|enum_descriptor| {
      trace!("find_enum_by_name_in_message: enum type = {:?}", enum_descriptor.name);
      if let Some(name) = &enum_descriptor.name {
        if name == last_name(enum_name) {
          Some(enum_descriptor.clone())
        } else {
          None
        }
      } else {
        None
      }
    })
}

/// Find the integer value of the given enum type and name in all the descriptors.
#[tracing::instrument(ret, skip_all, fields(%enum_name, %enum_value))]
pub fn find_enum_value_by_name(
  descriptors: &HashMap<String, &FileDescriptorProto>,
  enum_name: &str,
  enum_value: &str
) -> Option<(i32, EnumDescriptorProto)> {
  trace!(">> find_enum_value_by_name({}, {})", enum_name, enum_value);
  let package_names = enum_name.split('.').filter(|v| !v.is_empty()).collect::<Vec<_>>();
  if let Some((_name, package)) = package_names.split_last() {
    let package = package.join(".");
    descriptors.values()
      .find(|fd| fd.package.clone().unwrap_or_default() == package)
      .and_then(|fd| find_enum_value_by_name_in_message(&fd.enum_type, enum_name, enum_value))
  } else {
    None
  }
}

/// Find the given enum type by name in all the descriptors.
#[tracing::instrument(ret, skip_all, fields(%enum_name))]
pub fn find_enum_by_name(
  descriptors: &FileDescriptorSet,
  enum_name: &str
) -> Option<EnumDescriptorProto> {
  trace!(">> find_enum_by_name({})", enum_name);
  let package_names = enum_name.split('.').filter(|v| !v.is_empty()).collect::<Vec<_>>();
  trace!("package_names={:?}", package_names);
  if let Some((_name, package)) = package_names.split_last() {
    let package = package.join(".");
    descriptors.file.iter()
      .find(|fd| {
        if let Some(fd_package) = &fd.package {
          package == fd_package.as_str()
        } else {
          false
        }
      })
      .and_then(|fd| find_enum_by_name_in_message(&fd.enum_type, enum_name))
  } else {
    None
  }
}

/// Convert the message field data into a JSON value
pub fn field_data_to_json(
  field_data: Vec<ProtobufField>,
  descriptor: &DescriptorProto,
  descriptors: &FileDescriptorSet
) -> anyhow::Result<serde_json::Value> {
  let mut object = hashmap!{};

  for field in field_data {
    if let Some(value) = descriptor.field.iter().find(|f| f.number.unwrap_or(-1) as u32 == field.field_num) {
      match &value.name {
        Some(name) => {
          object.insert(name.clone(), match &field.data {
            ProtobufFieldData::String(s) => serde_json::Value::String(s.clone()),
            ProtobufFieldData::Boolean(b) => serde_json::Value::Bool(*b),
            ProtobufFieldData::UInteger32(n) => json!(n),
            ProtobufFieldData::Integer32(n) => json!(n),
            ProtobufFieldData::UInteger64(n) => json!(n),
            ProtobufFieldData::Integer64(n) => json!(n),
            ProtobufFieldData::Float(n) => json!(n),
            ProtobufFieldData::Double(n) => json!(n),
            ProtobufFieldData::Bytes(b) => serde_json::Value::Array(b.iter().map(|v| json!(v)).collect()),
            ProtobufFieldData::Enum(n, descriptor) => serde_json::Value::String(enum_name(*n, descriptor)),
            ProtobufFieldData::Message(b, descriptor) => {
              let mut bytes = BytesMut::from(b.as_slice());
              let message_data = decode_message(&mut bytes, descriptor, descriptors)?;
              field_data_to_json(message_data, descriptor, descriptors)?
            }
            ProtobufFieldData::Unknown(b) => serde_json::Value::Array(b.iter().map(|v| json!(v)).collect())
          });
        }
        None => warn!("Did not get the field name for field number {}", field.field_num)
      }
    } else {
      warn!("Did not find the descriptor for field number {}", field.field_num);
    }
  }

  Ok(serde_json::Value::Object(object.iter().map(|(k, v)| (k.clone(), v.clone())).collect()))
}

/// Parse the JSON string into a V4 Pact model
pub(crate) fn parse_pact_from_request_json(pact_json: &str, source: &str) -> anyhow::Result<V4Pact> {
  // Parse the Pact JSON string into a JSON struct
  let json: serde_json::Value = match serde_json::from_str(pact_json) {
    Ok(json) => json,
    Err(err) => {
      error!("Failed to parse Pact JSON: {}", err);
      return Err(anyhow!("Failed to parse Pact JSON: {}", err));
    }
  };

  // Load the Pact model from the JSON
  match load_pact_from_json(source, &json) {
    Ok(pact) => match pact.as_v4_pact() {
      Ok(pact) => Ok(pact),
      Err(err) => {
        error!("Failed to parse Pact JSON, not a V4 Pact: {}", err);
        Err(anyhow!("Failed to parse Pact JSON, not a V4 Pact: {}", err))
      }
    },
    Err(err) => {
      error!("Failed to parse Pact JSON to a V4 Pact: {}", err);
      Err(anyhow!("Failed to parse Pact JSON: {}", err))
    }
  }
}

/// Lookup up the interaction in the Pact file, given the ID
pub(crate) fn lookup_interaction_by_id<'a>(
  interaction_key: &str,
  pact: &'a V4Pact
) -> anyhow::Result<&'a Box<dyn V4Interaction + Send + Sync + RefUnwindSafe>> {
  match pact.interactions.iter()
    .find(|i| i.key().unwrap_or_default() == interaction_key) {
    Some(interaction) => Ok(interaction),
    None => Err(anyhow!("Did not find interaction with key '{}' in the Pact", interaction_key))
  }
}

pub(crate) fn lookup_interaction_config(interaction: &dyn V4Interaction) -> Option<HashMap<String, serde_json::Value>> {
  interaction.plugin_config().iter()
    .find_map(|(key, value)| {
      if key.as_str() == "protobuf" {
        Some(value.clone())
      } else {
        None
      }
    })
}

/// Returns the service descriptors for the given interaction
pub(crate) fn lookup_service_descriptors_for_interaction(
  interaction: &dyn V4Interaction,
  pact: &V4Pact
) -> anyhow::Result<(FileDescriptorSet, ServiceDescriptorProto, MethodDescriptorProto, String)> {
  let interaction_config = lookup_interaction_config(interaction)
    .ok_or_else(|| anyhow!("Interaction does not have any Protobuf configuration"))?;
  let descriptor_key = interaction_config.get("descriptorKey")
    .map(json_to_string)
    .ok_or_else(|| anyhow!("Interaction descriptorKey was missing in Pact file"))?;
  let service = interaction_config.get("service")
    .map(json_to_string)
    .ok_or_else(|| anyhow!("Interaction gRPC service was missing in Pact file"))?;
  let (service_name, method_name) = service.split_once('/')
    .ok_or_else(|| anyhow!("Service name '{}' is not valid, it should be of the form <SERVICE>/<METHOD>", service))?;

  let plugin_config = pact.plugin_data.iter()
    .find(|data| data.name == "protobuf")
    .map(|data| &data.configuration)
    .ok_or_else(|| anyhow!("Did not find any Protobuf configuration in the Pact file"))?
    .iter()
    .map(|(k, v)| (k.clone(), v.clone()))
    .collect();
  let descriptors = get_descriptors_for_interaction(descriptor_key.as_str(),
    &plugin_config)?;
  let (file_descriptor, service_descriptor) = find_service_descriptor(&descriptors, service_name)?;
  let method_descriptor = service_descriptor.method.iter().find(|method_desc| {
    method_desc.name.clone().unwrap_or_default() == method_name
  }).ok_or_else(|| anyhow!("Did not find the method {} in the Protobuf file descriptor for service '{}'", method_name, service))?;

  let package = file_descriptor.package.clone();
  Ok((descriptors.clone(), service_descriptor.clone(), method_descriptor.clone(), package.unwrap_or_default()))
}

/// Get the encoded Protobuf descriptors from the Pact level configuration for the message key
pub(crate) fn get_descriptors_for_interaction(
  message_key: &str,
  plugin_config: &BTreeMap<String, serde_json::Value>
) -> anyhow::Result<FileDescriptorSet> {
  let descriptor_config = plugin_config.get(message_key)
    .ok_or_else(|| anyhow!("Plugin configuration item with key '{}' is required. Received config {:?}", message_key, plugin_config.keys()))?
    .as_object()
    .ok_or_else(|| anyhow!("Plugin configuration item with key '{}' has an invalid format", message_key))?;
  let descriptor_bytes_encoded = descriptor_config.get("protoDescriptors")
    .map(json_to_string)
    .unwrap_or_default();
  if descriptor_bytes_encoded.is_empty() {
    return Err(anyhow!("Plugin configuration item with key '{}' is required, but the descriptors were empty. Received config {:?}", message_key, plugin_config.keys()));
  }

  // The descriptor bytes will be base 64 encoded.
  let descriptor_bytes = match BASE64.decode(descriptor_bytes_encoded) {
    Ok(bytes) => Bytes::from(bytes),
    Err(err) => {
      return Err(anyhow!("Failed to decode the Protobuf descriptor - {}", err));
    }
  };
  debug!("Protobuf file descriptor set is {} bytes", descriptor_bytes.len());

  // Get an MD5 hash of the bytes to check that it matches the descriptor key
  let digest = md5::compute(&descriptor_bytes);
  let descriptor_hash = format!("{:x}", digest);
  if descriptor_hash != message_key {
    return Err(anyhow!("Protobuf descriptors checksum failed. Expected {} but got {}", message_key, descriptor_hash));
  }

  // Decode the Protobuf descriptors
  FileDescriptorSet::decode(descriptor_bytes)
    .map_err(|err| anyhow!(err))
}

pub(crate) fn find_service_descriptor<'a>(
  descriptors: &'a FileDescriptorSet,
  service_name: &str
) -> anyhow::Result<(&'a FileDescriptorProto, &'a ServiceDescriptorProto)> {
  descriptors.file.iter().filter_map(|descriptor| {
    descriptor.service.iter()
      .find(|p| p.name.clone().unwrap_or_default() == service_name)
      .map(|p| (descriptor, p))
  })
    .next()
    .ok_or_else(|| anyhow!("Did not find a descriptor for service '{}'", service_name))
}

/// If a field type should be packed. These are repeated fields of primitive numeric types
/// (types which use the varint, 32-bit, or 64-bit wire types)
pub fn should_be_packed_type(field_type: Type) -> bool {
  matches!(field_type, Type::Double | Type::Float | Type::Int64 | Type::Uint64 | Type::Int32 | Type::Fixed64 |
     Type::Fixed32 | Type::Uint32 | Type::Sfixed32 | Type::Sfixed64 | Type::Sint32 |
     Type::Sint64)
}

/// Tries to convert a Protobuf Value to a Map. Returns an error if the incoming value is not a
/// value Protobuf type (Struct or NullValue)
pub fn proto_value_to_map(val: &Value) -> anyhow::Result<BTreeMap<String, Value>> {
  match &val.kind {
    Some(kind) => match kind {
      Kind::NullValue(_) => Ok(BTreeMap::default()),
      Kind::StructValue(s) => Ok(s.fields.clone()),
      _ => Err(anyhow!("Must be a Protobuf Struct or NullValue, got {}", type_of(kind)))
    }
    None => Ok(BTreeMap::default())
  }
}

fn type_of(kind: &Kind) -> String {
  match kind {
    Kind::NullValue(_) => "Null",
    Kind::NumberValue(_) => "Number",
    Kind::StringValue(_) => "String",
    Kind::BoolValue(_) => "Bool",
    Kind::StructValue(_) => "Struct",
    Kind::ListValue(_) => "List"
  }.to_string()
}

pub(crate) fn prost_string<S: Into<String>>(s: S) -> Value {
  Value {
    kind: Some(Kind::StringValue(s.into()))
  }
}

#[cfg(test)]
pub(crate) mod tests {
  use bytes::Bytes;
  use expectest::prelude::*;
  use maplit::hashmap;
  use prost::Message;
  use prost_types::{
    DescriptorProto,
    EnumDescriptorProto,
    EnumValueDescriptorProto,
    FieldDescriptorProto,
    FileDescriptorProto,
    FileDescriptorSet,
    MessageOptions
  };
  use prost_types::field_descriptor_proto::{Label, Type};

  use crate::utils::{as_hex, find_enum_value_by_name, find_message_type_by_name, find_nested_type, is_map_field, last_name};

  #[test]
  fn last_name_test() {
    expect!(last_name("")).to(be_equal_to(""));
    expect!(last_name("test")).to(be_equal_to("test"));
    expect!(last_name(".")).to(be_equal_to(""));
    expect!(last_name("test.")).to(be_equal_to(""));
    expect!(last_name(".test")).to(be_equal_to("test"));
    expect!(last_name("1.2")).to(be_equal_to("2"));
    expect!(last_name("1.2.3.4")).to(be_equal_to("4"));
  }

  pub(crate) const DESCRIPTOR_WITH_EXT_MESSAGE: [u8; 626] = [
    10, 168, 2, 10, 11, 86, 97, 108, 117, 101, 46, 112, 114, 111, 116, 111, 18, 21, 97, 114, 101,
    97, 95, 99, 97, 108, 99, 117, 108, 97, 116, 111, 114, 46, 86, 97, 108, 117, 101, 34, 162, 1,
    10, 14, 65, 100, 66, 114, 101, 97, 107, 67, 111, 110, 116, 101, 120, 116, 18, 36, 10, 14, 102,
    111, 114, 99, 101, 100, 95, 108, 105, 110, 101, 95, 105, 100, 24, 1, 32, 1, 40, 9, 82, 12, 102,
    111, 114, 99, 101, 100, 76, 105, 110, 101, 73, 100, 18, 44, 10, 18, 102, 111, 114, 99, 101,
    100, 95, 99, 114, 101, 97, 116, 105, 118, 101, 95, 105, 100, 24, 2, 32, 1, 40, 9, 82, 16, 102,
    111, 114, 99, 101, 100, 67, 114, 101, 97, 116, 105, 118, 101, 73, 100, 18, 30, 10, 11, 97, 100,
    95, 98, 114, 101, 97, 107, 95, 105, 100, 24, 3, 32, 1, 40, 9, 82, 9, 97, 100, 66, 114, 101, 97,
    107, 73, 100, 18, 28, 10, 9, 115, 101, 115, 115, 105, 111, 110, 73, 100, 24, 4, 32, 1, 40, 9,
    82, 9, 115, 101, 115, 115, 105, 111, 110, 73, 100, 42, 85, 10, 13, 65, 100, 66, 114, 101, 97,
    107, 65, 100, 84, 121, 112, 101, 18, 28, 10, 24, 77, 73, 83, 83, 73, 78, 71, 95, 65, 68, 95,
    66, 82, 69, 65, 75, 95, 65, 68, 95, 84, 89, 80, 69, 16, 0, 18, 18, 10, 14, 65, 85, 68, 73, 79,
    95, 65, 68, 95, 66, 82, 69, 65, 75, 16, 1, 18, 18, 10, 14, 86, 73, 68, 69, 79, 95, 65, 68, 95,
    66, 82, 69, 65, 75, 16, 2, 98, 6, 112, 114, 111, 116, 111, 51, 10, 196, 2, 10, 21, 97, 114,
    101, 97, 95, 99, 97, 108, 99, 117, 108, 97, 116, 111, 114, 46, 112, 114, 111, 116, 111, 18, 15,
    97, 114, 101, 97, 95, 99, 97, 108, 99, 117, 108, 97, 116, 111, 114, 26, 11, 86, 97, 108, 117,
    101, 46, 112, 114, 111, 116, 111, 34, 97, 10, 14, 65, 100, 66, 114, 101, 97, 107, 82, 101, 113,
    117, 101, 115, 116, 18, 79, 10, 16, 97, 100, 95, 98, 114, 101, 97, 107, 95, 99, 111, 110, 116,
    101, 120, 116, 24, 1, 32, 3, 40, 11, 50, 37, 46, 97, 114, 101, 97, 95, 99, 97, 108, 99, 117,
    108, 97, 116, 111, 114, 46, 86, 97, 108, 117, 101, 46, 65, 100, 66, 114, 101, 97, 107, 67, 111,
    110, 116, 101, 120, 116, 82, 14, 97, 100, 66, 114, 101, 97, 107, 67, 111, 110, 116, 101, 120,
    116, 34, 36, 10, 12, 65, 114, 101, 97, 82, 101, 115, 112, 111, 110, 115, 101, 18, 20, 10, 5,
    118, 97, 108, 117, 101, 24, 1, 32, 3, 40, 2, 82, 5, 118, 97, 108, 117, 101, 50, 94, 10, 10,
    67, 97, 108, 99, 117, 108, 97, 116, 111, 114, 18, 80, 10, 12, 99, 97, 108, 99, 117, 108, 97,
    116, 101, 79, 110, 101, 18, 31, 46, 97, 114, 101, 97, 95, 99, 97, 108, 99, 117, 108, 97, 116,
    111, 114, 46, 65, 100, 66, 114, 101, 97, 107, 82, 101, 113, 117, 101, 115, 116, 26, 29, 46, 97,
    114, 101, 97, 95, 99, 97, 108, 99, 117, 108, 97, 116, 111, 114, 46, 65, 114, 101, 97, 82, 101,
    115, 112, 111, 110, 115, 101, 34, 0, 66, 28, 90, 23, 105, 111, 46, 112, 97, 99, 116, 47, 97,
    114, 101, 97, 95, 99, 97, 108, 99, 117, 108, 97, 116, 111, 114, 208, 2, 1, 98, 6, 112, 114,
    111, 116, 111, 51
  ];

  #[test]
  fn find_message_type_by_name_test() {
    let bytes: &[u8] = &DESCRIPTOR_WITH_EXT_MESSAGE;
    let buffer = Bytes::from(bytes);
    let fds = FileDescriptorSet::decode(buffer).unwrap();

    expect!(find_message_type_by_name("", &fds)).to(be_err());
    expect!(find_message_type_by_name("Does not exist", &fds)).to(be_err());

    let (result, _) = find_message_type_by_name("AdBreakRequest", &fds).unwrap();
    expect!(result.name).to(be_some().value("AdBreakRequest"));

    let (result, _) = find_message_type_by_name("AdBreakContext", &fds).unwrap();
    expect!(result.name).to(be_some().value("AdBreakContext"));
  }

  #[test]
  fn find_nested_type_test() {
    let non_message_field = FieldDescriptorProto {
      r#type: Some(Type::Bytes as i32),
      .. FieldDescriptorProto::default()
    };
    let field_with_no_type_name = FieldDescriptorProto {
      r#type: Some(Type::Message as i32),
      .. FieldDescriptorProto::default()
    };
    let field_with_incorrect_type_name = FieldDescriptorProto {
      r#type: Some(Type::Message as i32),
      type_name: Some("field_with_incorrect_type_name".to_string()),
      .. FieldDescriptorProto::default()
    };
    let field_with_matching_type_name = FieldDescriptorProto {
      r#type: Some(Type::Message as i32),
      type_name: Some("CorrectType".to_string()),
      .. FieldDescriptorProto::default()
    };
    let nested = DescriptorProto {
      name: Some("CorrectType".to_string()),
      .. DescriptorProto::default()
    };
    let message = DescriptorProto {
      field: vec![
        non_message_field.clone(),
        field_with_no_type_name.clone(),
        field_with_incorrect_type_name.clone()
      ],
      nested_type: vec![
        nested.clone()
      ],
      .. DescriptorProto::default()
    };
    expect!(find_nested_type(&message, &non_message_field)).to(be_none());
    expect!(find_nested_type(&message, &field_with_no_type_name)).to(be_none());
    expect!(find_nested_type(&message, &field_with_incorrect_type_name)).to(be_none());
    expect!(find_nested_type(&message, &field_with_matching_type_name)).to(be_some().value(nested));
  }

  #[test]
  fn is_map_field_test() {
    let non_message_field = FieldDescriptorProto {
      r#type: Some(Type::Bytes as i32),
      .. FieldDescriptorProto::default()
    };
    let non_repeated_field = FieldDescriptorProto {
      r#type: Some(Type::Message as i32),
      .. FieldDescriptorProto::default()
    };
    let repeated_field_with_no_nested_type = FieldDescriptorProto {
      r#type: Some(Type::Message as i32),
      label: Some(Label::Repeated as i32),
      type_name: Some("field_with_incorrect_type_name".to_string()),
      .. FieldDescriptorProto::default()
    };
    let field_with_non_map_nested_type = FieldDescriptorProto {
      r#type: Some(Type::Message as i32),
      label: Some(Label::Repeated as i32),
      type_name: Some("NonMapType".to_string()),
      .. FieldDescriptorProto::default()
    };
    let field_with_map_nested_type = FieldDescriptorProto {
      r#type: Some(Type::Message as i32),
      label: Some(Label::Repeated as i32),
      type_name: Some("MapType".to_string()),
      .. FieldDescriptorProto::default()
    };
    let non_map_nested = DescriptorProto {
      name: Some("NonMapType".to_string()),
      .. DescriptorProto::default()
    };
    let map_nested = DescriptorProto {
      name: Some("MapType".to_string()),
      options: Some(MessageOptions {
        message_set_wire_format: None,
        no_standard_descriptor_accessor: None,
        deprecated: None,
        map_entry: Some(true),
        uninterpreted_option: vec![]
      }),
      .. DescriptorProto::default()
    };
    let message = DescriptorProto {
      field: vec![
        non_message_field.clone(),
        non_repeated_field.clone(),
        repeated_field_with_no_nested_type.clone(),
        field_with_non_map_nested_type.clone(),
        field_with_map_nested_type.clone()
      ],
      nested_type: vec![
        non_map_nested,
        map_nested
      ],
      .. DescriptorProto::default()
    };
    expect!(is_map_field(&message, &non_message_field)).to(be_false());
    expect!(is_map_field(&message, &non_repeated_field)).to(be_false());
    expect!(is_map_field(&message, &repeated_field_with_no_nested_type)).to(be_false());
    expect!(is_map_field(&message, &field_with_non_map_nested_type)).to(be_false());
    expect!(is_map_field(&message, &field_with_map_nested_type)).to(be_true());
  }

  #[test]
  fn as_hex_test() {
    expect!(as_hex(&[])).to(be_equal_to(""));
    expect!(as_hex(&[1, 2, 3, 255])).to(be_equal_to("010203ff"));
  }

  #[test]
  fn find_enum_value_by_name_test() {
    let enum1 = EnumDescriptorProto {
      name: Some("TestEnum".to_string()),
      value: vec![
        EnumValueDescriptorProto {
          name: Some("VALUE_ZERO".to_string()),
          number: Some(0),
          options: None,
        },
        EnumValueDescriptorProto {
          name: Some("VALUE_ONE".to_string()),
          number: Some(1),
          options: None,
        },
        EnumValueDescriptorProto {
          name: Some("VALUE_TWO".to_string()),
          number: Some(2),
          options: None,
        },
      ],
      .. EnumDescriptorProto::default()
    };
    let fds = FileDescriptorProto {
      name: Some("test_enum.proto".to_string()),
      package: Some("routeguide.v2".to_string()),
      message_type: vec![
        DescriptorProto {
          name: Some("Feature".to_string()),
          field: vec![
            FieldDescriptorProto {
              name: Some("result".to_string()),
              number: Some(1),
              label: Some(1),
              r#type: Some(14),
              type_name: Some(".routeguide.v2.TestEnum".to_string()),
              .. FieldDescriptorProto::default()
            },
          ],
          .. DescriptorProto::default()
        }
      ],
      enum_type: vec![
        enum1.clone()
      ],
      .. FileDescriptorProto::default()
    };
    let fds2 = FileDescriptorProto {
      name: Some("test_enum2.proto".to_string()),
      package: Some("routeguide".to_string()),
      message_type: vec![
        DescriptorProto {
          name: Some("Feature".to_string()),
          field: vec![
            FieldDescriptorProto {
              name: Some("result".to_string()),
              number: Some(1),
              label: Some(1),
              r#type: Some(14),
              type_name: Some(".routeguide.TestEnum".to_string()),
              .. FieldDescriptorProto::default()
            },
          ],
          .. DescriptorProto::default()
        }
      ],
      enum_type: vec![
        enum1.clone()
      ],
      .. FileDescriptorProto::default()
    };
    let fds3 = FileDescriptorProto {
      name: Some("test_enum3.proto".to_string()),
      package: Some("".to_string()),
      message_type: vec![
        DescriptorProto {
          name: Some("Feature".to_string()),
          field: vec![
            FieldDescriptorProto {
              name: Some("result".to_string()),
              number: Some(1),
              label: Some(1),
              r#type: Some(14),
              type_name: Some(".TestEnum".to_string()),
              .. FieldDescriptorProto::default()
            },
          ],
          .. DescriptorProto::default()
        }
      ],
      enum_type: vec![
        enum1.clone()
      ],
      .. FileDescriptorProto::default()
    };
    let descriptors = hashmap!{
      "test_enum.proto".to_string() => &fds,
      "test_enum2.proto".to_string() => &fds2,
      "test_enum3.proto".to_string() => &fds3
    };

    let result = find_enum_value_by_name(&descriptors, ".routeguide.v2.TestEnum", "VALUE_ONE");
    expect!(result).to(be_some().value((1, enum1.clone())));

    let result2 = find_enum_value_by_name(&descriptors, ".routeguide.TestEnum", "VALUE_ONE");
    expect!(result2).to(be_some().value((1, enum1.clone())));

    let result3 = find_enum_value_by_name(&descriptors, ".TestEnum", "VALUE_TWO");
    expect!(result3).to(be_some().value((2, enum1.clone())));
  }
}
