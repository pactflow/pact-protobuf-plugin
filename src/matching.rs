//! Functions for matching Protobuf messages

use std::collections::BTreeMap;
use std::fmt::{Debug, Display, Formatter};

use anyhow::anyhow;
use bytes::{Bytes, BytesMut};
use itertools::Itertools;
use maplit::hashmap;
use pact_matching::{BodyMatchResult, CoreMatchingContext, DiffConfig, MatchingContext, Mismatch};
use pact_matching::json::compare_json;
use pact_matching::matchers::{match_values, Matches};
use pact_matching::matchingrules::{compare_lists_with_matchingrule, compare_maps_with_matchingrule};
use pact_matching::Mismatch::BodyMismatch;
use pact_models::content_types::ContentType;
use pact_models::matchingrules::MatchingRule;
use pact_models::path_exp::DocPath;
use pact_models::prelude::MatchingRuleCategory;
use prost_types::{DescriptorProto, FieldDescriptorProto, FileDescriptorSet};
use tracing::{debug, trace, warn};

use crate::message_decoder::{decode_message, ProtobufField, ProtobufFieldData};
use crate::utils::{display_bytes, enum_name, field_data_to_json, find_message_field_by_name, find_message_type_by_name, find_service_descriptor, is_map_field, is_repeated_field, last_name};

/// Match a single Protobuf message
pub fn match_message(
  message_name: &str,
  descriptors: &FileDescriptorSet,
  expected_request: &mut Bytes,
  actual_request: &mut Bytes,
  matching_rules: &MatchingRuleCategory,
  allow_unexpected_keys: bool
) -> anyhow::Result<BodyMatchResult> {
  debug!("Looking for message '{}'", message_name);
  let message_descriptor = find_message_type_by_name(message_name, descriptors)?;

  let expected_message = decode_message(expected_request, &message_descriptor, descriptors)?;
  debug!("expected message = {:?}", expected_message);

  let actual_message = decode_message(actual_request, &message_descriptor, descriptors)?;
  debug!("actual message = {:?}", actual_message);

  let plugin_config = hashmap!{};
  let diff_config = if allow_unexpected_keys {
    DiffConfig::AllowUnexpectedKeys
  } else {
    DiffConfig::NoUnexpectedKeys
  };
  let context = CoreMatchingContext::new(diff_config, matching_rules, &plugin_config);

  compare(&message_descriptor, &expected_message, &actual_message, &context,
          expected_request, descriptors)
}

/// Match a Protobuf service call, which has an input and output message
pub fn match_service(
  service_name: &str,
  method_name: &str,
  descriptors: &FileDescriptorSet,
  expected_request: &mut Bytes,
  actual_request: &mut Bytes,
  rules: &MatchingRuleCategory,
  allow_unexpected_keys: bool,
  content_type: &ContentType
) -> anyhow::Result<BodyMatchResult> {
  debug!("Looking for service '{}'", service_name);
  let (_, service_descriptor) = find_service_descriptor(descriptors, service_name)?;
  trace!("Found service descriptor with name {:?}", service_descriptor.name);

  let (method_name, service_part) = if method_name.contains(':') {
    method_name.split_once(':').unwrap_or((method_name, ""))
  } else {
    (method_name, "")
  };
  let method_descriptor = service_descriptor.method.iter().find(|method_desc| {
    method_desc.name.clone().unwrap_or_default() == method_name
  }).ok_or_else(|| anyhow!("Did not find the method {} in the Protobuf file descriptor for service '{}'", method_name, service_name))?;
  trace!("Found method descriptor with name {:?}", method_descriptor.name);

  let expected_message_type = content_type.attributes.get("message");
  let message_type = if let Some(message_type) = expected_message_type {
    let input_type = method_descriptor.input_type.clone().unwrap_or_default();
    if last_name(input_type.as_str()) == message_type.as_str() {
      input_type
    } else {
      method_descriptor.output_type.clone().unwrap_or_default()
    }
  } else if service_part == "request" {
    method_descriptor.input_type.clone().unwrap_or_default()
  } else {
    method_descriptor.output_type.clone().unwrap_or_default()
  };

  trace!("Message type = {}", message_type);
  match_message(last_name(message_type.as_str()), descriptors,
                expected_request, actual_request,
                rules, allow_unexpected_keys)
}

/// Compare the expected message to the actual one
pub(crate) fn compare(
  message_descriptor: &DescriptorProto,
  expected_message: &[ProtobufField],
  actual_message: &[ProtobufField],
  matching_context: &dyn MatchingContext,
  expected_message_bytes: &Bytes,
  descriptors: &FileDescriptorSet
) -> anyhow::Result<BodyMatchResult> {
  if expected_message.is_empty() {
    Ok(BodyMatchResult::Ok)
  } else if actual_message.is_empty() {
    Ok(BodyMatchResult::BodyMismatches(hashmap!{
      "$".to_string() => vec![Mismatch::BodyMismatch {
        path: "$".to_string(),
        expected: Some(expected_message_bytes.clone()),
        actual: None,
        mismatch: format!("Expected message '{}' but was missing or empty", message_descriptor.name.clone().unwrap_or_default())
      }]
    }))
  } else {
    compare_message(DocPath::root(), expected_message, actual_message, matching_context, message_descriptor, descriptors)
  }
}

/// Compare the fields of the expected and actual messages
fn compare_message(
  path: DocPath,
  expected_message_fields: &[ProtobufField],
  actual_message_fields: &[ProtobufField],
  matching_context: &dyn MatchingContext,
  message_descriptor: &DescriptorProto,
  descriptors: &FileDescriptorSet,
) -> anyhow::Result<BodyMatchResult> {
  trace!(">> compare_message({}, {:?}, {:?})", path, expected_message_fields, actual_message_fields);

  let mut results = hashmap!{};

  let fields = message_descriptor.field.iter()
    .filter_map(|field| {
      field.number.map(|no| {
        let expected_field_values = expected_message_fields.iter().filter(|value| value.field_num == no as u32)
          .collect_vec();
        let actual_field_values = actual_message_fields.iter().filter(|value| value.field_num == no as u32)
          .collect_vec();
        (no as u32, (field, expected_field_values, actual_field_values))
      })
    });

  for (field_no, (field_descriptor, expected, actual)) in fields {
    let field_name = field_descriptor.name
      .clone()
      .unwrap_or_else(|| {
        warn!("Field number {} does not have a field name in the descriptor, will use the number", field_no);
        field_no.to_string()
      });
    let field_path = path.join(&field_name);
    trace!("Comparing message field {}:{}", field_name, field_no);

    if is_map_field(message_descriptor, field_descriptor) {
      let map_comparison = compare_map_field(&field_path, field_descriptor, expected, actual, matching_context, descriptors);
      if !map_comparison.is_empty() {
        results.insert(field_path.to_string(), map_comparison);
      }
    } else if is_repeated_field(field_descriptor) {
      let e = expected.iter().map(|f| (*f).clone()).collect_vec();
      let a = actual.iter().map(|f| (*f).clone()).collect_vec();
      let repeated_comparison = compare_repeated_field(&field_path, field_descriptor, &e, &a, matching_context, descriptors);
      if !repeated_comparison.is_empty() {
        results.insert(field_path.to_string(), repeated_comparison);
      }
    } else if !expected.is_empty() && actual.is_empty() {
      results.insert(field_path.to_string(), vec![
        BodyMismatch {
          path: field_path.to_string(),
          expected: expected.first().map(|field_data| Bytes::from(field_data.data.as_bytes())),
          actual: None,
          mismatch: format!("Expected message field '{}' but was missing", field_name)
        }
      ]);
    } else if let Some(expected_value) = expected.first() {
      let comparison = compare_field(&field_path, *expected_value, field_descriptor, *actual.first().unwrap(), matching_context, descriptors);
      if !comparison.is_empty() {
        results.insert(field_path.to_string(), comparison);
      }
    } else if !actual.is_empty() && matching_context.config() == DiffConfig::NoUnexpectedKeys {
      results.insert(field_path.to_string(), vec![
        BodyMismatch {
          path: field_path.to_string(),
          expected: None,
          actual: actual.first().map(|field_data| Bytes::from(field_data.data.as_bytes())),
          mismatch: format!("Expected field '{}' to be missing, but received a value for it", field_name)
        }
      ]);
    }
  }

  if results.is_empty() {
    Ok(BodyMatchResult::Ok)
  } else {
    Ok(BodyMatchResult::BodyMismatches(results))
  }
}

/// Compare a simple field (non-map and non-repeated)
fn compare_field(
  path: &DocPath,
  field: &ProtobufField,
  descriptor: &FieldDescriptorProto,
  actual: &ProtobufField,
  matching_context: &dyn MatchingContext,
  descriptors: &FileDescriptorSet
) -> Vec<Mismatch> {
  trace!("compare_field({}, {:?}, {:?}, {:?})", path, field, descriptor, actual);

  match (&field.data, &actual.data) {
    (ProtobufFieldData::String(s1), ProtobufFieldData::String(s2)) => {
      trace!("Comparing string values");
      let s1 = s1.clone();
      let s2 = s2.clone();
      compare_value(path, field, &s1, &s2, s1.as_str(), s2.as_str(), matching_context)
    },
    (ProtobufFieldData::Boolean(b1), ProtobufFieldData::Boolean(b2)) => {
      trace!("Comparing boolean values");
      compare_value(path, field, *b1, *b2, b1.to_string().as_str(), b2.to_string().as_str(), matching_context)
    },
    (ProtobufFieldData::UInteger32(n1), ProtobufFieldData::UInteger32(n2)) => {
      trace!("Comparing UInteger32 values");
      compare_value(path, field, *n1 as u64, *n2 as u64, n1.to_string().as_str(), n2.to_string().as_str(), matching_context)
    },
    (ProtobufFieldData::Integer32(n1), ProtobufFieldData::Integer32(n2)) => {
      trace!("Comparing Integer32 values");
      compare_value(path, field, *n1, *n2, n1.to_string().as_str(), n2.to_string().as_str(), matching_context)
    },
    (ProtobufFieldData::UInteger64(n1), ProtobufFieldData::UInteger64(n2)) => {
      trace!("Comparing UInteger64 values");
      compare_value(path, field, *n1, *n2, n1.to_string().as_str(), n2.to_string().as_str(), matching_context)
    },
    (ProtobufFieldData::Integer64(n1), ProtobufFieldData::Integer64(n2)) => {
      trace!("Comparing Integer64 values");
      compare_value(path, field, *n1, *n2, n1.to_string().as_str(), n2.to_string().as_str(), matching_context)
    },
    (ProtobufFieldData::Float(n1), ProtobufFieldData::Float(n2)) => {
      trace!("Comparing Float values");
      compare_value(path, field, *n1 as f64, *n2 as f64, n1.to_string().as_str(), n2.to_string().as_str(), matching_context)
    },
    (ProtobufFieldData::Double(n1), ProtobufFieldData::Double(n2)) => {
      trace!("Comparing Double values");
      compare_value(path, field, *n1, *n2, n1.to_string().as_str(), n2.to_string().as_str(), matching_context)
    },
    (ProtobufFieldData::Bytes(b1), ProtobufFieldData::Bytes(b2)) => {
      trace!("Comparing byte arrays");
      let b1_str = display_bytes(b1);
      let b2_str = display_bytes(b2);
      compare_value(path, field, b1.as_slice(), b2.as_slice(), b1_str.as_str(), b2_str.as_str(), matching_context)
    },
    (ProtobufFieldData::Enum(b1, descriptor), ProtobufFieldData::Enum(b2, _)) => {
      trace!("Comparing Enum values");
      let enum_1 = enum_name(*b1, descriptor);
      let enum_2 = enum_name(*b2, descriptor);
      compare_value(path, field, &enum_1, &enum_2, enum_1.as_str(), enum_2.as_str(), matching_context)
    },
    (ProtobufFieldData::Message(b1, message_descriptor), ProtobufFieldData::Message(b2, _)) => {
      trace!("Comparing embedded messages");
      let mut expected_bytes = BytesMut::from(b1.as_slice());
      let expected_message = match decode_message(&mut expected_bytes, message_descriptor, descriptors) {
        Ok(message) => message,
        Err(err) => {
          return vec![
            BodyMismatch {
              path: path.to_string(),
              expected: Some(field.data.to_string().into()),
              actual: Some(actual.data.to_string().into()),
              mismatch: format!("Could not decode expected message field {} - {}", descriptor.name.clone().unwrap_or_else(|| "unknown".to_string()), err)
            }
          ];
        }
      };
      let mut actual_bytes = BytesMut::from(b2.as_slice());
      let actual_message = match decode_message(&mut actual_bytes, message_descriptor, descriptors) {
        Ok(message) => message,
        Err(err) => {
          return vec![
            BodyMismatch {
              path: path.to_string(),
              expected: Some(field.data.to_string().into()),
              actual: Some(actual.data.to_string().into()),
              mismatch: format!("Could not decode actual message field {} - {}", descriptor.name.clone().unwrap_or_else(|| "unknown".to_string()), err)
            }
          ];
        }
      };
      match &descriptor.type_name {
        Some(name) => match name.as_str() {
          ".google.protobuf.BytesValue" => {
            debug!("Field is a Protobuf BytesValue");
            let expected_field_data = find_message_field_by_name(message_descriptor, expected_message, "value");
            let actual_field_data = find_message_field_by_name(message_descriptor, actual_message, "value");
            let b1 = expected_field_data.map(|f| match f.data {
              ProtobufFieldData::Bytes(b) => b,
              _ => vec![]
            }).unwrap_or_default();
            let b1_str = display_bytes(&b1);
            let b2 = actual_field_data.map(|f| match f.data {
              ProtobufFieldData::Bytes(b) => b,
              _ => vec![]
            }).unwrap_or_default();
            let b2_str = display_bytes(&b2);
            compare_value(path, field, b1, b2, b1_str.as_str(), b2_str.as_str(), matching_context)
          }
          ".google.protobuf.Struct" => {
            debug!("Field is a Protobuf Struct, will compare it as JSON");
            let expected_json = match field_data_to_json(expected_message, message_descriptor, descriptors) {
              Ok(j) => j,
              Err(err) => {
                return vec![
                  BodyMismatch {
                    path: path.to_string(),
                    expected: Some(field.data.to_string().into()),
                    actual: Some(actual.data.to_string().into()),
                    mismatch: format!("Could not decode expected message field {} - {}", descriptor.name.clone().unwrap_or_else(|| "unknown".to_string()), err)
                  }
                ];
              }
            };
            let actual_json = match field_data_to_json(actual_message, message_descriptor, descriptors) {
              Ok(j) => j,
              Err(err) => {
                return vec![
                  BodyMismatch {
                    path: path.to_string(),
                    expected: Some(field.data.to_string().into()),
                    actual: Some(actual.data.to_string().into()),
                    mismatch: format!("Could not decode actual message field {} - {}", descriptor.name.clone().unwrap_or_else(|| "unknown".to_string()), err)
                  }
                ];
              }
            };

            match compare_json(path, &expected_json, &actual_json, matching_context) {
              Ok(_) => vec![],
              Err(err) => err
            }
          }
          _ => {
            debug!("Field is a normal message");
            match compare_message(path.clone(), &expected_message, &actual_message, matching_context, message_descriptor, descriptors) {
              Ok(result) => match result {
                BodyMatchResult::Ok => vec![],
                BodyMatchResult::BodyTypeMismatch { message, .. } => vec![
                  BodyMismatch {
                    path: path.to_string(),
                    expected: Some(name.clone().into()),
                    actual: Some(name.clone().into()),
                    mismatch: message
                  }
                ],
                BodyMatchResult::BodyMismatches(mismatches) => mismatches.values().cloned().flatten().collect()
              }
              Err(err) => vec![
                BodyMismatch {
                  path: path.to_string(),
                  expected: Some(name.clone().into()),
                  actual: Some(name.clone().into()),
                  mismatch: err.to_string()
                }
              ]
            }
          }
        }
        None => vec![
          BodyMismatch {
            path: path.to_string(),
            expected: Some(field.data.to_string().into()),
            actual: Some(actual.data.to_string().into()),
            mismatch: format!("Message field {} type name is not set, can not compare it", descriptor.name.clone().unwrap_or_else(|| "unknown".to_string()))
          }
        ]
      }
    }
    _ => vec![
      BodyMismatch {
        path: path.to_string(),
        expected: Some(field.data.to_string().into()),
        actual: Some(actual.data.to_string().into()),
        mismatch: format!("Expected and actual field have different types: {} and {}",
                          field, actual)
      }
    ]
  }
}

/// Compares the actual value to the expected one.
fn compare_value<T>(
  path: &DocPath,
  field: &ProtobufField,
  expected: T,
  actual: T,
  expected_str: &str,
  actual_str: &str,
  matching_context: &dyn MatchingContext
) -> Vec<Mismatch> where T: Clone + Debug + Matches<T> {
  trace!("compare_value({}, {:?}, {}, {})", path, field, expected_str, actual_str);

  if matching_context.matcher_is_defined(path) {
    debug!("compare_value: Matcher defined for path '{}' and values {:?} -> {:?}", path, expected, actual);
    match match_values(path, &matching_context.select_best_matcher(path), expected, actual) {
      Ok(_) => vec![],
      Err(mismatches) => mismatches.iter().map(|m| BodyMismatch {
        path: path.to_string(),
        expected: Some(expected_str.as_bytes().to_vec().into()),
        actual: Some(actual_str.as_bytes().to_vec().into()),
        mismatch: m.clone()
      }).collect()
    }
  } else {
    debug!("compare_value: No matcher defined for path '{}', using equality", path);
    match expected.matches_with(actual, &MatchingRule::Equality, false) {
      Ok(_) => vec![],
      Err(err) => vec![BodyMismatch {
        path: path.to_string(),
        expected: Some(expected_str.as_bytes().to_vec().into()),
        actual: Some(actual_str.as_bytes().to_vec().into()),
        mismatch: err.to_string()
      }]
    }
  }
}

/// Compare a repeated field
fn compare_repeated_field(
  path: &DocPath,
  descriptor: &FieldDescriptorProto,
  expected_fields: &[ProtobufField],
  actual_fields: &[ProtobufField],
  matching_context: &dyn MatchingContext,
  descriptors: &FileDescriptorSet
) -> Vec<Mismatch> {
  trace!(">>> compare_repeated_field({}, {:?}, {:?})", path, expected_fields, actual_fields);

  let mut result = vec![];

  if matching_context.matcher_is_defined(path) {
    debug!("compare_repeated_field: Matcher defined for path '{}'", path);
    let rules = matching_context.select_best_matcher(path);
    for matcher in &rules.rules {
      if let Err(comparison) = compare_lists_with_matchingrule(matcher, path,
        expected_fields, actual_fields, matching_context, rules.cascaded, &mut |field_path, expected, actual, context| {
          let comparison = compare_field(field_path, expected, descriptor, actual, context, descriptors);
          if comparison.is_empty() {
            Ok(())
          } else {
            Err(comparison)
          }
        }) {
        result.extend(comparison);
      }
    }
  } else if expected_fields.is_empty() && !actual_fields.is_empty() {
    result.push(Mismatch::BodyMismatch {
      path: path.to_string(),
      expected: None,
      actual: None,
      mismatch: format!("Expected repeated field '{}' to be empty but received {} values",
        descriptor.name.clone().unwrap_or_else(|| descriptor.number.unwrap_or_default().to_string()),
        actual_fields.len()
      )
    })
  } else {
    result.extend(compare_list_content(path, descriptor, expected_fields, actual_fields, matching_context, descriptors));
    if expected_fields.len() != actual_fields.len() {
      result.push(Mismatch::BodyMismatch {
        path: path.to_string(),
        expected: None,
        actual: None,
        mismatch: format!("Expected repeated field '{}' to have {} values but received {} values",
          descriptor.name.clone().unwrap_or_else(|| descriptor.number.unwrap_or_default().to_string()),
          expected_fields.len(),
          actual_fields.len()
        )
      })
    }
  }

  result
}

/// Compare a map field
fn compare_map_field(
  path: &DocPath,
  descriptor: &FieldDescriptorProto,
  expected_fields: Vec<&ProtobufField>,
  actual_fields: Vec<&ProtobufField>,
  matching_context: &dyn MatchingContext,
  descriptors: &FileDescriptorSet
) -> Vec<Mismatch> {
  trace!(">> compare_map_field('{}', {:?}, {:?})", path, expected_fields, actual_fields);

  let mut result = vec![];

  if expected_fields.is_empty() && !actual_fields.is_empty() && matching_context.config() == DiffConfig::NoUnexpectedKeys {
    result.push(Mismatch::BodyMismatch {
      path: path.to_string(),
      expected: None,
      actual: None,
      mismatch: format!("Expected repeated field '{}' to be empty but received {} values",
        descriptor.name.clone().unwrap_or_else(|| descriptor.number.unwrap_or_default().to_string()),
        actual_fields.len()
      )
    });
  } else {
    let expected_map = expected_fields.iter()
      .filter_map(|f| {
        match &f.data {
          ProtobufFieldData::Message(d, descriptor) => decode_message_map_entry(descriptor, d, descriptors).ok(),
          _ => None
        }
      })
      .collect::<BTreeMap<String, MapEntry>>();
    let actual_map = actual_fields.iter()
      .filter_map(|f| {
        match &f.data {
          ProtobufFieldData::Message(d, descriptor) => decode_message_map_entry(descriptor, d, descriptors).ok(),
          _ => None
        }
      })
      .collect::<BTreeMap<String, MapEntry>>();

    if matching_context.matcher_is_defined(path) {
      debug!("compare_map_field: matcher defined for path '{}'", path);
      let rules = matching_context.select_best_matcher(path);
      for matcher in &rules.rules {
        trace!("compare_map_field: matcher = {:?}", matcher);
        if let Err(comparison) = compare_maps_with_matchingrule(matcher, rules.cascaded, path,
          &expected_map, &actual_map, matching_context, &mut |field_path, expected, actual| {
            let comparison = compare_field(field_path, &expected.value, &expected.field_descriptor, &actual.value, matching_context, descriptors);
            if comparison.is_empty() {
              Ok(())
            } else {
              Err(comparison)
            }
          }) {
          result.extend(comparison);
        }
      }
    } else {
      debug!("compare_map_field: no matcher defined for path '{}'", path);
      debug!("                   expected keys {:?}", expected_map.keys());
      debug!("                   actual keys {:?}", actual_map.keys());
      let expected_keys = expected_map.keys().cloned().collect();
      let actual_keys = actual_map.keys().cloned().collect();
      if let Err(mismatches) = matching_context.match_keys(path, &expected_keys, &actual_keys) {
        result.extend(mismatches);
      }
      for (key, value) in &expected_map {
        let entry_path = path.join(key);
        if let Some(actual) = actual_map.get(key.as_str()) {
          result.extend(compare_field(&entry_path, &value.value, &value.field_descriptor, &actual.value, matching_context, descriptors));
        } else {
          result.push(Mismatch::BodyMismatch {
            path: path.to_string(),
            expected: None,
            actual: None,
            mismatch: format!("Expected map field '{}' to have entry '{}', but was missing",
              descriptor.name.clone().unwrap_or_else(|| descriptor.number.unwrap_or_default().to_string()),
              key
            )
          });
        }
      }
    }
  }

  result
}

/// Struct to represent a protobuf map entry
#[derive(Clone, Debug)]
struct MapEntry {
  pub field_descriptor: FieldDescriptorProto,
  pub value: ProtobufField
}

impl Display for MapEntry {
  fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
    write!(f, "{}", self.value)
  }
}

/// Decodes an embedded entry message into a key and value part
fn decode_message_map_entry(
  descriptor: &DescriptorProto,
  data: &[u8],
  descriptors: &FileDescriptorSet
) -> anyhow::Result<(String, MapEntry)> {
  let message = decode_message(&mut BytesMut::from(data), descriptor, descriptors)?;
  let key = message.iter().find(|field| field.field_num == 1)
    .ok_or_else(|| anyhow!("Did not find the key value when decoding map entry {}", descriptor.name.clone().unwrap_or_else(|| "unknown".to_string())))?;
  let value = message.iter().find(|field| field.field_num == 2)
    .ok_or_else(|| anyhow!("Did not find the value value when decoding map entry {}", descriptor.name.clone().unwrap_or_else(|| "unknown".to_string())))?;
  let value_descriptor = find_field_descriptor(value, descriptor)
    .ok_or_else(|| anyhow!("Did not find the field descriptor for the value field of the map entry"))?;
  let key_str = match &key.data {
    ProtobufFieldData::String(s) => s.clone(),
    _ => key.data.to_string()
  };
  Ok((key_str, MapEntry { field_descriptor: value_descriptor, value: value.clone() } ))
}

/// Compares the items in the actual list against the expected
fn compare_list_content(
  path: &DocPath,
  descriptor: &FieldDescriptorProto,
  expected: &[ProtobufField],
  actual: &[ProtobufField],
  matching_context: &dyn MatchingContext,
  descriptors: &FileDescriptorSet
) -> Vec<Mismatch> {
  let mut result = vec![];
  for (index, value) in expected.iter().enumerate() {
    let ps = index.to_string();
    debug!("Comparing list item {} with value '{:?}' to '{:?}'", index, actual.get(index), value);
    let p = path.join(ps);
    if index < actual.len() {
      result.extend(compare_field(&p, value, descriptor, actual.get(index).unwrap(), matching_context, descriptors));
    } else if !matching_context.matcher_is_defined(&p) {
      result.push(Mismatch::BodyMismatch {
        path: path.to_string(),
        expected: Some(Bytes::from(value.data.to_string())),
        actual: None,
        mismatch: format!("Expected field {}({})={} but was missing", descriptor.name.clone().unwrap_or_default(), value.field_num, value.data)
      });
    }
  }
  result
}

/// Find the field descriptor in the message descriptor for the given field value
fn find_field_descriptor(field: &ProtobufField, descriptor: &DescriptorProto) -> Option<FieldDescriptorProto> {
  descriptor.field.iter()
    .find(|field_desc| field_desc.number.unwrap_or_default() == field.field_num as i32)
    .cloned()
}
