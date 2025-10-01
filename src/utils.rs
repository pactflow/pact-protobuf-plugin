//! Shared utilities

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;
use std::panic::RefUnwindSafe;
use std::sync::{Arc, RwLock};

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

/// A cached descriptor lookup structure that provides efficient access to protobuf descriptors.
/// Replaces direct usage of FileDescriptorSet and HashMap<String, &FileDescriptorProto> throughout the codebase.
/// 
/// Now includes caching for improved lookup performance:
/// - Package -> File descriptors cache (pre-built)
/// - FQN -> Message descriptor cache (lazy, populated on demand)
/// 
/// Note: Not cloneable due to internal RwLock caches (thread-safe, shared access by reference)
#[derive(Debug)]
pub struct DescriptorCache {
  /// All file descriptors (single source of truth)
  file_descriptors: Vec<FileDescriptorProto>,
  /// Map of file name to index in file_descriptors vec (pre-built for O(1) lookup)
  file_descriptors_map: HashMap<String, usize>,
  /// Cache: package name -> indices of file descriptors (pre-built for performance)
  package_cache: HashMap<String, Vec<usize>>,
  /// Cache: fully qualified name -> (message descriptor, file descriptor) (lazy, uses RwLock for thread-safe interior mutability)
  message_fqn_cache: RwLock<HashMap<String, (DescriptorProto, FileDescriptorProto)>>,
  /// Cache: fully qualified name -> (file descriptor, service descriptor) (lazy, uses RwLock for thread-safe interior mutability)
  service_fqn_cache: RwLock<HashMap<String, (FileDescriptorProto, ServiceDescriptorProto)>>,
  /// Cache: enum name -> enum descriptor (lazy, uses RwLock for thread-safe interior mutability)
  enum_fqn_cache: RwLock<HashMap<String, EnumDescriptorProto>>,
}

impl DescriptorCache {
  /// Create a new DescriptorCache from a FileDescriptorSet
  /// Pre-builds index-based maps for fast O(1) lookups
  pub fn new(fds: FileDescriptorSet) -> Self {
    let file_descriptors = fds.file;
    
    // Build map: filename -> index in file_descriptors vec
    let file_descriptors_map: HashMap<String, usize> = file_descriptors
      .iter()
      .enumerate()
      .map(|(idx, des)| (des.name.clone().unwrap_or_default(), idx))
      .collect();
    
    // Pre-build package cache: package -> Vec<indices>
    let package_cache = Self::build_package_cache(&file_descriptors);
    
    DescriptorCache {
      file_descriptors,
      file_descriptors_map,
      package_cache,
      message_fqn_cache: RwLock::new(HashMap::new()),
      service_fqn_cache: RwLock::new(HashMap::new()),
      enum_fqn_cache: RwLock::new(HashMap::new()),
    }
  }

  /// Build the package cache mapping package names to indices of file descriptors
  /// Files with no package (package=None) are stored with key ""
  fn build_package_cache(file_descriptors: &[FileDescriptorProto]) -> HashMap<String, Vec<usize>> {
    let mut cache: HashMap<String, Vec<usize>> = HashMap::new();
    
    for (idx, fd) in file_descriptors.iter().enumerate() {
      let package_key = fd.package.clone().unwrap_or_else(|| String::from(""));
      cache.entry(package_key)
        .or_insert_with(Vec::new)
        .push(idx);
    }
    
    debug!("Built package cache with {} entries", cache.len());
    cache
  }

  /// Split a fully-qualified name and try all package/name combinations.
  /// For `.package.Message.Nested`, returns iterator of (package, name) tuples:
  /// - ("", "package.Message.Nested")
  /// - ("package", "Message.Nested")
  /// - ("package.Message", "Nested")
  fn try_package_splits(fqn: &str) -> Vec<(String, String)> {
    if !fqn.starts_with('.') {
      // Relative name - no package splitting needed
      return vec![(String::new(), fqn.to_string())];
    }
    
    let parts: Vec<&str> = fqn[1..].split('.').collect();
    let mut results = Vec::new();
    
    for i in 0..=parts.len() {
      let (package_parts, name_parts) = parts.split_at(i);
      let name = name_parts.join(".");
      
      if !name.is_empty() {
        let package = package_parts.join(".");
        results.push((package, name));
      }
    }
    
    results
  }

  /// Tier 1 helper: Try all package/name splits and call Tier 2 lookup for each.
  /// This eliminates duplication across message, service, and enum Tier 1 lookups.
  fn try_all_package_splits<T, F>(
    &self,
    type_name: &str,
    descriptor_type: &str,
    lookup_fn: F
  ) -> anyhow::Result<T>
  where
    F: Fn(&str, Option<&str>) -> anyhow::Result<T>
  {
    let mut last_error = anyhow!("{} '{}' not found", descriptor_type, type_name);
    let is_fqn = type_name.starts_with('.');
    
    // Warn about deprecated relative name usage
    if !is_fqn {
      warn!(
        "DEPRECATED: Using relative {} name '{}' without leading dot. \
         This is supported for backward compatibility with old pact files, \
         but new pacts should use fully qualified names (e.g., '.package.{}')",
        descriptor_type.to_lowercase(),
        type_name,
        type_name
      );
    }
    
    for (package, name) in Self::try_package_splits(type_name) {
      // For FQN: always pass Some(package), even for empty string (empty = no package)
      // For relative names: pass None to search all files (backward compatibility)
      let package_opt = if is_fqn {
        Some(package.as_str())
      } else {
        None
      };
      
      match lookup_fn(&name, package_opt) {
          Ok(result) => return Ok(result),
          Err(e) => last_error = e
        }
      }
      
      Err(last_error)
    }

  /// Tier 2 helper: Search for a descriptor across file descriptors.
  /// This eliminates duplication across message, service, and enum Tier 2 lookups.
  fn lookup_in_files<T, F>(
    &self,
    name: &str,
    descriptor_type: &str,
    package: Option<&str>,
    search_fn: F
  ) -> anyhow::Result<T>
  where
    F: Fn(&FileDescriptorProto) -> Option<T>
  {
    trace!("Looking for {} '{}' in package '{:?}'", descriptor_type, name, package);
    
    let file_descriptors = self.find_file_descriptors(package)?;
    
    file_descriptors.iter()
      .find_map(search_fn)
      .ok_or_else(|| {
        anyhow!(
          "{} '{}' not found in package '{:?}' (searched {} file(s))",
          descriptor_type,
          name,
          package,
          file_descriptors.len()
        )
      })
  }
  
  /// Split a descriptor name into parts, filtering empty strings.
  /// E.g., "Message.Nested" -> ["Message", "Nested"]
  fn split_name_parts(name: &str) -> Vec<&str> {
    name.split('.').filter(|v| !v.is_empty()).collect()
  }

  /// Generic cache wrapper to eliminate cache access duplication.
  /// Checks cache first, runs lookup function if not cached, then caches result.
  fn with_cache<K, V, F>(
    &self,
    cache: &RwLock<HashMap<K, V>>,
    key: K,
    cache_name: &str,
    lookup_fn: F
  ) -> anyhow::Result<V>
  where
    K: Eq + std::hash::Hash + Clone + std::fmt::Display,
    V: Clone,
    F: FnOnce() -> anyhow::Result<V>
  {
    // Check cache first
    let cache_read = cache.read().unwrap();
    if let Some(cached) = cache_read.get(&key) {
      trace!("Found {} for '{}' in cache", cache_name, key);
      return Ok(cached.clone());
    }
    drop(cache_read); // Release read lock before doing lookup
    
    // Do lookup
    let result = lookup_fn()?;
    
    // Cache the result
    cache.write().unwrap().insert(key.clone(), result.clone());
    trace!("Cached {} for '{}'", cache_name, key);
    
    Ok(result)
  }

  // ============================================================================
  // Message Descriptor Lookup (Three-Tier Architecture)
  // ============================================================================

  /// Tier 1: Find a message descriptor for a given type name, fully qualified or relative.
  /// Uses cache and tries all package/name combinations.
  pub fn find_message_descriptor_for_type(
    &self,
    type_name: &str,
  ) -> anyhow::Result<(DescriptorProto, FileDescriptorProto)> {
    self.with_cache(
      &self.message_fqn_cache,
      type_name.to_string(),
      "message descriptor",
      || self.try_all_package_splits(type_name, "Message", 
           |name, pkg| self.lookup_message_descriptor(name, pkg))
    )
  }

  /// Tier 2: Looks up message descriptor across file descriptors for a specific package.
  /// If package is None, searches all descriptors (for backward compatibility).
  fn lookup_message_descriptor(
    &self,
    message_name: &str,
    package: Option<&str>,
  ) -> anyhow::Result<(DescriptorProto, FileDescriptorProto)> {
    self.lookup_in_files(message_name, "Message", package, |fd| {
      self.find_message_in_file(message_name, fd)
        .ok()
        .map(|msg| (msg, fd.clone()))
    })
  }

  /// Tier 3: Find a message within a specific file descriptor, handling nested messages.
  fn find_message_in_file(
    &self,
    message_name: &str,
    file_descriptor: &FileDescriptorProto
  ) -> anyhow::Result<DescriptorProto> {
    let parts = Self::split_name_parts(message_name);
    
    if parts.is_empty() {
      return Err(anyhow!("Empty message name"));
    }
    
    // Find top-level message
    let first_message = file_descriptor.message_type.iter()
      .find(|msg| msg.name() == parts[0])
      .cloned()
      .ok_or_else(|| anyhow!("Message '{}' not found in file '{}'", 
        parts[0], file_descriptor.name()))?;
    
    if parts.len() == 1 {
      return Ok(first_message);
    }
    
    // Recursively find nested messages
    self.find_nested_message_recursive(&first_message, &parts[1..])
  }

  // ============================================================================
  // Service Descriptor Lookup (Three-Tier Architecture)
  // (but service is 2 tiers only because it cannot be nested in a message)
  // ============================================================================

  /// Tier 1: Find a service descriptor for a given service type name, fully qualified or relative.
  /// Uses cache and tries all package/name combinations.
  pub fn find_service_descriptor_for_type(
    &self,
    type_name: &str
  ) -> anyhow::Result<(FileDescriptorProto, ServiceDescriptorProto)> {
    self.with_cache(
      &self.service_fqn_cache,
      type_name.to_string(),
      "service descriptor",
      || self.try_all_package_splits(type_name, "Service",
           |name, pkg| self.lookup_service_descriptor(name, pkg))
    )
  }

  /// Tier 2: Looks up service descriptor across file descriptors for a specific package.
  /// If package is None, searches all descriptors (for backward compatibility).
  fn lookup_service_descriptor(
    &self,
    service_name: &str,
    package: Option<&str>
  ) -> anyhow::Result<(FileDescriptorProto, ServiceDescriptorProto)> {
    self.lookup_in_files(service_name, "Service", package, |fd| {
      fd.service.iter()
        .find(|s| s.name() == service_name)
        .map(|s| (fd.clone(), s.clone()))
    })
  }

  // ============================================================================
  // Enum Descriptor Lookup (Three-Tier Architecture)
  // ============================================================================

  /// Tier 1: Find an enum by name, fully qualified or relative.
  /// Uses cache and tries all package/name combinations.
  pub fn find_enum_by_name(&self, enum_name: &str) -> Option<EnumDescriptorProto> {
    trace!(">> find_enum_by_name({})", enum_name);
    
    // Use with_cache but convert Result to Option
    self.with_cache(
      &self.enum_fqn_cache,
      enum_name.to_string(),
      "enum descriptor",
      || self.try_all_package_splits(enum_name, "Enum",
           |name, pkg| self.lookup_enum_descriptor(name, pkg))
    ).ok()
  }

  /// Find an enum value by name. First finds the enum, then looks up the value.
  /// This is simpler and more efficient than the old approach which duplicated lookup logic.
  pub fn find_enum_value_by_name(
    &self,
    enum_name: &str,
    enum_value: &str
  ) -> Option<(i32, EnumDescriptorProto)> {
    trace!(">> find_enum_value_by_name({}, {})", enum_name, enum_value);
    
    // Find the enum descriptor (uses cache)
    let enum_descriptor = self.find_enum_by_name(enum_name)?;
    
    // Find the value in the enum descriptor
    find_enum_value_in_descriptor(&enum_descriptor, enum_value)
      .map(|value_num| (value_num, enum_descriptor))
  }

  /// Tier 2: Looks up enum descriptor across file descriptors for a specific package.
  fn lookup_enum_descriptor(
    &self,
    enum_name: &str,
    package: Option<&str>
  ) -> anyhow::Result<EnumDescriptorProto> {
    self.lookup_in_files(enum_name, "Enum", package, |fd| {
      self.find_enum_in_file(enum_name, fd)
    })
  }

  /// Tier 3: Find an enum within a specific file descriptor.
  /// Enums can be top-level or nested inside messages.
  fn find_enum_in_file(
    &self,
    enum_name: &str,
    file_descriptor: &FileDescriptorProto
  ) -> Option<EnumDescriptorProto> {
    let parts = Self::split_name_parts(enum_name);
    
    if parts.is_empty() {
      return None;
    }
    
    if parts.len() == 1 {
      // Top-level enum in file
      find_enum_by_name_in_message(&file_descriptor.enum_type, enum_name)
            } else {
      // Nested in message - last part is enum, rest is message path
      let message_path = parts[..parts.len()-1].join(".");
      let enum_simple_name = parts[parts.len()-1];
      
      if let Ok(message) = self.find_message_in_file(&message_path, file_descriptor) {
        find_enum_by_name_in_message(&message.enum_type, enum_simple_name)
    } else {
      None
      }
    }
  }

  /// Find file descriptors by package (internal/test use only)
  pub(crate) fn find_file_descriptors(
    &self,
    package: Option<&str>,
  ) -> anyhow::Result<Vec<FileDescriptorProto>> {
    match package {
      Some(pkg) if pkg.is_empty() => {
        debug!("Looking for file descriptors with no package");
        self.find_all_file_descriptors_with_no_package()
      }
      Some(pkg) => {
        debug!("Looking for file descriptors with package '{}'", pkg);
        self.find_all_file_descriptors_for_package(pkg)
      }
      None => Ok(self.file_descriptors.clone())
    }
  }

  fn find_all_file_descriptors_for_package(
    &self,
    package: &str,
  ) -> anyhow::Result<Vec<FileDescriptorProto>> {
    let package = if package.starts_with('.') {
        &package[1..]
    } else {
        package
    };
    
    // Use package cache - stores indices, convert to FileDescriptorProto
    if let Some(indices) = self.package_cache.get(package) {
      trace!("Found {} file descriptors for package '{}' in cache", indices.len(), package);
      let descriptors: Vec<FileDescriptorProto> = indices.iter()
        .map(|&idx| self.file_descriptors[idx].clone())
        .collect();
      Ok(descriptors)
        } else {
      // Package not in cache means it doesn't exist (cache is complete)
        Err(anyhow!("Did not find any file descriptors for a package '{}'", package))
    }
  }

  fn find_all_file_descriptors_with_no_package(
    &self
    ) -> anyhow::Result<Vec<FileDescriptorProto>> {
    // Files with no package are cached with empty string key
    if let Some(indices) = self.package_cache.get("") {
      trace!("Found {} file descriptors with no package in cache", indices.len());
      let descriptors: Vec<FileDescriptorProto> = indices.iter()
        .map(|&idx| self.file_descriptors[idx].clone())
        .collect();
      Ok(descriptors)
    } else {
      // Empty string key not in cache means no files without package
      Err(anyhow!("Did not find any file descriptors with no package specified"))
    }
  }

  // Backward compatibility methods for accessing raw collections

  /// Get all file descriptors as a vector (used for logging/tracing)
  pub fn get_file_descriptors_vec(&self) -> &Vec<FileDescriptorProto> {
    &self.file_descriptors
  }

  /// Get a file descriptor by file name (uses pre-built index map for O(1) lookup)
  pub fn get_file_descriptor_by_name(&self, file_name: &str) -> Option<&FileDescriptorProto> {
    self.file_descriptors_map
      .get(file_name)
      .map(|&idx| &self.file_descriptors[idx])
  }

  /// Helper function to recursively find nested messages
  fn find_nested_message_recursive(
    &self,
    current_message: &DescriptorProto,
    remaining_parts: &[&str]
  ) -> anyhow::Result<DescriptorProto> {
    if remaining_parts.is_empty() {
      return Ok(current_message.clone());
    }

    let next_message_name = remaining_parts[0];
    trace!("find_nested_message_recursive: looking for '{}' in message '{}'", next_message_name, current_message.name());

    // Look in the nested types of the current message
    let nested_message = current_message.nested_type.iter()
      .find(|msg| msg.name() == next_message_name)
      .cloned()
      .ok_or_else(|| anyhow!("Did not find nested message type '{}' in message '{}'",
        next_message_name, current_message.name()))?;

    // Recursively search for the remaining parts
    self.find_nested_message_recursive(&nested_message, &remaining_parts[1..])
  }

  }

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

  /// Find the integer value of the given enum type and name in the message descriptor.
/// This is a convenience function that combines finding the enum and finding the value within it.
  #[tracing::instrument(ret, skip_all, fields(%enum_name, %enum_value))]
pub fn find_enum_value_by_name_in_message(
    enum_types: &[EnumDescriptorProto],
    enum_name: &str,
    enum_value: &str
  ) -> Option<(i32, EnumDescriptorProto)> {
  trace!(">> find_enum_value_by_name_in_message({}, {})", enum_name, enum_value);
  
  // First find the enum by name, then find the value within it
  find_enum_by_name_in_message(enum_types, enum_name)
    .and_then(|enum_desc| {
      find_enum_value_in_descriptor(&enum_desc, enum_value)
        .map(|n| (n, enum_desc))
      })
  }

  /// Find the enum type by name in the message descriptor.
  #[tracing::instrument(ret, skip_all, fields(%enum_name))]
pub fn find_enum_by_name_in_message(
    enum_types: &[EnumDescriptorProto],
    enum_name: &str
  ) -> Option<EnumDescriptorProto> {
  trace!(">> find_enum_by_name_in_message({})", enum_name);
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

/// Find the integer value of a given enum value name in an enum descriptor.
/// This is a simple helper that just looks up the value in the enum's value list.
pub fn find_enum_value_in_descriptor(
  enum_descriptor: &EnumDescriptorProto,
  enum_value: &str
) -> Option<i32> {
  enum_descriptor.value.iter()
    .find(|val| val.name() == enum_value)
    .and_then(|val| val.number)
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

/// Convert the Google Struct field data into a JSON value
#[instrument(level = "trace", skip(descriptor_cache))]
pub fn struct_field_data_to_json(
  field_data: Vec<ProtobufField>,
  descriptor: &DescriptorProto,
  descriptor_cache: &DescriptorCache
) -> anyhow::Result<serde_json::Value> {
  let mut object = Map::new();

  for field in field_data {
    if let ProtobufFieldData::Message(b, entry_descriptor) = &field.data {
      trace!(name = ?entry_descriptor.name, ?b, "constructing entry");
      let mut bytes = BytesMut::from(b.as_slice());
      let message_data = decode_message(&mut bytes, entry_descriptor, descriptor_cache)?;
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
        let value = proto_value_to_json(descriptor_cache, value_field)?;
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

#[instrument(level = "trace", skip(descriptor_cache))]
fn proto_value_to_json(
  descriptor_cache: &DescriptorCache,
  value_field: &ProtobufField
) -> anyhow::Result<serde_json::Value> {
  match &value_field.data {
    ProtobufFieldData::Message(m, d) => {
      let mut bytes = BytesMut::from(m.as_slice());
      let message_data = decode_message(&mut bytes, d, descriptor_cache)?;
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
              let list_data = decode_message(&mut list_bytes, desc, descriptor_cache)?;
              trace!(?list_data, "decoded list");
              let mut items = vec![];
              for field in &list_data {
                items.push(proto_value_to_json(descriptor_cache, field)?);
              }
              Ok(serde_json::Value::Array(items))
            } else if desc.name() == "Struct" {
              let mut struct_bytes = BytesMut::from(m.as_slice());
              let struct_data = decode_message(&mut struct_bytes, desc, descriptor_cache)?;
              trace!(?struct_data, "decoded struct");
              struct_field_data_to_json(struct_data, desc, descriptor_cache)
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

/// Lookup service and method descriptors for an interaction using a pre-built cache.
/// This function does NOT parse descriptors - it expects the cache to already exist.
/// 
/// # Arguments
/// * `interaction` - The V4 interaction to lookup descriptors for
/// * `descriptor_cache` - Pre-built descriptor cache to use for lookups
/// 
/// # Returns
/// A tuple of:
/// - ServiceDescriptorProto - the service descriptor for this gRPC service
/// - MethodDescriptorProto - the method descriptor for this gRPC service
/// - FileDescriptorProto - the file descriptor containing this gRPC service
pub(crate) fn lookup_service_and_method_for_interaction(
  interaction: &dyn V4Interaction,
  descriptor_cache: &Arc<DescriptorCache>
) -> anyhow::Result<(ServiceDescriptorProto, MethodDescriptorProto, FileDescriptorProto)> {
  let interaction_config = lookup_interaction_config(interaction)
    .ok_or_else(|| anyhow!("Interaction does not have any Protobuf configuration"))?;
  
  let service = interaction_config.get("service")
    .map(json_to_string)
    .ok_or_else(|| anyhow!("Interaction gRPC service was missing in Pact file"))?;
  
  let (service_with_package, method_name) = split_service_and_method(service.as_str())?;
  trace!("gRPC service for interaction: {}", service_with_package);
  
  let (file_descriptor, service_descriptor) = descriptor_cache.find_service_descriptor_for_type(service_with_package)?;
  let method_descriptor = find_method_descriptor_for_service(method_name, &service_descriptor)?;
  
  Ok((service_descriptor.clone(), method_descriptor.clone(), file_descriptor.clone()))
}

/// Lookup service and method descriptors for an interaction, parsing descriptors from the pact.
/// This function parses the descriptors from the pact and creates a new cache.
/// For performance-critical scenarios with multiple interactions, consider pre-parsing descriptors
/// and using `lookup_service_and_method_for_interaction` instead.
/// 
/// # Arguments
/// * `interaction` - The V4 interaction to lookup descriptors for
/// * `pact` - The V4 pact containing the descriptor configuration
/// 
/// # Returns
/// A tuple of:
/// - DescriptorCache - cached descriptor lookup structure (wrapped in Arc)
/// - ServiceDescriptorProto - the service descriptor for this gRPC service
/// - MethodDescriptorProto - the method descriptor for this gRPC service
/// - FileDescriptorProto - the file descriptor containing this gRPC service
pub(crate) fn lookup_service_descriptors_for_interaction(
  interaction: &dyn V4Interaction,
  pact: &V4Pact
) -> anyhow::Result<(Arc<DescriptorCache>, ServiceDescriptorProto, MethodDescriptorProto, FileDescriptorProto)> {
  // TODO: a similar flow happens in server::compare_contents, can it be refactored to a common function?
  // compare_contents works with both service and message, while this one only works with the service.
  let interaction_config = lookup_interaction_config(interaction)
    .ok_or_else(|| anyhow!("Interaction does not have any Protobuf configuration"))?;
  let descriptor_key = interaction_config.get("descriptorKey")
    .map(json_to_string)
    .ok_or_else(|| anyhow!("Interaction descriptorKey was missing in Pact file"))?;
  
  let plugin_config = lookup_plugin_config(pact)?;
  let fds = get_descriptors_for_interaction(descriptor_key.as_str(), &plugin_config)?;
  let descriptor_cache = Arc::new(DescriptorCache::new(fds));
  trace!("file descriptors for interaction {:?}", descriptor_cache.get_file_descriptors_vec());
  
  // Use the new function to do the actual lookup
  let (service_descriptor, method_descriptor, file_descriptor) = 
    lookup_service_and_method_for_interaction(interaction, &descriptor_cache)?;
  
  Ok((descriptor_cache, service_descriptor, method_descriptor, file_descriptor))
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
  use crate::utils::{as_hex, struct_field_data_to_json, find_nested_type, is_map_field, last_name, parse_name, to_fully_qualified_name, DescriptorCache};
  use super::{
    build_grpc_route,
    find_method_descriptor_for_service,
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
    let descriptor_cache = DescriptorCache::new(fds);

    expect!(descriptor_cache.find_message_descriptor_for_type("")).to(be_err());
    expect!(descriptor_cache.find_message_descriptor_for_type("Does not exist")).to(be_err());

    let (result, _) = descriptor_cache.find_message_descriptor_for_type("AdBreakRequest").unwrap();
    expect!(result.name).to(be_some().value("AdBreakRequest"));

    let (result, file_descriptor) = descriptor_cache.find_message_descriptor_for_type(".area_calculator.Value.AdBreakContext").unwrap();
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
    let file_descriptor_set = FileDescriptorSet {
      file: vec![fds.clone(), fds2.clone(), fds3.clone(), fds4.clone()]
    };
    let descriptor_cache = DescriptorCache::new(file_descriptor_set);

    let result = descriptor_cache.find_enum_value_by_name(".routeguide.v2.TestEnum", "VALUE_ONE");
    expect!(result).to(be_some().value((1, enum1.clone())));

    let result2 = descriptor_cache.find_enum_value_by_name(".routeguide.TestEnum", "VALUE_ONE");
    expect!(result2).to(be_some().value((1, enum1.clone())));

    let result3 = descriptor_cache.find_enum_value_by_name(".TestEnum", "VALUE_TWO");
    expect!(result3).to(be_some().value((2, enum1.clone())));

    let result4 = descriptor_cache.find_enum_value_by_name(".routeguide.v3.Feature.TestEnum", "VALUE_ONE");
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
    let descriptor_cache = DescriptorCache::new(all_descriptors);
    
    // fully qualified name
    let (md, fd) = descriptor_cache.find_message_descriptor_for_type(".service.Request").unwrap();
    expect!(&md).to(be_equal_to(&request_msg));
    expect!(&fd).to(be_equal_to(&request_file));

    // relative name
    let (md, fd) = descriptor_cache.find_message_descriptor_for_type("AnotherRequest").unwrap();
    expect!(&md).to(be_equal_to(&another_request_msg));
    expect!(&fd).to(be_equal_to(&request_file));

    // package not found error
    let result_err = descriptor_cache.find_message_descriptor_for_type(".missing.MissingType");
    expect!(result_err.as_ref()).to(be_err());
    expect!(&result_err.unwrap_err().to_string()).to(be_equal_to(
      "Did not find any file descriptors for a package 'missing'"));
    // message not found error
    let result_err = descriptor_cache.find_message_descriptor_for_type(".service.MissingType");
    expect!(result_err.as_ref()).to(be_err());
    let error_msg = result_err.unwrap_err().to_string();
    // Error message changed after refactoring - now shows which package was searched
    expect!(error_msg.contains("MissingType")).to(be_true());
    expect!(error_msg.contains("not found")).to(be_true());
  }

  #[test]
  fn find_message_descriptor_for_type_with_nested_messages_test() {
    // Test the new functionality that correctly handles nested messages
    // like .package.Message.NestedMessage by trying all possible package/message splits
    
    // Create a nested message structure: Entity.FullName and Entity.Details
    let full_name_msg = DescriptorProto {
      name: Some("FullName".to_string()),
      field: vec![
        FieldDescriptorProto {
          name: Some("first".to_string()),
          number: Some(1),
          r#type: Some(Type::String as i32),
          .. FieldDescriptorProto::default()
        },
        FieldDescriptorProto {
          name: Some("last".to_string()),
          number: Some(2),
          r#type: Some(Type::String as i32),
          .. FieldDescriptorProto::default()
        },
      ],
      .. DescriptorProto::default()
    };
    
    let details_msg = DescriptorProto {
      name: Some("Details".to_string()),
      field: vec![
        FieldDescriptorProto {
          name: Some("name".to_string()),
          number: Some(1),
          r#type: Some(Type::Message as i32),
          type_name: Some(".sample.Entity.FullName".to_string()),
          .. FieldDescriptorProto::default()
        },
      ],
      nested_type: vec![full_name_msg.clone()],
      .. DescriptorProto::default()
    };
    
    let entity_msg = DescriptorProto {
      name: Some("Entity".to_string()),
      field: vec![
        FieldDescriptorProto {
          name: Some("id".to_string()),
          number: Some(1),
          r#type: Some(Type::String as i32),
          .. FieldDescriptorProto::default()
        },
      ],
      nested_type: vec![details_msg.clone(), full_name_msg.clone()],
      .. DescriptorProto::default()
    };
    
    let request_msg = DescriptorProto {
      name: Some("GetEntityRequest".to_string()),
      field: vec![
        FieldDescriptorProto {
          name: Some("id".to_string()),
          number: Some(1),
          r#type: Some(Type::String as i32),
          .. FieldDescriptorProto::default()
        },
      ],
      .. DescriptorProto::default()
    };
    
    let file_descriptor = FileDescriptorProto {
      name: Some("sample.proto".to_string()),
      package: Some("sample".to_string()),
      message_type: vec![entity_msg.clone(), request_msg.clone()],
      .. FileDescriptorProto::default()
    };
    
    let fds = FileDescriptorSet { file: vec![file_descriptor.clone()] };
    let descriptor_cache = DescriptorCache::new(fds);
    
    // Test 1: Find top-level message with fully qualified name
    let (md, fd) = descriptor_cache.find_message_descriptor_for_type(".sample.Entity").unwrap();
    expect!(md.name()).to(be_equal_to("Entity"));
    expect!(fd.package()).to(be_equal_to("sample"));
    
    // Test 2: Find top-level message with relative name
    let (md, _) = descriptor_cache.find_message_descriptor_for_type("GetEntityRequest").unwrap();
    expect!(md.name()).to(be_equal_to("GetEntityRequest"));
    
    // Test 3: Find nested message with fully qualified name - this is the key test!
    // This used to fail because it would split at the last dot: package=".sample.Entity", message="FullName"
    // Now it tries all combinations and finds: package="sample", message="Entity.FullName"
    let (md, fd) = descriptor_cache.find_message_descriptor_for_type(".sample.Entity.FullName").unwrap();
    expect!(md.name()).to(be_equal_to("FullName"));
    expect!(fd.package()).to(be_equal_to("sample"));
    
    // Test 4: Find doubly nested message
    let (md, fd) = descriptor_cache.find_message_descriptor_for_type(".sample.Entity.Details").unwrap();
    expect!(md.name()).to(be_equal_to("Details"));
    expect!(fd.package()).to(be_equal_to("sample"));
    
    // Test 5: Relative name for nested message (should work by searching all files)
    let (md, _) = descriptor_cache.find_message_descriptor_for_type("Entity.FullName").unwrap();
    expect!(md.name()).to(be_equal_to("FullName"));
    
    // Test 6: Error case - message doesn't exist
    let result = descriptor_cache.find_message_descriptor_for_type(".sample.Entity.NonExistent");
    expect!(result).to(be_err());
    
    // Test 7: Error case - package doesn't exist
    let result = descriptor_cache.find_message_descriptor_for_type(".nonexistent.Entity");
    expect!(result).to(be_err());
  }

  #[test]
  fn find_message_descriptor_for_type_with_multiple_packages_and_nested_messages_test() {
    // Test with multiple packages to ensure the algorithm tries all combinations correctly
    
    let nested_msg = DescriptorProto {
      name: Some("Nested".to_string()),
      .. DescriptorProto::default()
    };
    
    let outer_msg = DescriptorProto {
      name: Some("Outer".to_string()),
      nested_type: vec![nested_msg.clone()],
      .. DescriptorProto::default()
    };
    
    // File 1: package "a.b" with message "Outer" containing "Nested"
    let file1 = FileDescriptorProto {
      name: Some("file1.proto".to_string()),
      package: Some("a.b".to_string()),
      message_type: vec![outer_msg.clone()],
      .. FileDescriptorProto::default()
    };
    
    // File 2: package "a" with a different message
    let other_msg = DescriptorProto {
      name: Some("Other".to_string()),
      .. DescriptorProto::default()
    };
    
    let file2 = FileDescriptorProto {
      name: Some("file2.proto".to_string()),
      package: Some("a".to_string()),
      message_type: vec![other_msg.clone()],
      .. FileDescriptorProto::default()
    };
    
    let fds = FileDescriptorSet { file: vec![file1.clone(), file2.clone()] };
    let descriptor_cache = DescriptorCache::new(fds);
    
    // Should find Outer in package "a.b"
    let (md, fd) = descriptor_cache.find_message_descriptor_for_type(".a.b.Outer").unwrap();
    expect!(md.name()).to(be_equal_to("Outer"));
    expect!(fd.package()).to(be_equal_to("a.b"));
    
    // Should find Nested as a nested message in Outer within package "a.b"
    // This tests the algorithm tries: package="a.b", message="Outer.Nested"
    let (md, fd) = descriptor_cache.find_message_descriptor_for_type(".a.b.Outer.Nested").unwrap();
    expect!(md.name()).to(be_equal_to("Nested"));
    expect!(fd.package()).to(be_equal_to("a.b"));
    
    // Should find Other in package "a"
    let (md, fd) = descriptor_cache.find_message_descriptor_for_type(".a.Other").unwrap();
    expect!(md.name()).to(be_equal_to("Other"));
    expect!(fd.package()).to(be_equal_to("a"));
  }

  #[test]
  fn find_message_descriptor_for_type_edge_cases_test() {
    // Test edge cases for the new algorithm
    
    // Edge case 1: Empty package
    let msg_no_package = DescriptorProto {
      name: Some("NoPackageMessage".to_string()),
      .. DescriptorProto::default()
    };
    
    let file_no_package = FileDescriptorProto {
      name: Some("no_package.proto".to_string()),
      package: None,
      message_type: vec![msg_no_package.clone()],
      .. FileDescriptorProto::default()
    };
    
    let fds = FileDescriptorSet { file: vec![file_no_package.clone()] };
    let descriptor_cache = DescriptorCache::new(fds);
    
    // Should find message with relative name
    let (md, _) = descriptor_cache.find_message_descriptor_for_type("NoPackageMessage").unwrap();
    expect!(md.name()).to(be_equal_to("NoPackageMessage"));
    
    // Should find message with fully qualified name starting with dot (empty package)
    let (md, _) = descriptor_cache.find_message_descriptor_for_type(".NoPackageMessage").unwrap();
    expect!(md.name()).to(be_equal_to("NoPackageMessage"));
    
    // Edge case 2: Deep nesting
    let level3_msg = DescriptorProto {
      name: Some("Level3".to_string()),
      .. DescriptorProto::default()
    };
    
    let level2_msg = DescriptorProto {
      name: Some("Level2".to_string()),
      nested_type: vec![level3_msg.clone()],
      .. DescriptorProto::default()
    };
    
    let level1_msg = DescriptorProto {
      name: Some("Level1".to_string()),
      nested_type: vec![level2_msg.clone()],
      .. DescriptorProto::default()
    };
    
    let deep_file = FileDescriptorProto {
      name: Some("deep.proto".to_string()),
      package: Some("pkg".to_string()),
      message_type: vec![level1_msg.clone()],
      .. FileDescriptorProto::default()
    };
    
    let fds = FileDescriptorSet { file: vec![deep_file.clone()] };
    let descriptor_cache = DescriptorCache::new(fds);
    
    // Should find deeply nested message
    let (md, _) = descriptor_cache.find_message_descriptor_for_type(".pkg.Level1.Level2.Level3").unwrap();
    expect!(md.name()).to(be_equal_to("Level3"));
    
    // Should also work with relative name
    let (md, _) = descriptor_cache.find_message_descriptor_for_type("Level1.Level2.Level3").unwrap();
    expect!(md.name()).to(be_equal_to("Level3"));
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
    let descriptor_cache = DescriptorCache::new(all_descriptors);

    let (fd, sd) = descriptor_cache.find_service_descriptor_for_type(".service.Service").unwrap();
    expect!(fd).to(be_equal_to(service));
    expect!(sd).to(be_equal_to(service_desc));

    let (fd, sd) = descriptor_cache.find_service_descriptor_for_type("RelativeNameService").unwrap();
    expect!(fd).to(be_equal_to(service2));
    expect!(sd).to(be_equal_to(relative_name_service));

    // missing package case
    let result_err = descriptor_cache.find_service_descriptor_for_type(".missing.MissingService");
    expect!(result_err.as_ref()).to(be_err());
    expect!(&result_err.unwrap_err().to_string()).to(be_equal_to(
      "Did not find any file descriptors for a package 'missing'"));
    // missing service case
    let result_err = descriptor_cache.find_service_descriptor_for_type(".service.MissingService");
    expect!(result_err.as_ref()).to(be_err());
    let error_msg = result_err.unwrap_err().to_string();
    // Error message changed after refactoring - now shows which package was searched
    expect!(error_msg.contains("MissingService")).to(be_true());
    expect!(error_msg.contains("not found")).to(be_true());
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
    
    let fds = FileDescriptorSet { file: all_descriptors.clone() };
    let descriptor_cache = DescriptorCache::new(fds);
    
    // explicitly provide package name
    _check_find_file_descriptors(Some("service"), &all_descriptors_with_package_names, &descriptor_cache);

    // same but with a dot
    _check_find_file_descriptors(Some(".service"), &all_descriptors_with_package_names, &descriptor_cache);

    // empty package means return descriptors without packages only
    _check_find_file_descriptors(Some(""), &all_descriptors_with_no_pacakge_names, &descriptor_cache);

    // none package means return all descriptors
    _check_find_file_descriptors(None, &all_descritptor_names, &descriptor_cache);

    // Errors
    // did not find any file descriptor with specified package
    let result = descriptor_cache.find_file_descriptors(Some("missing"));
    expect!(result.as_ref()).to(be_err());
    expect!(&result.unwrap_err().to_string()).to(be_equal_to("Did not find any file descriptors for a package 'missing'"));
    
    // did not find any file descriptors with no package
    let empty_fds = FileDescriptorSet { file: vec![] };
    let empty_cache = DescriptorCache::new(empty_fds);
    let result = empty_cache.find_file_descriptors(Some(""));
    expect!(&result.unwrap_err().to_string()).to(be_equal_to("Did not find any file descriptors with no package specified"));
  }

  fn _check_find_file_descriptors(
    package: Option<&str>,
    expected: &HashSet<String>,
    descriptor_cache: &DescriptorCache
  ) {
    let actual = descriptor_cache.find_file_descriptors(package).unwrap().iter()
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
    let descriptor_cache = DescriptorCache::new(fds);

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

    let result = struct_field_data_to_json(field_data, &field_descriptor, &descriptor_cache).unwrap();
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

    let result = struct_field_data_to_json(field_data, &field_descriptor, &descriptor_cache).unwrap();
    assert_eq!(result, json!({
      "message": "test",
      "kind": "general"
    }));
  }

  #[test]
  fn find_enum_value_by_name_deep_nested_test() {
    let top_level_enum = EnumDescriptorProto {
      name: Some("TopLevelEnum".to_string()),
      value: vec![
        EnumValueDescriptorProto {
          name: Some("VALUE_ONE".to_string()),
          number: Some(1),
          options: None,
        },
      ],
      .. EnumDescriptorProto::default()
    };

    let one_level_enum = EnumDescriptorProto {
      name: Some("OneLevelEnum".to_string()),
      value: vec![
        EnumValueDescriptorProto {
          name: Some("VALUE_A".to_string()),
          number: Some(1),
          options: None,
        },
      ],
      .. EnumDescriptorProto::default()
    };

    let two_level_enum = EnumDescriptorProto {
      name: Some("TwoLevelEnum".to_string()),
      value: vec![
        EnumValueDescriptorProto {
          name: Some("VALUE_X".to_string()),
          number: Some(1),
          options: None,
        },
      ],
      .. EnumDescriptorProto::default()
    };

    let three_level_enum = EnumDescriptorProto {
      name: Some("ThreeLevelEnum".to_string()),
      value: vec![
        EnumValueDescriptorProto {
          name: Some("VALUE_DEEP".to_string()),
          number: Some(1),
          options: None,
        },
      ],
      .. EnumDescriptorProto::default()
    };

    let deep_message = DescriptorProto {
      name: Some("DeepMessage".to_string()),
      enum_type: vec![three_level_enum.clone()],
      .. DescriptorProto::default()
    };

    let inner_message = DescriptorProto {
      name: Some("InnerMessage".to_string()),
      enum_type: vec![two_level_enum.clone()],
      nested_type: vec![deep_message.clone()],
      .. DescriptorProto::default()
    };

    let outer_message = DescriptorProto {
      name: Some("OuterMessage".to_string()),
      enum_type: vec![one_level_enum.clone()],
      nested_type: vec![inner_message.clone()],
      .. DescriptorProto::default()
    };

    let file_descriptor = FileDescriptorProto {
      name: Some("nested_enum.proto".to_string()),
      package: Some("test".to_string()),
      message_type: vec![outer_message.clone()],
      enum_type: vec![top_level_enum.clone()],
      .. FileDescriptorProto::default()
    };

    let file_descriptor_set = FileDescriptorSet {
      file: vec![file_descriptor.clone()]
    };
    let descriptor_cache = DescriptorCache::new(file_descriptor_set);

    // 1. Top-level enum (0 levels deep)
    let result1 = descriptor_cache.find_enum_value_by_name(".test.TopLevelEnum", "VALUE_ONE");
    expect!(result1).to(be_some().value((1, top_level_enum.clone())));

    // 2. One-level nested enum
    let result2 = descriptor_cache.find_enum_value_by_name(".test.OuterMessage.OneLevelEnum", "VALUE_A");
    expect!(result2).to(be_some().value((1, one_level_enum.clone())));

    // 3. Two-level nested enum
    let result3 = descriptor_cache.find_enum_value_by_name(".test.OuterMessage.InnerMessage.TwoLevelEnum", "VALUE_X");
    expect!(result3).to(be_some().value((1, two_level_enum.clone())));

    // 4. Three-level nested enum
    let result4 = descriptor_cache.find_enum_value_by_name(".test.OuterMessage.InnerMessage.DeepMessage.ThreeLevelEnum", "VALUE_DEEP");
    expect!(result4).to(be_some().value((1, three_level_enum.clone())));

  }

  #[test]
  fn test_message_name_collision_with_and_without_package() {
    // This test reproduces the imported_without_package integration test scenario:
    // Two messages with the same simple name "Tag", but different FQNs:
    // - .imported.Tag (has package "imported")
    // - .Tag (NO package, empty string)
    
    // Create Tag message in the "imported" package
    let imported_tag_message = DescriptorProto {
      name: Some("Tag".to_string()),
      field: vec![
        FieldDescriptorProto {
          name: Some("some_value".to_string()),
          number: Some(1),
          r#type: Some(Type::String as i32),
          .. FieldDescriptorProto::default()
        },
        FieldDescriptorProto {
          name: Some("some_bool".to_string()),
          number: Some(2),
          r#type: Some(Type::Bool as i32),
          .. FieldDescriptorProto::default()
        },
      ],
      .. DescriptorProto::default()
    };
    
    // Create Tag message with NO package
    let no_package_tag_message = DescriptorProto {
      name: Some("Tag".to_string()),
      field: vec![
        FieldDescriptorProto {
          name: Some("name".to_string()),
          number: Some(1),
          r#type: Some(Type::String as i32),
          .. FieldDescriptorProto::default()
        },
        FieldDescriptorProto {
          name: Some("value".to_string()),
          number: Some(2),
          r#type: Some(Type::String as i32),
          .. FieldDescriptorProto::default()
        },
      ],
      .. DescriptorProto::default()
    };
    
    // File descriptor for imported.proto (has package)
    let imported_file = FileDescriptorProto {
      name: Some("imported/imported.proto".to_string()),
      package: Some("imported".to_string()),
      message_type: vec![imported_tag_message.clone()],
      .. FileDescriptorProto::default()
    };
    
    // File descriptor for tag.proto (NO package)
    let no_package_file = FileDescriptorProto {
      name: Some("no_package/tag.proto".to_string()),
      package: None, // No package!
      message_type: vec![no_package_tag_message.clone()],
      .. FileDescriptorProto::default()
    };
    
    let file_descriptor_set = FileDescriptorSet {
      file: vec![imported_file.clone(), no_package_file.clone()]
    };
    let descriptor_cache = DescriptorCache::new(file_descriptor_set);
    
    // Test 1: Looking up .imported.Tag should find the message with "some_value" and "some_bool"
    let (message_desc, _) = descriptor_cache.find_message_descriptor_for_type(".imported.Tag").unwrap();
    expect!(message_desc.field.len()).to(be_equal_to(2));
    expect!(message_desc.field[0].name.as_ref().unwrap().as_str()).to(be_equal_to("some_value"));
    expect!(message_desc.field[1].name.as_ref().unwrap().as_str()).to(be_equal_to("some_bool"));
    
    // Test 2: Looking up .Tag (no package) should find the message with "name" and "value"
    // THIS IS THE CRITICAL TEST - it should find the Tag from no_package, not from imported!
    let (message_desc, _) = descriptor_cache.find_message_descriptor_for_type(".Tag").unwrap();
    expect!(message_desc.field.len()).to(be_equal_to(2));
    expect!(message_desc.field[0].name.as_ref().unwrap().as_str()).to(be_equal_to("name"));
    expect!(message_desc.field[1].name.as_ref().unwrap().as_str()).to(be_equal_to("value"));
  }
}

