//! Shared utilities

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;

use anyhow::anyhow;
use bytes::{Bytes, BytesMut};
use maplit::hashmap;
use pact_models::json_utils::json_to_string;
use pact_models::pact::load_pact_from_json;
use pact_models::prelude::v4::V4Pact;
use pact_models::v4::interaction::V4Interaction;
use prost::Message;
use prost_types::{DescriptorProto, EnumDescriptorProto, field_descriptor_proto, FieldDescriptorProto, FileDescriptorProto, FileDescriptorSet, MethodDescriptorProto, ServiceDescriptorProto, Value};
use prost_types::field_descriptor_proto::Label;
use prost_types::value::Kind;
use serde_json::json;
use tracing::{debug, error, trace, warn};

use crate::message_decoder::{decode_message, ProtobufField, ProtobufFieldData};

/// Return the last name in a dot separated string
pub fn last_name(entry_type_name: &str) -> &str {
  entry_type_name.split('.').last().unwrap_or(entry_type_name)
}

/// Convert a Protobuf Struct to a BTree Map
pub fn proto_struct_to_btreemap(val: &prost_types::Struct) -> BTreeMap<String, Value> {
  val.fields.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// Search for a message by type name in all the descriptors
pub fn find_message_type_by_name(message_name: &str, descriptors: &FileDescriptorSet) -> anyhow::Result<DescriptorProto> {
  descriptors.file.iter()
    .map(|descriptor| find_message_type_in_file_descriptor(message_name, descriptor).ok())
    .find(|result| result.is_some())
    .flatten()
    .ok_or_else(|| anyhow!("Did not find a message type '{}' in the descriptors", message_name))
}

/// Search for a message by type name in the file descriptor
pub fn find_message_type_in_file_descriptor(message_name: &str, descriptor: &FileDescriptorProto) -> anyhow::Result<DescriptorProto> {
  descriptor.message_type.iter()
    .find(|message| message.name.clone().unwrap_or_default() == message_name)
    .cloned()
    .ok_or_else(|| anyhow!("Did not find a message type '{}' in the file descriptor '{:?}'",
      message_name, descriptor.name))
}

/// If the field is a map field. A field will be a map field if it is a repeated field, the field
/// type is a message and the nested type has the map flag set on the message options.
pub fn is_map_field(message_descriptor: &DescriptorProto, field: &FieldDescriptorProto) -> bool {
  if field.label() == Label::Repeated && field.r#type() == field_descriptor_proto::Type::Message {
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
  if field.r#type() == field_descriptor_proto::Type::Message {
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

/// If the message fields include the field with the given descriptor
pub fn find_message_field<'a>(message: &'a [ProtobufField], field_descriptor: &ProtobufField) -> Option<&'a ProtobufField> {
  message.iter().find(|v| v.field_num == field_descriptor.field_num)
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

/// Find the integer value of the given enum type and name.
pub fn find_enum_value_by_name(message_descriptor: &DescriptorProto, enum_name: &str, enum_value: &str) -> Option<i32> {
  trace!(">> find_enum_value_by_name({:?}, {}, {})", message_descriptor.name, enum_name, enum_value);
  message_descriptor.enum_type.iter()
    .find_map(|enum_descriptor| {
      trace!("find_enum_value_by_name: enum type = {:?}", enum_descriptor.name);
      if let Some(name) = &enum_descriptor.name {
        if name == last_name(enum_name) {
          enum_descriptor.value.iter().find_map(|val| {
            if let Some(name) = &val.name {
              if name == enum_value {
                val.number
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

/// Return the type name of a Prootbuf value
pub fn proto_type_name(value: &Value) -> String {
  match &value.kind {
    Some(kind) => match kind {
      Kind::NullValue(_) => "Null".to_string(),
      Kind::NumberValue(_) => "Number".to_string(),
      Kind::StringValue(_) => "String".to_string(),
      Kind::BoolValue(_) => "Boolean".to_string(),
      Kind::StructValue(_) => "Struct".to_string(),
      Kind::ListValue(_) => "List".to_string(),
    }
    None => "Unknown".to_string()
  }
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
pub(crate) fn lookup_interaction_by_id<'a>(interaction_key: &str, pact: &'a V4Pact) -> anyhow::Result<&'a Box<dyn V4Interaction + Send + Sync>> {
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
  let descriptor_bytes = match base64::decode(descriptor_bytes_encoded) {
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

#[cfg(test)]
pub(crate) mod tests {
  use bytes::Bytes;
  use expectest::prelude::*;
  use prost::Message;
  use prost_types::{DescriptorProto, FieldDescriptorProto, FileDescriptorSet, MessageOptions};
  use prost_types::field_descriptor_proto::{Label, Type};

  use crate::utils::{as_hex, find_message_type_by_name, find_nested_type, is_map_field, last_name};

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

  pub const DESCRIPTORS: &'static str = "CuIFChxnb29nbGUvcHJvdG9idWYvc3RydWN0LnByb3RvEg9nb29\
    nbGUucHJvdG9idWYimAEKBlN0cnVjdBI7CgZmaWVsZHMYASADKAsyIy5nb29nbGUucHJvdG9idWYuU3RydWN0LkZpZWxkc\
    0VudHJ5UgZmaWVsZHMaUQoLRmllbGRzRW50cnkSEAoDa2V5GAEgASgJUgNrZXkSLAoFdmFsdWUYAiABKAsyFi5nb29nbGU\
    ucHJvdG9idWYuVmFsdWVSBXZhbHVlOgI4ASKyAgoFVmFsdWUSOwoKbnVsbF92YWx1ZRgBIAEoDjIaLmdvb2dsZS5wcm90b2\
    J1Zi5OdWxsVmFsdWVIAFIJbnVsbFZhbHVlEiMKDG51bWJlcl92YWx1ZRgCIAEoAUgAUgtudW1iZXJWYWx1ZRIjCgxzdHJpb\
    mdfdmFsdWUYAyABKAlIAFILc3RyaW5nVmFsdWUSHwoKYm9vbF92YWx1ZRgEIAEoCEgAUglib29sVmFsdWUSPAoMc3RydWN0\
    X3ZhbHVlGAUgASgLMhcuZ29vZ2xlLnByb3RvYnVmLlN0cnVjdEgAUgtzdHJ1Y3RWYWx1ZRI7CgpsaXN0X3ZhbHVlGAYgASg\
    LMhouZ29vZ2xlLnByb3RvYnVmLkxpc3RWYWx1ZUgAUglsaXN0VmFsdWVCBgoEa2luZCI7CglMaXN0VmFsdWUSLgoGdmFsdW\
    VzGAEgAygLMhYuZ29vZ2xlLnByb3RvYnVmLlZhbHVlUgZ2YWx1ZXMqGwoJTnVsbFZhbHVlEg4KCk5VTExfVkFMVUUQAEJ/C\
    hNjb20uZ29vZ2xlLnByb3RvYnVmQgtTdHJ1Y3RQcm90b1ABWi9nb29nbGUuZ29sYW5nLm9yZy9wcm90b2J1Zi90eXBlcy9r\
    bm93bi9zdHJ1Y3RwYvgBAaICA0dQQqoCHkdvb2dsZS5Qcm90b2J1Zi5XZWxsS25vd25UeXBlc2IGcHJvdG8zCoYECh5nb29\
    nbGUvcHJvdG9idWYvd3JhcHBlcnMucHJvdG8SD2dvb2dsZS5wcm90b2J1ZiIjCgtEb3VibGVWYWx1ZRIUCgV2YWx1ZRgBIA\
    EoAVIFdmFsdWUiIgoKRmxvYXRWYWx1ZRIUCgV2YWx1ZRgBIAEoAlIFdmFsdWUiIgoKSW50NjRWYWx1ZRIUCgV2YWx1ZRgBI\
    AEoA1IFdmFsdWUiIwoLVUludDY0VmFsdWUSFAoFdmFsdWUYASABKARSBXZhbHVlIiIKCkludDMyVmFsdWUSFAoFdmFsdWUY\
    ASABKAVSBXZhbHVlIiMKC1VJbnQzMlZhbHVlEhQKBXZhbHVlGAEgASgNUgV2YWx1ZSIhCglCb29sVmFsdWUSFAoFdmFsdWU\
    YASABKAhSBXZhbHVlIiMKC1N0cmluZ1ZhbHVlEhQKBXZhbHVlGAEgASgJUgV2YWx1ZSIiCgpCeXRlc1ZhbHVlEhQKBXZhbH\
    VlGAEgASgMUgV2YWx1ZUKDAQoTY29tLmdvb2dsZS5wcm90b2J1ZkINV3JhcHBlcnNQcm90b1ABWjFnb29nbGUuZ29sYW5nL\
    m9yZy9wcm90b2J1Zi90eXBlcy9rbm93bi93cmFwcGVyc3Bi+AEBogIDR1BCqgIeR29vZ2xlLlByb3RvYnVmLldlbGxLbm93\
    blR5cGVzYgZwcm90bzMKvgEKG2dvb2dsZS9wcm90b2J1Zi9lbXB0eS5wcm90bxIPZ29vZ2xlLnByb3RvYnVmIgcKBUVtcHR\
    5Qn0KE2NvbS5nb29nbGUucHJvdG9idWZCCkVtcHR5UHJvdG9QAVouZ29vZ2xlLmdvbGFuZy5vcmcvcHJvdG9idWYvdHlwZX\
    Mva25vd24vZW1wdHlwYvgBAaICA0dQQqoCHkdvb2dsZS5Qcm90b2J1Zi5XZWxsS25vd25UeXBlc2IGcHJvdG8zCv0iCgxwb\
    HVnaW4ucHJvdG8SDmlvLnBhY3QucGx1Z2luGhxnb29nbGUvcHJvdG9idWYvc3RydWN0LnByb3RvGh5nb29nbGUvcHJvdG9i\
    dWYvd3JhcHBlcnMucHJvdG8aG2dvb2dsZS9wcm90b2J1Zi9lbXB0eS5wcm90byJVChFJbml0UGx1Z2luUmVxdWVzdBImCg5\
    pbXBsZW1lbnRhdGlvbhgBIAEoCVIOaW1wbGVtZW50YXRpb24SGAoHdmVyc2lvbhgCIAEoCVIHdmVyc2lvbiLHAgoOQ2F0YW\
    xvZ3VlRW50cnkSPAoEdHlwZRgBIAEoDjIoLmlvLnBhY3QucGx1Z2luLkNhdGFsb2d1ZUVudHJ5LkVudHJ5VHlwZVIEdHlwZ\
    RIQCgNrZXkYAiABKAlSA2tleRJCCgZ2YWx1ZXMYAyADKAsyKi5pby5wYWN0LnBsdWdpbi5DYXRhbG9ndWVFbnRyeS5WYWx1\
    ZXNFbnRyeVIGdmFsdWVzGjkKC1ZhbHVlc0VudHJ5EhAKA2tleRgBIAEoCVIDa2V5EhQKBXZhbHVlGAIgASgJUgV2YWx1ZTo\
    COAEiZgoJRW50cnlUeXBlEhMKD0NPTlRFTlRfTUFUQ0hFUhAAEhUKEUNPTlRFTlRfR0VORVJBVE9SEAESDwoLTU9DS19TRV\
    JWRVIQAhILCgdNQVRDSEVSEAMSDwoLSU5URVJBQ1RJT04QBCJSChJJbml0UGx1Z2luUmVzcG9uc2USPAoJY2F0YWxvZ3VlG\
    AEgAygLMh4uaW8ucGFjdC5wbHVnaW4uQ2F0YWxvZ3VlRW50cnlSCWNhdGFsb2d1ZSJJCglDYXRhbG9ndWUSPAoJY2F0YWxv\
    Z3VlGAEgAygLMh4uaW8ucGFjdC5wbHVnaW4uQ2F0YWxvZ3VlRW50cnlSCWNhdGFsb2d1ZSLlAQoEQm9keRIgCgtjb250ZW5\
    0VHlwZRgBIAEoCVILY29udGVudFR5cGUSNQoHY29udGVudBgCIAEoCzIbLmdvb2dsZS5wcm90b2J1Zi5CeXRlc1ZhbHVlUg\
    djb250ZW50Ek4KD2NvbnRlbnRUeXBlSGludBgDIAEoDjIkLmlvLnBhY3QucGx1Z2luLkJvZHkuQ29udGVudFR5cGVIaW50U\
    g9jb250ZW50VHlwZUhpbnQiNAoPQ29udGVudFR5cGVIaW50EgsKB0RFRkFVTFQQABIICgRURVhUEAESCgoGQklOQVJZEAIi\
    pQMKFkNvbXBhcmVDb250ZW50c1JlcXVlc3QSMAoIZXhwZWN0ZWQYASABKAsyFC5pby5wYWN0LnBsdWdpbi5Cb2R5UghleHB\
    lY3RlZBIsCgZhY3R1YWwYAiABKAsyFC5pby5wYWN0LnBsdWdpbi5Cb2R5UgZhY3R1YWwSMgoVYWxsb3dfdW5leHBlY3RlZF\
    9rZXlzGAMgASgIUhNhbGxvd1VuZXhwZWN0ZWRLZXlzEkcKBXJ1bGVzGAQgAygLMjEuaW8ucGFjdC5wbHVnaW4uQ29tcGFyZ\
    UNvbnRlbnRzUmVxdWVzdC5SdWxlc0VudHJ5UgVydWxlcxJVChNwbHVnaW5Db25maWd1cmF0aW9uGAUgASgLMiMuaW8ucGFj\
    dC5wbHVnaW4uUGx1Z2luQ29uZmlndXJhdGlvblITcGx1Z2luQ29uZmlndXJhdGlvbhpXCgpSdWxlc0VudHJ5EhAKA2tleRg\
    BIAEoCVIDa2V5EjMKBXZhbHVlGAIgASgLMh0uaW8ucGFjdC5wbHVnaW4uTWF0Y2hpbmdSdWxlc1IFdmFsdWU6AjgBIkkKE0\
    NvbnRlbnRUeXBlTWlzbWF0Y2gSGgoIZXhwZWN0ZWQYASABKAlSCGV4cGVjdGVkEhYKBmFjdHVhbBgCIAEoCVIGYWN0dWFsI\
    sMBCg9Db250ZW50TWlzbWF0Y2gSNwoIZXhwZWN0ZWQYASABKAsyGy5nb29nbGUucHJvdG9idWYuQnl0ZXNWYWx1ZVIIZXhw\
    ZWN0ZWQSMwoGYWN0dWFsGAIgASgLMhsuZ29vZ2xlLnByb3RvYnVmLkJ5dGVzVmFsdWVSBmFjdHVhbBIaCghtaXNtYXRjaBg\
    DIAEoCVIIbWlzbWF0Y2gSEgoEcGF0aBgEIAEoCVIEcGF0aBISCgRkaWZmGAUgASgJUgRkaWZmIlQKEUNvbnRlbnRNaXNtYX\
    RjaGVzEj8KCm1pc21hdGNoZXMYASADKAsyHy5pby5wYWN0LnBsdWdpbi5Db250ZW50TWlzbWF0Y2hSCm1pc21hdGNoZXMip\
    wIKF0NvbXBhcmVDb250ZW50c1Jlc3BvbnNlEhQKBWVycm9yGAEgASgJUgVlcnJvchJHCgx0eXBlTWlzbWF0Y2gYAiABKAsy\
    Iy5pby5wYWN0LnBsdWdpbi5Db250ZW50VHlwZU1pc21hdGNoUgx0eXBlTWlzbWF0Y2gSTgoHcmVzdWx0cxgDIAMoCzI0Lml\
    vLnBhY3QucGx1Z2luLkNvbXBhcmVDb250ZW50c1Jlc3BvbnNlLlJlc3VsdHNFbnRyeVIHcmVzdWx0cxpdCgxSZXN1bHRzRW\
    50cnkSEAoDa2V5GAEgASgJUgNrZXkSNwoFdmFsdWUYAiABKAsyIS5pby5wYWN0LnBsdWdpbi5Db250ZW50TWlzbWF0Y2hlc\
    1IFdmFsdWU6AjgBIoABChtDb25maWd1cmVJbnRlcmFjdGlvblJlcXVlc3QSIAoLY29udGVudFR5cGUYASABKAlSC2NvbnRl\
    bnRUeXBlEj8KDmNvbnRlbnRzQ29uZmlnGAIgASgLMhcuZ29vZ2xlLnByb3RvYnVmLlN0cnVjdFIOY29udGVudHNDb25maWc\
    iUwoMTWF0Y2hpbmdSdWxlEhIKBHR5cGUYASABKAlSBHR5cGUSLwoGdmFsdWVzGAIgASgLMhcuZ29vZ2xlLnByb3RvYnVmLl\
    N0cnVjdFIGdmFsdWVzIkEKDU1hdGNoaW5nUnVsZXMSMAoEcnVsZRgBIAMoCzIcLmlvLnBhY3QucGx1Z2luLk1hdGNoaW5nU\
    nVsZVIEcnVsZSJQCglHZW5lcmF0b3ISEgoEdHlwZRgBIAEoCVIEdHlwZRIvCgZ2YWx1ZXMYAiABKAsyFy5nb29nbGUucHJv\
    dG9idWYuU3RydWN0UgZ2YWx1ZXMisQEKE1BsdWdpbkNvbmZpZ3VyYXRpb24SUwoYaW50ZXJhY3Rpb25Db25maWd1cmF0aW9\
    uGAEgASgLMhcuZ29vZ2xlLnByb3RvYnVmLlN0cnVjdFIYaW50ZXJhY3Rpb25Db25maWd1cmF0aW9uEkUKEXBhY3RDb25maW\
    d1cmF0aW9uGAIgASgLMhcuZ29vZ2xlLnByb3RvYnVmLlN0cnVjdFIRcGFjdENvbmZpZ3VyYXRpb24iiAYKE0ludGVyYWN0a\
    W9uUmVzcG9uc2USMAoIY29udGVudHMYASABKAsyFC5pby5wYWN0LnBsdWdpbi5Cb2R5Ughjb250ZW50cxJECgVydWxlcxgC\
    IAMoCzIuLmlvLnBhY3QucGx1Z2luLkludGVyYWN0aW9uUmVzcG9uc2UuUnVsZXNFbnRyeVIFcnVsZXMSUwoKZ2VuZXJhdG9\
    ycxgDIAMoCzIzLmlvLnBhY3QucGx1Z2luLkludGVyYWN0aW9uUmVzcG9uc2UuR2VuZXJhdG9yc0VudHJ5UgpnZW5lcmF0b3\
    JzEkEKD21lc3NhZ2VNZXRhZGF0YRgEIAEoCzIXLmdvb2dsZS5wcm90b2J1Zi5TdHJ1Y3RSD21lc3NhZ2VNZXRhZGF0YRJVC\
    hNwbHVnaW5Db25maWd1cmF0aW9uGAUgASgLMiMuaW8ucGFjdC5wbHVnaW4uUGx1Z2luQ29uZmlndXJhdGlvblITcGx1Z2lu\
    Q29uZmlndXJhdGlvbhIsChFpbnRlcmFjdGlvbk1hcmt1cBgGIAEoCVIRaW50ZXJhY3Rpb25NYXJrdXASZAoVaW50ZXJhY3R\
    pb25NYXJrdXBUeXBlGAcgASgOMi4uaW8ucGFjdC5wbHVnaW4uSW50ZXJhY3Rpb25SZXNwb25zZS5NYXJrdXBUeXBlUhVpbn\
    RlcmFjdGlvbk1hcmt1cFR5cGUSGgoIcGFydE5hbWUYCCABKAlSCHBhcnROYW1lGlcKClJ1bGVzRW50cnkSEAoDa2V5GAEgA\
    SgJUgNrZXkSMwoFdmFsdWUYAiABKAsyHS5pby5wYWN0LnBsdWdpbi5NYXRjaGluZ1J1bGVzUgV2YWx1ZToCOAEaWAoPR2Vu\
    ZXJhdG9yc0VudHJ5EhAKA2tleRgBIAEoCVIDa2V5Ei8KBXZhbHVlGAIgASgLMhkuaW8ucGFjdC5wbHVnaW4uR2VuZXJhdG9\
    yUgV2YWx1ZToCOAEiJwoKTWFya3VwVHlwZRIPCgtDT01NT05fTUFSSxAAEggKBEhUTUwQASLSAQocQ29uZmlndXJlSW50ZX\
    JhY3Rpb25SZXNwb25zZRIUCgVlcnJvchgBIAEoCVIFZXJyb3ISRQoLaW50ZXJhY3Rpb24YAiADKAsyIy5pby5wYWN0LnBsd\
    Wdpbi5JbnRlcmFjdGlvblJlc3BvbnNlUgtpbnRlcmFjdGlvbhJVChNwbHVnaW5Db25maWd1cmF0aW9uGAMgASgLMiMuaW8u\
    cGFjdC5wbHVnaW4uUGx1Z2luQ29uZmlndXJhdGlvblITcGx1Z2luQ29uZmlndXJhdGlvbiLTAgoWR2VuZXJhdGVDb250ZW5\
    0UmVxdWVzdBIwCghjb250ZW50cxgBIAEoCzIULmlvLnBhY3QucGx1Z2luLkJvZHlSCGNvbnRlbnRzElYKCmdlbmVyYXRvcn\
    MYAiADKAsyNi5pby5wYWN0LnBsdWdpbi5HZW5lcmF0ZUNvbnRlbnRSZXF1ZXN0LkdlbmVyYXRvcnNFbnRyeVIKZ2VuZXJhd\
    G9ycxJVChNwbHVnaW5Db25maWd1cmF0aW9uGAMgASgLMiMuaW8ucGFjdC5wbHVnaW4uUGx1Z2luQ29uZmlndXJhdGlvblIT\
    cGx1Z2luQ29uZmlndXJhdGlvbhpYCg9HZW5lcmF0b3JzRW50cnkSEAoDa2V5GAEgASgJUgNrZXkSLwoFdmFsdWUYAiABKAs\
    yGS5pby5wYWN0LnBsdWdpbi5HZW5lcmF0b3JSBXZhbHVlOgI4ASJLChdHZW5lcmF0ZUNvbnRlbnRSZXNwb25zZRIwCghjb2\
    50ZW50cxgBIAEoCzIULmlvLnBhY3QucGx1Z2luLkJvZHlSCGNvbnRlbnRzMuIDCgpQYWN0UGx1Z2luElMKCkluaXRQbHVna\
    W4SIS5pby5wYWN0LnBsdWdpbi5Jbml0UGx1Z2luUmVxdWVzdBoiLmlvLnBhY3QucGx1Z2luLkluaXRQbHVnaW5SZXNwb25z\
    ZRJECg9VcGRhdGVDYXRhbG9ndWUSGS5pby5wYWN0LnBsdWdpbi5DYXRhbG9ndWUaFi5nb29nbGUucHJvdG9idWYuRW1wdHk\
    SYgoPQ29tcGFyZUNvbnRlbnRzEiYuaW8ucGFjdC5wbHVnaW4uQ29tcGFyZUNvbnRlbnRzUmVxdWVzdBonLmlvLnBhY3QucG\
    x1Z2luLkNvbXBhcmVDb250ZW50c1Jlc3BvbnNlEnEKFENvbmZpZ3VyZUludGVyYWN0aW9uEisuaW8ucGFjdC5wbHVnaW4uQ\
    29uZmlndXJlSW50ZXJhY3Rpb25SZXF1ZXN0GiwuaW8ucGFjdC5wbHVnaW4uQ29uZmlndXJlSW50ZXJhY3Rpb25SZXNwb25z\
    ZRJiCg9HZW5lcmF0ZUNvbnRlbnQSJi5pby5wYWN0LnBsdWdpbi5HZW5lcmF0ZUNvbnRlbnRSZXF1ZXN0GicuaW8ucGFjdC5\
    wbHVnaW4uR2VuZXJhdGVDb250ZW50UmVzcG9uc2VCEFoOaW8ucGFjdC5wbHVnaW5iBnByb3RvMw==";

  #[test]
  fn find_message_type_by_name_test() {
    let bytes = base64::decode(DESCRIPTORS).unwrap();
    let bytes1 = Bytes::copy_from_slice(bytes.as_slice());
    let fds = FileDescriptorSet::decode(bytes1).unwrap();

    expect!(find_message_type_by_name("", &fds)).to(be_err());
    expect!(find_message_type_by_name("Does not exist", &fds)).to(be_err());

    let result = find_message_type_by_name("GenerateContentRequest", &fds).unwrap();
    expect!(result.name).to(be_some().value("GenerateContentRequest"));
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
}
