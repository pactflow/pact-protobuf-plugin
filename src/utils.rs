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
use pact_models::path_exp::DocPath;
use pact_models::prelude::v4::V4Pact;
use pact_models::v4::interaction::V4Interaction;
use pact_models::v4::sync_message::SynchronousMessage;
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
use serde_json::{json, Map};
use tracing::{debug, error, instrument, trace, warn};

use crate::message_decoder::{decode_message, ProtobufField, ProtobufFieldData};

pub fn fds_to_map(fds: &FileDescriptorSet) -> HashMap<String, &FileDescriptorProto> {
  fds.file.iter().map(
    |des| (des.name.clone().unwrap_or_default(), des)).collect()
}

fn fds_map_to_vec(descriptors: &HashMap<String, &FileDescriptorProto>) -> Vec<FileDescriptorProto> {
  descriptors.values().map(|d| *d).cloned().collect()
}

/// Return the last name in a dot separated string
pub fn last_name(entry_type_name: &str) -> &str {
  entry_type_name.split('.').last().unwrap_or(entry_type_name)
}

/// Split a dot-seperated string into the package and name part
pub fn parse_name(name: &str) -> (&str, Option<&str>) {
  // if name starts with the '.' it's a fully-qualified name that can contain a package
  if name.starts_with('.') {
    name.rsplit_once('.')
    .map(|(package, name)| {
      if let Some(trimmed) = package.strip_prefix(".") {
        (name, Some(trimmed))
      } else {
        (name, Some(package))
      }
    })
    .unwrap_or_else(|| (name, None))
  } else {
    // otherwise it's a relative name, so if it contains dots, this means embedded types, not packages
    // we don't support embedded types at this point
    (name, None)
  }
}

/// Converts a relative protobuf type name to a fully qualified one by prepending `.<package>.`,
/// or if the package is empty, just a `.`.
/// E.g. `MyType` with package `example` becomes `.example.MyType`
/// and `MyType` with empty package becomes `.MyType`
pub fn to_fully_qualified_name(name: &str, package: &str) -> anyhow::Result<String> {
  match name {
    "" => Err(anyhow!("type name cannot be empty when constructing a fully qualified name")),
    _ => Ok(match package {
      "" => format!(".{}", name),
      _ => format!(".{}.{}", package, name)
    })
  }
}

/// Split a service/method definition into two seprate parts.
/// E.g. MyService/MyMethod becomes ("MyService", "MyMethod")
pub fn split_service_and_method(service_name: &str) -> anyhow::Result<(&str, &str)> {
  match service_name.split_once('/') {
    Some(result) => Ok(result),
    None => Err(anyhow!("Service name '{}' is not valid, it should be of the form <SERVICE>/<METHOD>", service_name))
  }
}


/// Converts from `.package.Service` (fully-qualified name) and `Method` to `/package.Service/Method`
pub fn build_grpc_route(service_full_name: &str, method_name: &str) -> anyhow::Result<String> {
  if service_full_name.is_empty() {
    return Err(anyhow!("Service name cannot be empty"));
  }
  if method_name.is_empty() {
    return Err(anyhow!("Method name cannot be empty"));
  }
  let service_no_dot = if service_full_name.starts_with('.') {
    &service_full_name[1..] // remove the leading dot
  } else {
    service_full_name
  };
  Ok(format!("/{service_no_dot}/{method_name}"))
}

/// Parses `/package.Service/Method` into `.package.Service` (fully-qualified name) and `Method`
pub fn parse_grpc_route(route_key: &str) -> Option<(String, String)> {
  if !route_key.starts_with("/") {
    return None;  // invalid grpc route
  }
  // remove all trailing slashes
  let route_key = route_key.trim_end_matches('/');
  match route_key[1..].split_once('/') { // remove the leading slash
    Some((service, method)) => Some((format!(".{service}"), method.to_string())),
    None => None
  }
}

/// Search for a message by type name in the file descriptor
pub fn find_message_type_in_file_descriptor(
  message_name: &str,
  descriptor: &FileDescriptorProto
) -> anyhow::Result<DescriptorProto> {
  descriptor.message_type.iter()
    .find(|message| message.name() == message_name)
    .cloned()
    .ok_or_else(|| anyhow!("Did not find a message type '{}' in the file descriptor '{}'",
      message_name, descriptor.name.as_deref().unwrap_or("unknown")))
}

// TODO: handle nested types properly
// current name resolution is dumb - just splits package and message name by a dot
// but if you have .package.Message.NestedMessage.NestedMessageDeeperLevel this whole structure breaks down
// because we'll be looking for packages with the name `.package.Message.NestedMessage`
// while here the package is `.package` and then we need to find message called `Message` and go over it's nested types.
// To be fair, I don't think this ever worked properly - it was looking across all file descriptors instead of narrowing them down by packages,
// but it still wasn't looking for nested types.

/// Helper to select a method descriptor by name from a service descriptor.
pub fn find_method_descriptor_for_service(
  method_name: &str,
  service_descriptor: &ServiceDescriptorProto
) -> anyhow::Result<MethodDescriptorProto> {
  let method_descriptor = service_descriptor.method.iter().find(|method_desc| {
    method_desc.name() == method_name
  }).cloned().ok_or_else(|| anyhow!("Did not find the method {} in the Protobuf descriptor for service '{}'", 
    method_name, service_descriptor.name()))?;
  trace!("Found method descriptor {:?} for method {}", method_descriptor, method_name);
  Ok(method_descriptor)
}

/// Find a descriptor for a given type name, fully qualified or relative.
/// Type name format is the same as in `type_name` field in field descriptor
/// or the `input_type`/`output_type` fields in method descriptor.
/// 
/// If type name starts with a dot ('.') it's a fully qualified name, so it is split into package and message names; 
/// if the package is empty, will only lookup messages which have no package.
/// 
/// If type name does not contain a dot, it is a relative type. We'll search all file descriptors then.
/// This isn't techically correct, since we're supposed to start from the current file, and then search
/// level by level, but it's good enough for now (and this is how the plugin used to work for all messages anyway)
pub fn find_message_descriptor_for_type_in_vec(
  type_name: &str,
  all_descriptors: &Vec<FileDescriptorProto>
) -> anyhow::Result<(DescriptorProto, FileDescriptorProto)> {
  let (message_name, package) = parse_name(type_name);
  find_message_descriptor(message_name, package, &all_descriptors)
}

/// Find a descriptor for a given type name, fully qualified or relative.
/// Type name format is the same as in `type_name` field in field descriptor
/// or the `input_type`/`output_type` fields in method descriptor.
/// 
/// If type name starts with a dot ('.') it's a fully qualified name, so it is split into package and message names; 
/// if the package is empty, will only lookup messages which have no package.
/// 
/// If type name does not contain a dot, it is a relative type. We'll search all file descriptors then.
/// This isn't techically correct, since we're supposed to start from the current file, and then search
/// level by level, but it's good enough for now (and this is how the plugin used to work for all messages anyway)
pub fn find_message_descriptor_for_type_in_map(
  type_name: &str,
  descriptors: &HashMap<String, &FileDescriptorProto>,
) -> anyhow::Result<(DescriptorProto, FileDescriptorProto)> {
  let values = fds_map_to_vec(descriptors);
  find_message_descriptor_for_type_in_vec(type_name, &values)
}

/// Find a descriptor for a given type name, fully qualified or relative.
/// Type name format is the same as in `type_name` field in field descriptor
/// or the `input_type`/`output_type` fields in method descriptor.
/// 
/// If type name starts with a dot ('.') it's a fully qualified name, so it is split into package and message names; 
/// if the package is empty, will only lookup messages which have no package.
/// 
/// If type name does not contain a dot, it is a relative type. We'll search all file descriptors then.
/// This isn't techically correct, since we're supposed to start from the current file, and then search
/// level by level, but it's good enough for now (and this is how the plugin used to work for all messages anyway)
pub fn find_message_descriptor_for_type(
  type_name: &str,
  descriptors: &FileDescriptorSet,
) -> anyhow::Result<(DescriptorProto, FileDescriptorProto)> {
  find_message_descriptor_for_type_in_vec(type_name, &descriptors.file)
}

/// Finds message descriptor in a vector of file descriptors. If the package is not none, it will
/// search only the descriptors matching the package 
/// (empty string means descriptors without package, because package is an optional field in proto3). 
/// If it is none, it will search all descriptors, to support cases where pact was generated by the older
/// plugin version which didn't record the message package in the interaction config.
pub(crate) fn find_message_descriptor(
  message_name: &str,
  package: Option<&str>,
  all_descriptors: &Vec<FileDescriptorProto>,
) -> anyhow::Result<(DescriptorProto, FileDescriptorProto)> {
  if package.is_some() {
    trace!("Looking for message descriptor for message '{}' in package '{:?}'", message_name, package);
  } else {
    trace!("Looking for message descriptor for message '{}'", message_name);
  }
  let descriptors = find_file_descriptors(package, &all_descriptors)?;
  descriptors.iter()
    .find_map(|fd| find_message_type_in_file_descriptor(message_name, fd).ok().map(|msg| (msg, fd.clone())))
    .ok_or_else(|| {
        anyhow!(
            "Did not find a message type '{}' in any of the file descriptors '{:?}'", 
            message_name, 
            descriptors.iter().map(|d| d.name()).collect::<Vec<_>>())
    })
}

/// Find a service descriptor for a given service type name, fully qualified or relative.
/// 
/// If type name starts with a dot ('.') it's a fully qualified name, so it is split into package and message names; 
/// if the package is empty, will only lookup services which have no package.
/// 
/// If type name does not contain a dot, it is a relative type. We'll search all file descriptors then.
/// This isn't techically correct, since we're supposed to start from the current file, and then search
/// level by level, but it's good enough for now (and this is how the plugin used to work for all services anyway)
pub(crate) fn find_service_descriptor_for_type(
  type_name: &str,
  all_descriptors: &FileDescriptorSet
) -> anyhow::Result<(FileDescriptorProto, ServiceDescriptorProto)> {
  let (message_name, package) = parse_name(type_name);
  find_service_descriptor(message_name, package, all_descriptors)
}

pub(crate) fn find_service_descriptor(
  service_name: &str,
  package: Option<&str>,
  descriptors: &FileDescriptorSet
) -> anyhow::Result<(FileDescriptorProto, ServiceDescriptorProto)> {
  if package.is_some() {
    debug!("Looking for service '{}' with package '{:?}'", service_name, package);
  } else {
    debug!("Looking for service '{}'", service_name);
  }
  let file_descriptors = find_file_descriptors(package, &descriptors.file)?;
  file_descriptors.iter().filter_map(|descriptor| {
    descriptor.service.iter()
      .find(|p| p.name() == service_name)
      .map(|p| {
        trace!("Found service descriptor with name {:?}", p.name);
        (descriptor.clone(), p.clone())
      })
  })
    .next()
    .ok_or_else(|| anyhow!("Did not find a descriptor for service '{}'", service_name))
}

pub fn find_file_descriptors(
  package: Option<&str>,
  all_descriptors: &Vec<FileDescriptorProto>,
) -> anyhow::Result<Vec<FileDescriptorProto>> {
  match package {
    Some(pkg) if pkg.is_empty() => {
      debug!("Looking for file descriptors with no package");
      find_all_file_descriptors_with_no_package(all_descriptors)
    }
    Some(pkg) => {
      debug!("Looking for file descriptors with package '{}'", pkg);
      find_all_file_descriptors_for_package(pkg, all_descriptors)
    }
    None => Ok(all_descriptors.clone())
  }
}

fn find_all_file_descriptors_for_package(
  package: &str,
  all_descriptors: &Vec<FileDescriptorProto>,
) -> anyhow::Result<Vec<FileDescriptorProto>> {
  let package = if package.starts_with('.') {
      &package[1..]
  } else {
      package
  };
  let found: Vec<_> = all_descriptors.iter().filter(|descriptor| {
      trace!("Checking file descriptor '{:?}' with package '{:?}' while looking for package '{}'", 
        descriptor.name, descriptor.package, package);
      if let Some(descriptor_package) = &descriptor.package {
          debug!("Found file descriptor '{:?}' with package '{:?}'", descriptor.name, descriptor_package);
          descriptor_package == package
      } else {
          false
      }
  }).cloned().collect();
  if found.is_empty() {
      Err(anyhow!("Did not find any file descriptors for a package '{}'", package))
  } else {
      debug!("Found {} file descriptors for package '{}'", found.len(), package);
      Ok(found)
  }
}

fn find_all_file_descriptors_with_no_package(
  all_descriptors: &Vec<FileDescriptorProto>
  ) -> anyhow::Result<Vec<FileDescriptorProto>> {
  let found: Vec<_> = all_descriptors.iter().filter(|d| d.package.is_none()).cloned().collect();
  if found.is_empty() {
      Err(anyhow!("Did not find any file descriptors with no package specified"))
  } else {
      debug!("Found {} file descriptors with no package", found.len());
      Ok(found)
  }
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
    let message_type = last_name(field.type_name());
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
    .find(|f| f.name() == field_name)
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
  trace!(">> find_enum_by_name_in_message({})",enum_name);
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
  let enum_name_full = enum_name.split('.').filter(|v| !v.is_empty()).collect::<Vec<_>>().join(".");
  let result = descriptors.values()
        .find_map(|fd| {
          let package = fd.package();
          if enum_name_full.starts_with(package) {
            let enum_name_short = enum_name_full.replace(package, "");
            let enum_name_parts = enum_name_short.split('.').filter(|v| !v.is_empty()).collect::<Vec<_>>();
            if let Some((_name, message_name)) = enum_name_parts.split_last() {
              if message_name.is_empty() {
                find_enum_value_by_name_in_message(&fd.enum_type, enum_name, enum_value)
              } else {
                let message_name = message_name.join(".");
                if let Ok(message_descriptor) = find_message_type_in_file_descriptor(&message_name, fd) {
                  find_enum_value_by_name_in_message(&message_descriptor.enum_type, enum_name, enum_value)
                } else {
                  None
                }
              }
            } else {
              None
            }
          } else {
            None
          }
        });
  if result.is_some() {
    result
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
  // TODO: unify this name split logic with the one in split_name
  let enum_name_full = enum_name.split('.').filter(|v| !v.is_empty()).collect::<Vec<_>>().join(".");
  let result = descriptors.file.iter()
        .find_map(|fd| {
          // TODO: combine this with the rest of the package search logic;
          // this one actually supports nested enum types,
          // but starts_with check is not always correct (need to split by dots)
          // and I don't think it recurses inside message-in-message
          let package = fd.package();
          if enum_name_full.starts_with(package) {
            let enum_name_short = enum_name_full.replace(package, "");
            let enum_name_parts = enum_name_short.split('.').filter(|v| !v.is_empty()).collect::<Vec<_>>();
            if let Some((_name, message_name)) = enum_name_parts.split_last() {
              if message_name.is_empty() {
                find_enum_by_name_in_message(&fd.enum_type, enum_name)
              } else {
                let message_name = message_name.join(".");
                if let Ok(message_descriptor) = find_message_type_in_file_descriptor(&message_name, fd) {
                  find_enum_by_name_in_message(&message_descriptor.enum_type, enum_name)
                } else {
                  None
                }
              }
            } else {
              None
            }
          } else {
            None
          }
        });
  if result.is_some() {
    result
  } else {
    None
  }
}

/// Convert the Google Struct field data into a JSON value
#[instrument(level = "trace", skip(descriptors))]
pub fn struct_field_data_to_json(
  field_data: Vec<ProtobufField>,
  descriptor: &DescriptorProto,
  descriptors: &FileDescriptorSet
) -> anyhow::Result<serde_json::Value> {
  let mut object = Map::new();

  for field in field_data {
    if let ProtobufFieldData::Message(b, entry_descriptor) = &field.data {
      trace!(name = ?entry_descriptor.name, ?b, "constructing entry");
      let mut bytes = BytesMut::from(b.as_slice());
      let message_data = decode_message(&mut bytes, entry_descriptor, descriptors)?;
      trace!(?message_data, "decoded entry");
      if message_data.len() == 2 {
        let key_field = message_data.iter().find(|f| f.field_name == "key")
          .ok_or_else(|| anyhow!("Did not find the key for the entry"))?;
        let value_field = message_data.iter().find(|f| f.field_name == "value")
          .ok_or_else(|| anyhow!("Did not find the value for the entry"))?;
        let key = if let ProtobufFieldData::String(key) = &key_field.data {
          key.clone()
        } else {
          return Err(anyhow!("Key for {} must be a String, but got {}", entry_descriptor.name(), key_field.data.type_name()));
        };
        let value = proto_value_to_json(descriptors, value_field)?;
        object.insert(key, value);
      } else {
        return Err(anyhow!("Was expecting 2 values (key, value) for the entry with field number {}, but got {:?}", field.field_num, message_data));
      }
    } else {
      return Err(anyhow!("Was expecting a message for the entry with field number {}, but got {}", field.field_num, field.data));
    }
  }

  Ok(serde_json::Value::Object(object))
}

#[instrument(level = "trace", skip(descriptors))]
fn proto_value_to_json(
  descriptors: &FileDescriptorSet,
  value_field: &ProtobufField
) -> anyhow::Result<serde_json::Value> {
  match &value_field.data {
    ProtobufFieldData::Message(m, d) => {
      let mut bytes = BytesMut::from(m.as_slice());
      let message_data = decode_message(&mut bytes, d, descriptors)?;
      trace!(?message_data, "decoded value");
      if let Some(field_data) = message_data.first() {
        match &field_data.data {
          ProtobufFieldData::String(s) => Ok(serde_json::Value::String(s.clone())),
          ProtobufFieldData::Boolean(b) => Ok(serde_json::Value::Bool(*b)),
          ProtobufFieldData::UInteger32(n) => Ok(json!(*n)),
          ProtobufFieldData::Integer32(n) => Ok(json!(*n)),
          ProtobufFieldData::UInteger64(n) => Ok(json!(*n)),
          ProtobufFieldData::Integer64(n) => Ok(json!(*n)),
          ProtobufFieldData::Float(f) => Ok(json!(*f)),
          ProtobufFieldData::Double(f) => Ok(json!(*f)),
          ProtobufFieldData::Message(m, desc) => {
            if desc.name() == "ListValue" {
              let mut list_bytes = BytesMut::from(m.as_slice());
              let list_data = decode_message(&mut list_bytes, desc, descriptors)?;
              trace!(?list_data, "decoded list");
              let mut items = vec![];
              for field in &list_data {
                items.push(proto_value_to_json(descriptors, field)?);
              }
              Ok(serde_json::Value::Array(items))
            } else if desc.name() == "Struct" {
              let mut struct_bytes = BytesMut::from(m.as_slice());
              let struct_data = decode_message(&mut struct_bytes, desc, descriptors)?;
              trace!(?struct_data, "decoded struct");
              struct_field_data_to_json(struct_data, desc, descriptors)
            } else {
              Err(anyhow!("{} is not a valid value for a Struct entry", field_data.data.type_name()))
            }
          }
          ProtobufFieldData::Enum(_, enum_desc) if enum_desc.name() == "NullValue" => {
            Ok(serde_json::Value::Null)
          }
          _ => {
            Err(anyhow!("{} is not a valid value for a Struct entry", field_data.data.type_name()))
          }
        }
      } else {
        warn!("Decoded entry value is empty");
        Ok(serde_json::Value::Null)
      }
    }
    _ => {
      Err(anyhow!("Found an unrecognisable type for a Google Struct field {}", value_field.data.type_name()))
    }
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
pub fn lookup_interaction_by_id<'a>(
  interaction_key: &str,
  pact: &'a V4Pact
) -> Option<&'a (dyn V4Interaction + Send + Sync + RefUnwindSafe)> {
  pact.interactions.iter()
    .find(|i| {
      trace!(interaction_key, unique_key=i.unique_key(), "Checking interaction for key");
      i.unique_key() == interaction_key
    })
    .map(|i| i.as_ref())
}

pub fn lookup_interaction_config(interaction: &dyn V4Interaction) -> Option<HashMap<String, serde_json::Value>> {
  interaction.plugin_config().iter()
    .find_map(|(key, value)| {
      if key.as_str() == "protobuf" {
        Some(value.clone())
      } else {
        None
      }
    })
}

pub fn lookup_plugin_config(pact: &V4Pact) -> anyhow::Result<BTreeMap<String, serde_json::Value>>{
  let plugin_config = pact.plugin_data.iter()
    .find(|data| data.name == "protobuf")
    .map(|data| &data.configuration)
    .ok_or_else(|| anyhow!("Did not find any Protobuf configuration in the Pact file"))?
    .iter()
    .map(|(k, v)| (k.clone(), v.clone()))
    .collect();
  Ok(plugin_config)
}

/// Returns the service descriptors for the given interaction.
/// Will load all descriptors from the pact file using `descriptorKey` from interaction config,
/// and then find the correct file, service and method descriptors using `service` value from
/// the interaction config.
/// 
/// # Arguments
/// - `interaction` - A specific interaction from the pact
/// - `pact` - Pact (contains this interaction too)
/// 
/// # Returns
/// A tuple of:
/// - FileDescriptorSet - all available file descriptors
/// - ServiceDescriptorProto - the service descriptor for this gRPC service
/// - MethodDescriptorProto - the method descriptor for this gRPC service
/// - FileDescriptorProto - the file descriptor containing this gRPC service
pub(crate) fn lookup_service_descriptors_for_interaction(
  interaction: &dyn V4Interaction,
  pact: &V4Pact
) -> anyhow::Result<(FileDescriptorSet, ServiceDescriptorProto, MethodDescriptorProto, FileDescriptorProto)> {
  // TODO: a similar flow happens in server::compare_contents, can it be refactored to a common function?
  // compare_contents works with both service and message, while this one only works with the service.
  let interaction_config = lookup_interaction_config(interaction)
    .ok_or_else(|| anyhow!("Interaction does not have any Protobuf configuration"))?;
  let descriptor_key = interaction_config.get("descriptorKey")
    .map(json_to_string)
    .ok_or_else(|| anyhow!("Interaction descriptorKey was missing in Pact file"))?;
  let service = interaction_config.get("service")
    .map(json_to_string)
    .ok_or_else(|| anyhow!("Interaction gRPC service was missing in Pact file"))?;
  
  let (service_with_package, method_name) = split_service_and_method(service.as_str())?;
  trace!("gRPC service for interaction: {}", service_with_package);
  
  let plugin_config = lookup_plugin_config(pact)?;
  let descriptors = get_descriptors_for_interaction(descriptor_key.as_str(), &plugin_config)?;
  trace!("file descriptors for interaction {:?}", descriptors);
  
  let (file_descriptor, service_descriptor) = find_service_descriptor_for_type(service_with_package, &descriptors)?;
  let method_descriptor = find_method_descriptor_for_service( method_name, &service_descriptor)?;
  Ok((descriptors.clone(), service_descriptor.clone(), method_descriptor.clone(), file_descriptor.clone()))
}

fn get_descriptor_config<'a>(
  message_key: &str,
  plugin_config: &'a BTreeMap<String, serde_json::Value>
) -> anyhow::Result<&'a serde_json::Map<String, serde_json::Value>> {
  plugin_config.get(message_key)
    .ok_or_else(|| anyhow!("Plugin configuration item with key '{}' is required. Received config {:?}", message_key, plugin_config.keys()))?
    .as_object()
    .ok_or_else(|| anyhow!("Plugin configuration item with key '{}' has an invalid format", message_key))
}

/// Get the encoded Protobuf descriptors from the Pact level configuration for the message key
pub fn get_descriptors_for_interaction(
  message_key: &str,
  plugin_config: &BTreeMap<String, serde_json::Value>
) -> anyhow::Result<FileDescriptorSet> {
  let descriptor_config = get_descriptor_config(message_key, plugin_config)?;
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

/// If a field type should be packed. These are repeated fields of primitive numeric types
/// (types which use the varint, 32-bit, or 64-bit wire types)
pub fn should_be_packed_type(field_type: Type) -> bool {
  matches!(field_type, Type::Double | Type::Float | Type::Int64 | Type::Uint64 | Type::Int32 | Type::Fixed64 |
     Type::Fixed32 | Type::Uint32 | Type::Sfixed32 | Type::Sfixed64 | Type::Sint32 |
     Type::Sint64 | Type::Enum)
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

pub fn build_expectations(
  interaction: &SynchronousMessage,
  part: &str
) -> Option<HashMap<DocPath, String>> {
  interaction.plugin_config()
    .get("protobuf")
    .and_then(|config| config.get("expectations"))
    .and_then(|config| config.as_object())
    .and_then(|expectations| expectations.get(part))
    .and_then(|config| config.as_object())
    .map(|expectations| expectations_from_json(expectations))
}

pub fn expectations_from_json(json: &Map<String, serde_json::Value>) -> HashMap<DocPath, String> {
  let path = DocPath::root();
  let mut result = hashmap!{};
  for (field, value) in json {
    expectations_from_json_inner(&path.join(field), &mut result, value);
  }
  result
}

fn expectations_from_json_inner(
  path: &DocPath,
  acc: &mut HashMap<DocPath, String>,
  json: &serde_json::Value
) {
  match json {
    serde_json::Value::Array(array) => {
      acc.insert(path.clone(), "".to_string());
      for (index, item) in array.iter().enumerate() {
        expectations_from_json_inner(&path.join_index(index), acc, item);
      }
    }
    serde_json::Value::Object(attrs) => {
      acc.insert(path.clone(), "".to_string());
      for (field, value) in attrs {
        expectations_from_json_inner(&path.join(field), acc, value);
      }
    }
    _ => {
      acc.insert(path.clone(), json.to_string());
    }
  }
}

#[cfg(test)]
pub(crate) mod tests {
  use std::collections::HashSet;
  use std::vec;

  use base64::Engine;
  use base64::engine::general_purpose::STANDARD as BASE64;
  use bytes::{BufMut, Bytes, BytesMut};
  use expectest::prelude::*;
  use maplit::{hashmap, hashset};
  use pretty_assertions::assert_eq;
  use prost::encoding::WireType::LengthDelimited;
  use prost::Message;
  use prost_types::{
    DescriptorProto,
    EnumDescriptorProto,
    EnumValueDescriptorProto,
    FieldDescriptorProto,
    FileDescriptorProto,
    FileDescriptorSet,
    MessageOptions,
    MethodDescriptorProto,
    ServiceDescriptorProto
  };
  use prost_types::field_descriptor_proto::{Label, Type};
  use prost_types::field_descriptor_proto::Label::Optional;
  use serde_json::json;
  use crate::message_decoder::{ProtobufField, ProtobufFieldData};
  use crate::utils::{as_hex, struct_field_data_to_json, find_enum_value_by_name, find_nested_type, is_map_field, last_name, parse_name, to_fully_qualified_name};
  use super::{
    build_grpc_route,
    find_file_descriptors,
    find_message_descriptor_for_type,
    find_method_descriptor_for_service,
    find_service_descriptor_for_type,
    parse_grpc_route,
    split_service_and_method
  };

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

  #[test]
  fn parse_name_test() {
    // fully-qulified names start with a dot
    expect!(parse_name(".package.Type")).to(be_equal_to(("Type", Some("package"))));
    expect!(parse_name(".Type")).to(be_equal_to(("Type", Some(""))));
    expect!(parse_name(".")).to(be_equal_to(("", Some(""))));  // TODO: should this be an error case?
    
    // relative names must have package set to None always
    expect!(parse_name("")).to(be_equal_to(("", None)));   // TODO: should this be an error case?
    expect!(parse_name("test")).to(be_equal_to(("test", None)));
    expect!(parse_name("test.")).to(be_equal_to(("test.", None)));
    expect!(parse_name("1.2.3.4")).to(be_equal_to(("1.2.3.4", None)));
  }

  #[test]
  fn split_service_and_method_test() {
    expect!(split_service_and_method("")).to(be_err());
    expect!(split_service_and_method("test")).to(be_err());
    expect!(split_service_and_method("/").unwrap()).to(be_equal_to(("", "")));
    expect!(split_service_and_method("/method").unwrap()).to(be_equal_to(("", "method")));
    expect!(split_service_and_method("service/").unwrap()).to(be_equal_to(("service", "")));
    expect!(split_service_and_method("service/method").unwrap()).to(be_equal_to(("service", "method")));
    // TODO: we don't support this case either way - maybe we should error out if there's more than one slash?
    expect!(split_service_and_method("service/subservice/method").unwrap()).to(be_equal_to(("service", "subservice/method")));
  }

  #[test]
  fn to_fully_qualified_name_test() {
    expect!(to_fully_qualified_name("service", "package").unwrap()).to(be_equal_to(".package.service"));
    expect!(to_fully_qualified_name("service", "package.with.dots").unwrap()).to(be_equal_to(".package.with.dots.service"));
    expect!(to_fully_qualified_name("service", "").unwrap()).to(be_equal_to(".service"));
    expect!(to_fully_qualified_name("", "package")).to(be_err());
  }

  #[test]
  fn test_build_grpc_route() {
    // Valid inputs
    expect!(build_grpc_route(".com.example.Service", "Method").unwrap()).to(be_equal_to("/com.example.Service/Method"));
    expect!(build_grpc_route("com.example.Service", "Method").unwrap()).to(be_equal_to("/com.example.Service/Method"));

    // Errors
    expect!(build_grpc_route("", "Method")).to(be_err());
    expect!(build_grpc_route("com.example.Service", "")).to(be_err());
    expect!(build_grpc_route("", "")).to(be_err());
  }

  #[test]
  fn test_parse_grpc_route() {
    // Valid inputs
    expect!(parse_grpc_route("/com.example.Service/Method")).to(be_some().value((".com.example.Service".to_string(), "Method".to_string())));
    expect!(parse_grpc_route("/com.example.Service/Method/")).to(be_some().value((".com.example.Service".to_string(), "Method".to_string())));

    // Errors
    expect!(parse_grpc_route("com.example.Service/Method")).to(be_none());
    expect!(parse_grpc_route("/com.example.Service")).to(be_none());
    expect!(parse_grpc_route("/com.example.Service/")).to(be_none());
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
  fn find_message_descriptor_for_type_ext_test() {
    /*
    Contents of the descriptor:
    File descriptor: Some("Value.proto") package Some("area_calculator.Value")
    Message: Some("AdBreakContext")
    Enum: Some("AdBreakAdType")
    File descriptor: Some("area_calculator.proto") package Some("area_calculator")
    Message: Some("AdBreakRequest")
    Message: Some("AreaResponse")
    Service: Some("Calculator")
    Method: Some("calculateOne")
     */
    let bytes: &[u8] = &DESCRIPTOR_WITH_EXT_MESSAGE;
    let buffer = Bytes::from(bytes);
    let fds = FileDescriptorSet::decode(buffer).unwrap();

    expect!(find_message_descriptor_for_type("", &fds)).to(be_err());
    expect!(find_message_descriptor_for_type("Does not exist", &fds)).to(be_err());

    let (result, _) = find_message_descriptor_for_type("AdBreakRequest", &fds).unwrap();
    expect!(result.name).to(be_some().value("AdBreakRequest"));

    let (result, file_descriptor) = find_message_descriptor_for_type(".area_calculator.Value.AdBreakContext", &fds).unwrap();
    expect!(result.name).to(be_some().value("AdBreakContext"));
    expect!(file_descriptor.package).to(be_some().value("area_calculator.Value"));
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
    let fds4 = FileDescriptorProto {
      name: Some("test_enum4.proto".to_string()),
      package: Some("routeguide.v3".to_string()),
      message_type: vec![
        DescriptorProto {
          name: Some("Feature".to_string()),
          enum_type: vec![
            enum1.clone()
          ],
          .. DescriptorProto::default()
        }
      ],
      .. FileDescriptorProto::default()
    };
    let descriptors = hashmap!{
      "test_enum.proto".to_string() => &fds,
      "test_enum2.proto".to_string() => &fds2,
      "test_enum3.proto".to_string() => &fds3,
      "test_enum4.proto".to_string() => &fds4
    };

    let result = find_enum_value_by_name(&descriptors, ".routeguide.v2.TestEnum", "VALUE_ONE");
    expect!(result).to(be_some().value((1, enum1.clone())));

    let result2 = find_enum_value_by_name(&descriptors, ".routeguide.TestEnum", "VALUE_ONE");
    expect!(result2).to(be_some().value((1, enum1.clone())));

    let result3 = find_enum_value_by_name(&descriptors, ".TestEnum", "VALUE_TWO");
    expect!(result3).to(be_some().value((2, enum1.clone())));

    let result4 = find_enum_value_by_name(&descriptors, ".routeguide.v3.Feature.TestEnum", "VALUE_ONE");
    expect!(result4).to(be_some().value((1, enum1.clone())));
  }

  #[test]
  fn find_message_descriptor_for_type_test() {
    let request_msg = DescriptorProto {
      name: Some("Request".to_string()),
      .. DescriptorProto::default()
    };
    let another_request_msg = DescriptorProto {
      name: Some("AnotherRequest".to_string()),
      .. DescriptorProto::default()
    };
    let request_file: FileDescriptorProto = FileDescriptorProto {
      name: Some("request.proto".to_string()),
      package: Some("service".to_string()),
      message_type: vec![
        request_msg.clone(),
        another_request_msg.clone()
      ],
      .. FileDescriptorProto::default()
    };
    let request_file2: FileDescriptorProto = FileDescriptorProto {
      name: Some("request.proto".to_string()),
      package: Some("service2".to_string()),
      message_type: vec![
        request_msg.clone()
      ],
      .. FileDescriptorProto::default()
    };
    let all_descriptors = FileDescriptorSet{file: vec!{request_file.clone(), request_file2.clone()}};
    // fully qualified name
    let (md, fd) = find_message_descriptor_for_type(".service.Request", &all_descriptors).unwrap();
    expect!(&md).to(be_equal_to(&request_msg));
    expect!(&fd).to(be_equal_to(&request_file));

    // relative name
    let (md, fd) = find_message_descriptor_for_type("AnotherRequest", &all_descriptors).unwrap();
    expect!(&md).to(be_equal_to(&another_request_msg));
    expect!(&fd).to(be_equal_to(&request_file));

    // package not found error
    let result_err = find_message_descriptor_for_type(".missing.MissingType", &all_descriptors);
    expect!(result_err.as_ref()).to(be_err());
    expect!(&result_err.unwrap_err().to_string()).to(be_equal_to(
      "Did not find any file descriptors for a package 'missing'"));
    // message not found error
    let result_err = find_message_descriptor_for_type(".service.MissingType", &all_descriptors);
    expect!(result_err.as_ref()).to(be_err());
    let error_msg = result_err.unwrap_err().to_string();
    expect!(error_msg.starts_with(
      "Did not find a message type 'MissingType' in any of the file descriptors")).to(be_true());
  }

  #[test]
  fn find_service_descriptor_for_type_test() {
    let service_desc = ServiceDescriptorProto {
      name: Some("Service".to_string()),
      .. ServiceDescriptorProto::default()
    }; 
    let service = FileDescriptorProto {
      name: Some("service.proto".to_string()),
      package: Some("service".to_string()),
      service: vec![
        service_desc.clone(),
        ServiceDescriptorProto {
          name: Some("AnotherService".to_string()),
          .. ServiceDescriptorProto::default()
        }
      ],
      .. FileDescriptorProto::default()
    };
    let relative_name_service = ServiceDescriptorProto {
      name: Some("RelativeNameService".to_string()),
      .. ServiceDescriptorProto::default()
    };
    let service2 = FileDescriptorProto {
      name: Some("service.proto".to_string()),
      package: Some("service".to_string()),
      service: vec![
        ServiceDescriptorProto {
          name: Some("Service".to_string()),
          .. ServiceDescriptorProto::default()
        },
        relative_name_service.clone()
      ],
      .. FileDescriptorProto::default()
    };
    let all_descriptors = FileDescriptorSet { file: vec!{service.clone(), service2.clone()} };

    let (fd, sd) = find_service_descriptor_for_type(".service.Service", &all_descriptors).unwrap();
    expect!(fd).to(be_equal_to(service));
    expect!(sd).to(be_equal_to(service_desc));

    let (fd, sd) = find_service_descriptor_for_type("RelativeNameService", &all_descriptors).unwrap();
    expect!(fd).to(be_equal_to(service2));
    expect!(sd).to(be_equal_to(relative_name_service));

    // missing package case
    let result_err = find_service_descriptor_for_type(".missing.MissingService", &all_descriptors);
    expect!(result_err.as_ref()).to(be_err());
    expect!(&result_err.unwrap_err().to_string()).to(be_equal_to(
      "Did not find any file descriptors for a package 'missing'"));
    // missing service case
    let result_err = find_service_descriptor_for_type(".service.MissingService", &all_descriptors);
    expect!(result_err.as_ref()).to(be_err());
    expect!(&result_err.unwrap_err().to_string()).to(be_equal_to(
      "Did not find a descriptor for service 'MissingService'"));
  }

  #[test]
  fn find_file_descriptors_test() {
    let request: FileDescriptorProto = FileDescriptorProto {
      name: Some("request.proto".to_string()),
      package: Some("service".to_string()),
      .. FileDescriptorProto::default()
    };
    let response = FileDescriptorProto {
      name: Some("response.proto".to_string()),
      package: Some("service".to_string()),
      .. FileDescriptorProto::default()
    };
    let request_no_package = FileDescriptorProto {
      name: Some("request_no_package.proto".to_string()),
        .. FileDescriptorProto::default()
    };
    let response_no_package = FileDescriptorProto {
      name: Some("response_no_package.proto".to_string()),
        .. FileDescriptorProto::default()
    };
    let all_descriptors_with_package_names = hashset!{
      "request.proto".to_string(), 
      "response.proto".to_string()
    };
    let all_descriptors_with_no_pacakge_names = hashset!{
      "request_no_package.proto".to_string(), 
      "response_no_package.proto".to_string()
    };
    let all_descritptor_names = hashset!{
      "request.proto".to_string(), 
      "response.proto".to_string(), 
      "request_no_package.proto".to_string(), 
      "response_no_package.proto".to_string()
    };
    let all_descriptors = vec!{request, response, request_no_package, response_no_package};
    // explicitly provide package name
    _check_find_file_descriptors(Some("service"), &all_descriptors_with_package_names, &all_descriptors);

    // same but with a dot
    _check_find_file_descriptors(Some(".service"), &all_descriptors_with_package_names, &all_descriptors);

    // empty package means return descriptors without packages only
    _check_find_file_descriptors(Some(""), &all_descriptors_with_no_pacakge_names, &all_descriptors);

    // none package means return all descriptors
    _check_find_file_descriptors(None, &all_descritptor_names, &all_descriptors);

    // Errors
    // did not find any file descriptor with specified package
    let result = find_file_descriptors(Some("missing"), &all_descriptors);
    expect!(result.as_ref()).to(be_err());
    expect!(&result.unwrap_err().to_string()).to(be_equal_to("Did not find any file descriptors for a package 'missing'"));
    // did not find any file descriptors with no package
    let result = find_file_descriptors(Some(""), &vec!{});
    expect!(&result.unwrap_err().to_string()).to(be_equal_to("Did not find any file descriptors with no package specified"));
  }

  fn _check_find_file_descriptors(
    package: Option<&str>,
    expected: &HashSet<String>,
    all_descriptors: &Vec<FileDescriptorProto>
  ) {
    let actual = find_file_descriptors(package, all_descriptors).unwrap().iter()
      .map(|d: &FileDescriptorProto| d.name.clone().unwrap_or_default()).collect::<HashSet<String>>();
    expect!(&actual).to(be_equal_to(expected));
  }

  #[test]
  fn find_method_descriptor_for_service_test() {
    let method_desc1 = MethodDescriptorProto{
      name: Some("method1".to_string()),
      ..MethodDescriptorProto::default()
    };
    let method_desc2 = MethodDescriptorProto{
      name: Some("method2".to_string()),
      ..MethodDescriptorProto::default()
    };
    let service_desc = ServiceDescriptorProto {
      name: Some("Service".to_string()),
      method: vec!{
        method_desc1.clone(),
        method_desc2.clone()
      },
      .. ServiceDescriptorProto::default()
    };
    let actual = find_method_descriptor_for_service("method1", &service_desc).unwrap();
    expect!(actual).to(be_equal_to(method_desc1));
    // error case
    let result_err = find_method_descriptor_for_service("missing", &service_desc);
    expect!(result_err.as_ref()).to(be_err());
    expect!(result_err.unwrap_err().to_string())
      .to(be_equal_to("Did not find the method missing in the Protobuf descriptor for service 'Service'"));
  }

  #[test_log::test]
  fn field_data_to_json_test() {
    // message Request {
    //   string name = 1;
    //   google.protobuf.Struct params = 2;
    // }
    let desc = "CuIFChxnb29nbGUvcHJvdG9idWYvc3RydWN0LnByb3RvEg9nb29nbGUucHJvdG9idWYimAEKBlN0\
    cnVjdBI7CgZmaWVsZHMYASADKAsyIy5nb29nbGUucHJvdG9idWYuU3RydWN0LkZpZWxkc0VudHJ5UgZmaWVsZHMaUQoLR\
    mllbGRzRW50cnkSEAoDa2V5GAEgASgJUgNrZXkSLAoFdmFsdWUYAiABKAsyFi5nb29nbGUucHJvdG9idWYuVmFsdWVSBX\
    ZhbHVlOgI4ASKyAgoFVmFsdWUSOwoKbnVsbF92YWx1ZRgBIAEoDjIaLmdvb2dsZS5wcm90b2J1Zi5OdWxsVmFsdWVIAFI\
    JbnVsbFZhbHVlEiMKDG51bWJlcl92YWx1ZRgCIAEoAUgAUgtudW1iZXJWYWx1ZRIjCgxzdHJpbmdfdmFsdWUYAyABKAlIA\
    FILc3RyaW5nVmFsdWUSHwoKYm9vbF92YWx1ZRgEIAEoCEgAUglib29sVmFsdWUSPAoMc3RydWN0X3ZhbHVlGAUgASgLMh\
    cuZ29vZ2xlLnByb3RvYnVmLlN0cnVjdEgAUgtzdHJ1Y3RWYWx1ZRI7CgpsaXN0X3ZhbHVlGAYgASgLMhouZ29vZ2xlLn\
    Byb3RvYnVmLkxpc3RWYWx1ZUgAUglsaXN0VmFsdWVCBgoEa2luZCI7CglMaXN0VmFsdWUSLgoGdmFsdWVzGAEgAygLMhY\
    uZ29vZ2xlLnByb3RvYnVmLlZhbHVlUgZ2YWx1ZXMqGwoJTnVsbFZhbHVlEg4KCk5VTExfVkFMVUUQAEJ/ChNjb20uZ29v\
    Z2xlLnByb3RvYnVmQgtTdHJ1Y3RQcm90b1ABWi9nb29nbGUuZ29sYW5nLm9yZy9wcm90b2J1Zi90eXBlcy9rbm93bi9zd\
    HJ1Y3RwYvgBAaICA0dQQqoCHkdvb2dsZS5Qcm90b2J1Zi5XZWxsS25vd25UeXBlc2IGcHJvdG8zCpwBChRnb29nbGVfc3\
    RydWN0cy5wcm90bxIOZ29vZ2xlX3N0cnVjdHMaHGdvb2dsZS9wcm90b2J1Zi9zdHJ1Y3QucHJvdG8iTgoHUmVxdWVzdBIS\
    CgRuYW1lGAEgASgJUgRuYW1lEi8KBnBhcmFtcxgCIAEoCzIXLmdvb2dsZS5wcm90b2J1Zi5TdHJ1Y3RSBnBhcmFtc2IGc\
    HJvdG8z";

    let bytes = BASE64.decode(desc).unwrap();
    let bytes1 = Bytes::copy_from_slice(bytes.as_slice());
    let fds: FileDescriptorSet = FileDescriptorSet::decode(bytes1).unwrap();

    let key_descriptor = FieldDescriptorProto {
      name: Some("key".to_string()),
      number: Some(1),
      label: Some(Optional as i32),
      r#type: Some(Type::String as i32),
      json_name: Some("key".to_string()),
      ..FieldDescriptorProto::default()
    };
    let value_descriptor = FieldDescriptorProto {
      name: Some("value".to_string()),
      number: Some(2),
      label: Some(Optional as i32),
      r#type: Some(Type::Message as i32),
      type_name: Some(".google.protobuf.Value".to_string()),
      json_name: Some("value".to_string()),
      ..FieldDescriptorProto::default()
    };
    let field_descriptor =  DescriptorProto {
      name: Some("FieldsEntry".to_string()),
      field: vec![
        key_descriptor.clone(),
        value_descriptor.clone()
      ],
      options: Some(MessageOptions {
        message_set_wire_format: None,
        no_standard_descriptor_accessor: None,
        deprecated: None,
        map_entry: Some(true),
        uninterpreted_option: vec![]
      }),
      .. DescriptorProto::default()
    };

    let mut buffer = BytesMut::new();
    buffer.put_u8(10); // field 1 length encoded (1 << 3 + 2 == 10)
    buffer.put_u8(1); // 1 byte
    buffer.put_slice("n".as_bytes());
    buffer.put_u8(18); // field 2 length encoded (2 << 3 + 2 == 18)
    buffer.put_u8(2); // 2 bytes
    buffer.put_u8(8); // field 1 varint (1 << 3 + 0 == 8)
    buffer.put_u8(0); // 0 (NULL Value)

    let mut buffer2 = BytesMut::new();
    buffer2.put_u8(10); // field 1 length encoded (1 << 3 + 2 == 10)
    buffer2.put_u8(1); // 1 byte
    buffer2.put_slice("b".as_bytes());
    buffer2.put_u8(18); // field 2 length encoded (2 << 3 + 2 == 18)
    buffer2.put_u8(2); // 2 bytes
    buffer2.put_u8(32); // field 4 varint (4 << 3 + 0 == 32)
    buffer2.put_u8(1); // 1 == true

    let mut buffer3 = BytesMut::new();
    buffer3.put_u8(10); // field 1 length encoded (1 << 3 + 2 == 10)
    buffer3.put_u8(3); // 3 bytes
    buffer3.put_slice("num".as_bytes());
    buffer3.put_u8(18); // field 2 length encoded (2 << 3 + 2 == 18)
    buffer3.put_u8(9); // 9 bytes
    buffer3.put_u8(17); // field 2 64bit (2 << 3 + 1 == 17)
    buffer3.put_f64_le(100.0); // 100 as f64

    let field_data = vec![
      ProtobufField {
        field_num: 1,
        field_name: "fields".to_string(),
        wire_type: LengthDelimited,
        data: ProtobufFieldData::Message(
          buffer.freeze().to_vec(),
          field_descriptor.clone()
        ),
        additional_data: vec![],
        descriptor: Default::default()
      },
      ProtobufField {
        field_num: 1,
        field_name: "fields".to_string(),
        wire_type: LengthDelimited,
        data: ProtobufFieldData::Message(
          buffer2.freeze().to_vec(),
          field_descriptor.clone()
        ),
        additional_data: vec![],
        descriptor: Default::default()
      },
      ProtobufField {
        field_num: 1,
        field_name: "fields".to_string(),
        wire_type: LengthDelimited,
        data: ProtobufFieldData::Message(
          buffer3.freeze().to_vec(),
          field_descriptor.clone()
        ),
        additional_data: vec![],
        descriptor: Default::default()
      }
    ];

    let result = struct_field_data_to_json(field_data, &field_descriptor, &fds).unwrap();
    assert_eq!(result, json!({
      "n": null,
      "b": true,
      "num": 100.0
    }));

    // Original Issue #71
    let mut buffer1 = BytesMut::new();
    buffer1.put_u8(10); // field 1 length encoded (1 << 3 + 2 == 10)
    buffer1.put_u8(7); // 7 bytes
    buffer1.put_slice("message".as_bytes());
    buffer1.put_u8(18); // field 2 length encoded (2 << 3 + 2 == 18)
    buffer1.put_u8(6); // 6 bytes
    buffer1.put_u8(26); // field 3 length encoded (3 << 3 + 2 == 26)
    buffer1.put_u8(4); // 4 bytes
    buffer1.put_slice("test".as_bytes());

    let mut buffer2 = BytesMut::new();
    buffer2.put_u8(10); // field 1 length encoded (1 << 3 + 2 == 10)
    buffer2.put_u8(4); // 4 bytes
    buffer2.put_slice("kind".as_bytes());
    buffer2.put_u8(18); // field 2 length encoded (2 << 3 + 2 == 18)
    buffer2.put_u8(9); // 9 bytes
    buffer2.put_u8(26); // field 3 length encoded (3 << 3 + 2 == 26)
    buffer2.put_u8(7); // 7 bytes
    buffer2.put_slice("general".as_bytes());

    let field_data = vec![
      ProtobufField {
        field_num: 1,
        field_name: "fields".to_string(),
        wire_type: LengthDelimited,
        data: ProtobufFieldData::Message(
          buffer1.freeze().to_vec(),
          field_descriptor.clone()
        ),
        additional_data: vec![],
        descriptor: Default::default()
      }, ProtobufField {
        field_num: 1,
        field_name: "fields".to_string(),
        wire_type: LengthDelimited,
        data: ProtobufFieldData::Message(
          buffer2.freeze().to_vec(),
          field_descriptor.clone()
        ),
        additional_data: vec![],
        descriptor: Default::default()
      }
    ];

    let result = struct_field_data_to_json(field_data, &field_descriptor, &fds).unwrap();
    assert_eq!(result, json!({
      "message": "test",
      "kind": "general"
    }));
  }
}
