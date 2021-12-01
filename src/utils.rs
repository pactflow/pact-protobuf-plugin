//! Shared utilities

use std::collections::BTreeMap;

use prost_types::Value;

/// Return the last name in a dot separated string
pub fn last_name(entry_type_name: &str) -> &str {
  entry_type_name.split('.').last().unwrap_or_else(|| entry_type_name)
}

/// Convert a Protobuf Struct to a BTree Map
pub fn proto_struct_to_btreemap(val: &prost_types::Struct) -> BTreeMap<String, Value> {
  val.fields.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}
