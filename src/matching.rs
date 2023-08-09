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
  let (message_descriptor, _) = find_message_type_by_name(message_name, descriptors)?;

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
#[tracing::instrument(ret, skip_all)]
pub(crate) fn compare(
  message_descriptor: &DescriptorProto,
  expected_message: &[ProtobufField],
  actual_message: &[ProtobufField],
  matching_context: &(dyn MatchingContext + Send + Sync),
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
#[tracing::instrument(ret,
  skip_all,
  fields(%path)
)]
pub fn compare_message(
  path: DocPath,
  expected_message_fields: &[ProtobufField],
  actual_message_fields: &[ProtobufField],
  matching_context: &(dyn MatchingContext + Send + Sync),
  message_descriptor: &DescriptorProto,
  descriptors: &FileDescriptorSet,
) -> anyhow::Result<BodyMatchResult> {
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
    trace!(%field_name, field_no, "Comparing message field {:?} => {:?}", expected, actual);

    if is_map_field(message_descriptor, field_descriptor) {
      trace!(%field_name, field_no, "field is a map field");
      let map_comparison = compare_map_field(&field_path, field_descriptor, expected, actual, matching_context, descriptors);
      if !map_comparison.is_empty() {
        results.insert(field_path.to_string(), map_comparison);
      }
    } else if is_repeated_field(field_descriptor) {
      trace!(%field_name, field_no, "field is a repeated field");
      let e = expected.iter().map(|f| (*f).clone()).collect_vec();
      let a = actual.iter().map(|f| (*f).clone()).collect_vec();
      let repeated_comparison = compare_repeated_field(&field_path, field_descriptor, &e, &a, matching_context, descriptors);
      if !repeated_comparison.is_empty() {
        results.insert(field_path.to_string(), repeated_comparison);
      }
    } else if let Some(expected_value) = expected.first() {
      let actual_value = actual.first().map(|v| (*v).clone()).unwrap_or_else(|| {
        // Need to compare against the default values, as gRPC lib may have skipped sending the field if it was a default
        expected_value.default_field_value(field_descriptor)
      });

      let comparison = compare_field(&field_path, expected_value, field_descriptor, &actual_value, matching_context, descriptors);
      if !comparison.is_empty() {
        results.insert(field_path.to_string(), comparison);
      }
    } else if !actual.is_empty() && matching_context.config() == DiffConfig::NoUnexpectedKeys {
      trace!(field_name = field_name.as_str(), field_no, "actual field list is not empty");
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
#[tracing::instrument(ret,
  skip_all,
  fields(%path, %field, %actual)
)]
fn compare_field(
  path: &DocPath,
  field: &ProtobufField,
  descriptor: &FieldDescriptorProto,
  actual: &ProtobufField,
  matching_context: &(dyn MatchingContext + Send + Sync),
  descriptors: &FileDescriptorSet
) -> Vec<Mismatch> {
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
                BodyMatchResult::BodyMismatches(mismatches) => mismatches.values().flatten().cloned().collect()
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
#[tracing::instrument(ret, skip_all, fields(%path))]
fn compare_repeated_field(
  path: &DocPath,
  descriptor: &FieldDescriptorProto,
  expected_fields: &[ProtobufField],
  actual_fields: &[ProtobufField],
  matching_context: &(dyn MatchingContext + Send + Sync),
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
    debug!("Expected an empty list, but actual has {} fields", actual_fields.len());
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
    trace!("Comparing repeated fields as a list");
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
  matching_context: &(dyn MatchingContext + Send + Sync),
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
          &expected_map, &actual_map, matching_context, &mut |field_path, expected, actual, context| {
            let comparison = compare_field(field_path, &expected.value, &expected.field_descriptor, &actual.value, context, descriptors);
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
#[tracing::instrument(ret, skip_all, fields(%path))]
fn compare_list_content(
  path: &DocPath,
  descriptor: &FieldDescriptorProto,
  expected: &[ProtobufField],
  actual: &[ProtobufField],
  matching_context: &(dyn MatchingContext + Send + Sync),
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

#[cfg(test)]
mod tests {
  use base64::Engine;
  use base64::engine::general_purpose::STANDARD as BASE64;
  use expectest::prelude::*;
  use pact_models::matchingrules::expressions::{MatchingRuleDefinition, ValueType};
  use pact_models::{matchingrules, matchingrules_list};
  use prost::encoding::WireType;
  use prost::Message;
  use prost_types::{DescriptorProto, EnumDescriptorProto, EnumValueDescriptorProto, FieldDescriptorProto, FileDescriptorSet, MessageOptions};
  use prost_types::field_descriptor_proto::Label::{Optional, Repeated};
  use prost_types::field_descriptor_proto::Type::{Enum, String};

  use crate::message_decoder::ProtobufField;

  use super::*;

  const DESCRIPTORS: &'static str = "CuIFChxnb29nbGUvcHJvdG9idWYvc3RydWN0LnByb3RvEg9nb29\
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

  #[test_log::test]
  fn compare_message_where_the_actual_field_is_missing_due_it_being_the_default_enum_value() {
    let bytes = BASE64.decode(DESCRIPTORS).unwrap();
    let bytes1 = Bytes::copy_from_slice(bytes.as_slice());
    let fds = FileDescriptorSet::decode(bytes1).unwrap();

    let (message_descriptor, _) = find_message_type_by_name("InitPluginResponse", &fds).unwrap();

    let path = DocPath::new("$").unwrap();
    let context = CoreMatchingContext::new(DiffConfig::AllowUnexpectedKeys, &matchingrules_list! {
      "body"; 
      "$.catalogue.type" => [ MatchingRule::Regex("CONTENT_MATCHER|CONTENT_GENERATOR".into()) ],
      "$.catalogue.key" => [ MatchingRule::NotEmpty ],
      "$.catalogue.*" => [ MatchingRule::Type ],
      "$.catalogue" => [ MatchingRule::Values ]
    }, &hashmap!{});

    let descriptor = DescriptorProto { 
      name: Some("CatalogueEntry".to_string()), 
      field: [
        FieldDescriptorProto { 
          name: Some("type".to_string()), 
          number: Some(1), 
          label: Some(Optional as i32), 
          r#type: Some(Enum as i32), 
          type_name: Some(".io.pact.plugin.CatalogueEntry.EntryType".to_string()), 
          extendee: None, 
          default_value: None, 
          oneof_index: None, 
          json_name: Some("type".to_string()), 
          options: None, 
          proto3_optional: None 
        }, 
        FieldDescriptorProto { 
          name: Some("key".to_string()), 
          number: Some(2), 
          label: Some(Optional as i32), 
          r#type: Some(String as i32), 
          type_name: None, 
          extendee: None, 
          default_value: None, 
          oneof_index: None, 
          json_name: Some("key".to_string()), 
          options: None, 
          proto3_optional: None 
        }, 
        FieldDescriptorProto { 
          name: Some("values".to_string()), 
          number: Some(3), 
          label: Some(Repeated as i32), 
          r#type: Some(prost_types::field_descriptor_proto::Type::Message as i32), 
          type_name: Some(".io.pact.plugin.CatalogueEntry.ValuesEntry".to_string()), 
          extendee: None, 
          default_value: None, 
          oneof_index: None, 
          json_name: Some("values".to_string()), 
          options: None, 
          proto3_optional: None 
        }
      ].to_vec(), 
      extension: Default::default(), 
      nested_type: [
        DescriptorProto { 
          name: Some("ValuesEntry".to_string()), 
          field: [
            FieldDescriptorProto { 
              name: Some("key".to_string()), 
              number: Some(1), 
              label: Some(Optional as i32), 
              r#type: Some(String as i32), 
              type_name: None, 
              extendee: None, 
              default_value: None, 
              oneof_index: None, 
              json_name: Some("key".to_string()), 
              options: None, 
              proto3_optional: None 
            }, 
            FieldDescriptorProto { 
              name: Some("value".to_string()), 
              number: Some(2), 
              label: Some(Optional as i32), 
              r#type: Some(String as i32), 
              type_name: None, 
              extendee: None, 
              default_value: None, 
              oneof_index: None, 
              json_name: Some("value".to_string()), 
              options: None, 
              proto3_optional: None 
            }
          ].to_vec(), 
          extension: Default::default(), 
          nested_type: Default::default(), 
          enum_type: Default::default(), 
          extension_range: Default::default(), 
          oneof_decl: Default::default(), 
          options: Some(MessageOptions { 
            message_set_wire_format: None, 
            no_standard_descriptor_accessor: None, 
            deprecated: None, 
            map_entry: Some(true), 
            uninterpreted_option: Default::default() 
          }), 
          reserved_range: Default::default(), 
          reserved_name: Default::default() 
        }
      ].to_vec(), 
      enum_type: [
        EnumDescriptorProto { 
          name: Some("EntryType".to_string()), 
          value: [
            EnumValueDescriptorProto { 
              name: Some("CONTENT_MATCHER".to_string()), 
              number: Some(0), 
              options: None 
            }, 
            EnumValueDescriptorProto { 
              name: Some("CONTENT_GENERATOR".to_string()), 
              number: Some(1), 
              options: None 
            }, 
            EnumValueDescriptorProto { 
              name: Some("TRANSPORT".to_string()), 
              number: Some(2), 
              options: None 
            }, 
            EnumValueDescriptorProto { 
              name: Some("MATCHER".to_string()), 
              number: Some(3), 
              options: None 
            }, 
            EnumValueDescriptorProto { 
              name: Some("INTERACTION".to_string()), 
              number: Some(4), 
              options: None 
            }
          ].to_vec(), 
          options: None, 
          reserved_range: Default::default(), 
          reserved_name: Default::default() 
        }
      ].to_vec(), 
      extension_range: Default::default(), 
      oneof_decl: Default::default(), 
      options: None, 
      reserved_range: Default::default(), 
      reserved_name: Default::default() 
    };
    let expected = vec![
      ProtobufField { 
        field_num: 1,
        field_name: "catalogue".to_string(),
        wire_type: WireType::LengthDelimited,
        data: ProtobufFieldData::Message(vec![8, 0, 18, 4, 116, 101, 115, 116], descriptor.clone()) 
      }
    ];
    let actual = vec![
        ProtobufField { 
          field_num: 1,
          field_name: "catalogue".to_string(),
          wire_type: WireType::LengthDelimited,
          data: ProtobufFieldData::Message(vec![18, 8, 112, 114, 111, 116, 111, 98, 117, 102, 26, 54, 10, 13, 99, 111, 110, 116, 101, 110, 116, 45, 116, 121, 112, 101, 115, 18, 37, 97, 112, 112, 108, 105, 99, 97, 116, 105, 111, 110, 47, 112, 114, 111, 116, 111, 98, 117, 102, 59, 97, 112, 112, 108, 105, 99, 97, 116, 105, 111, 110, 47, 103, 114, 112, 99], descriptor.clone())
        }
    ];

    let result = compare_message(
      path,
      &expected,
      &actual,
      &context,
      &message_descriptor,
      &fds,
    ).unwrap();

    expect!(result).to(be_equal_to(BodyMatchResult::Ok));
  }

  #[test_log::test]
  fn compare_message_with_repeated_field_and_each_value_matcher() {
    let descriptors = base64::engine::general_purpose::STANDARD.decode(
      "CogCCgxzaW1wbGUucHJvdG8iGwoJTWVzc2FnZUluEg4KAmluGAEgASgIUgJpbiIeCgpNZXNzYWdlT3V0EhAKA291\
    dBgBIAEoCFIDb3V0IicKD1ZhbHVlc01lc3NhZ2VJbhIUCgV2YWx1ZRgBIAMoCVIFdmFsdWUiKAoQVmFsdWVzTWVzc2FnZU\
    91dBIUCgV2YWx1ZRgBIAMoCVIFdmFsdWUyYAoEVGVzdBIkCgdHZXRUZXN0EgouTWVzc2FnZUluGgsuTWVzc2FnZU91dCIA\
    EjIKCUdldFZhbHVlcxIQLlZhbHVlc01lc3NhZ2VJbhoRLlZhbHVlc01lc3NhZ2VPdXQiAGIGcHJvdG8z").unwrap();
    let fds = FileDescriptorSet::decode(descriptors.as_slice()).unwrap();

    let (message_descriptor, _) = find_message_type_by_name("ValuesMessageIn", &fds).unwrap();

    let path = DocPath::new("$").unwrap();
    let context = CoreMatchingContext::new(DiffConfig::AllowUnexpectedKeys, &matchingrules_list! {
      "body";
      "$.value" => [
        MatchingRule::EachValue(MatchingRuleDefinition::new("00000000000000000000000000000000".to_string(), ValueType::Unknown, MatchingRule::Type, None))
      ]
    }, &hashmap!{});
    let expected = vec![
      ProtobufField {
        field_num: 1,
        field_name: "value".to_string(),
        wire_type: WireType::LengthDelimited,
        data: ProtobufFieldData::String("00000000000000000000000000000000".to_string())
      }
    ];
    let actual = vec![
      ProtobufField {
        field_num: 1,
        field_name: "value".to_string(),
        wire_type: WireType::LengthDelimited,
        data: ProtobufFieldData::String("value1".to_string())
      },
      ProtobufField {
        field_num: 1,
        field_name: "value".to_string(),
        wire_type: WireType::LengthDelimited,
        data: ProtobufFieldData::String("value2".to_string())
      }
    ];

    let result = compare_message(
      path,
      &expected,
      &actual,
      &context,
      &message_descriptor,
      &fds,
    ).unwrap();

    expect!(result).to(be_equal_to(BodyMatchResult::Ok));
  }

  #[test_log::test]
  fn compare_message_with_repeated_string_field_and_each_value_matcher_with_a_regex() {
    let descriptors = base64::engine::general_purpose::STANDARD.decode(
      "CusDChBlYWNoX3ZhbHVlLnByb3RvIhsKCU1lc3NhZ2VJbhIOCgJpbhgBIAEoCFICaW4iVQoKTWVzc2FnZU91d\
      BJHChRyZXNvdXJjZV9wZXJtaXNzaW9ucxgBIAMoCzIULlJlc291cmNlUGVybWlzc2lvbnNSE3Jlc291cmNlUGVybWlzc\
      2lvbnMiXQoTUmVzb3VyY2VQZXJtaXNzaW9ucxIlCghyZXNvdXJjZRgBIAEoCzIJLlJlc291cmNlUghyZXNvdXJjZRIfC\
      gZlZmZlY3QYAiABKAsyBy5FZmZlY3RSBmVmZmVjdCJ3CghSZXNvdXJjZRIxChRhcHBsaWNhdGlvbl9yZXNvdXJjZRgBI\
      AEoCVITYXBwbGljYXRpb25SZXNvdXJjZRIgCgtwZXJtaXNzaW9ucxgCIAMoCVILcGVybWlzc2lvbnMSFgoGZ3JvdXBzG\
      AMgAygJUgZncm91cHMiLQoGRWZmZWN0EiMKBnJlc3VsdBgBIAEoDjILLkVmZmVjdEVudW1SBnJlc3VsdComCgpFZmZlY\
      3RFbnVtEhgKFEVORk9SQ0VfRUZGRUNUX0FMTE9XEAAyLAoEVGVzdBIkCgdHZXRUZXN0EgouTWVzc2FnZUluGgsuTWVzc\
      2FnZU91dCIAYgZwcm90bzM=").unwrap();
    let fds = FileDescriptorSet::decode(descriptors.as_slice()).unwrap();

    let (message_descriptor, _) = find_message_type_by_name("Resource", &fds).unwrap();

    let each_value = MatchingRule::EachValue(MatchingRuleDefinition::new("foo".to_string(), ValueType::Unknown, MatchingRule::Type, None));
    let each_value_groups = MatchingRule::EachValue(MatchingRuleDefinition::new(
      "00000000000000000000000000000000".to_string(), ValueType::Unknown,
      MatchingRule::Regex(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}|\*".to_string()), None));
    let matching_rules = matchingrules! {
      "body" => {
        "$.resource_permissions.*.resource.application_resource" => [ MatchingRule::Type ],
        "$.resource_permissions" => [ MatchingRule::Values ],
        "$.resource_permissions.*" => [ MatchingRule::Type ],
        "$.resource_permissions.*.resource.permissions" => [ each_value ],
        "$.resource_permissions.*.resource.groups" => [ each_value_groups ]
      }
    };
    let context = CoreMatchingContext::new(DiffConfig::AllowUnexpectedKeys,
      &matching_rules.rules_for_category("body").unwrap(), &hashmap!{});
    let expected = vec![
      ProtobufField {
        field_num: 3,
        field_name: "groups".to_string(),
        wire_type: WireType::LengthDelimited,
        data: ProtobufFieldData::String("*".to_string())
      }
    ];
    let actual = vec![
      ProtobufField {
        field_num: 3,
        field_name: "groups".to_string(),
        wire_type: WireType::LengthDelimited,
        data: ProtobufFieldData::String("*".to_string())
      },
      ProtobufField {
        field_num: 3,
        field_name: "groups".to_string(),
        wire_type: WireType::LengthDelimited,
        data: ProtobufFieldData::String("*".to_string())
      }
    ];

    let path = DocPath::new("$.resource_permissions.*.resource").unwrap();
    let result = compare_message(
      path,
      &expected,
      &actual,
      &context,
      &message_descriptor,
      &fds,
    ).unwrap();

    expect!(result).to(be_equal_to(BodyMatchResult::Ok));
  }
}
