//! Module for dealing with gRPC metadata (as per https://grpc.io/docs/what-is-grpc/core-concepts/#metadata).

use std::collections::HashMap;

use ansi_term::Colour::{Green, Red};
use ansi_term::Style;
use anyhow::anyhow;
use itertools::{Either, Itertools};
use maplit::hashmap;
use pact_matching::{CoreMatchingContext, matchers, MatchingContext, Mismatch};
use pact_matching::matchers::Matches;
use pact_models::generators::Generator;
use pact_models::json_utils::json_to_string;
use pact_models::matchingrules::{MatchingRule, MatchingRuleCategory, RuleLogic};
use pact_models::matchingrules::expressions::{is_matcher_def, MatchingRuleDefinition, parse_matcher_def};
use pact_models::path_exp::DocPath;
use pact_models::v4::message_parts::MessageContents;
use pact_plugin_driver::utils::proto_value_to_string;
use prost_types::Value;
use tonic::{Code, Status};
use tonic::metadata::{Ascii, MetadataMap, MetadataValue};
use tracing::instrument;
use tracing::log::trace;

use crate::utils::proto_value_to_map;

#[derive(Clone, Debug)]
pub struct MessageMetadata {
  pub matching_rules: MatchingRuleCategory,
  pub generators: HashMap<String, Generator>,
  pub values: HashMap<String, String>,
}

#[instrument(ret, level = "trace")]
pub fn process_metadata(metadata_config: Option<&Value>) -> anyhow::Result<Option<MessageMetadata>> {
  trace!("processing metadata configuration");
  match metadata_config {
    None => Ok(None),
    Some(config) => {
      let metadata_map = proto_value_to_map(config)
        .map_err(|err| anyhow!("gRPC metadata must be configured with a Map - {}", err))?;
      let mut matching_rules = MatchingRuleCategory::empty("metadata");
      let mut generators = hashmap!{};
      let mut values = hashmap!{};

      for (key, value) in &metadata_map {
        let str_value = proto_value_to_string(value).unwrap_or_default();
        if is_matcher_def(str_value.as_str()) {
          let mrd = parse_matcher_def(str_value.as_str())?;
          if !mrd.rules.is_empty() {
            for rule in &mrd.rules {
              match rule {
                Either::Left(rule) => matching_rules.add_rule(DocPath::new(key)?, rule.clone(), RuleLogic::And),
                Either::Right(mr) => return Err(anyhow!("Was expecting a value for metadata key '{}', but got a matching reference {:?}", key, mr))
              }
            }
          }
          if let Some(generator) = mrd.generator {
            generators.insert(key.clone(), generator);
          }
          values.insert(key.clone(), mrd.value);
        } else {
          values.insert(key.clone(), str_value);
        }
      }

      Ok(Some(MessageMetadata {
        matching_rules,
        generators,
        values
      }))
    }
  }
}

#[derive(Clone, Debug)]
pub struct MetadataMatchResult {
  /// Overall result of the comparison
  pub result: bool,
  /// List of mismatches that occurred
  pub mismatches: Vec<Mismatch>
}

impl MetadataMatchResult {
  /// If all the metadata values were matched ok
  pub(crate) fn all_matched(&self) -> bool {
    self.result
  }

  /// Create a result with mismatches
  pub(crate) fn mismatches(mismatches: Vec<Mismatch>) -> MetadataMatchResult {
    MetadataMatchResult {
      result: false,
      mismatches,
    }
  }

  /// Create an OK result (no mismatches)
  pub(crate) fn ok() -> MetadataMatchResult {
    MetadataMatchResult {
      result: true,
      mismatches: vec![]
    }
  }
}

#[instrument(ret)]
pub fn compare_metadata(
  expected_metadata: &HashMap<String, serde_json::Value>,
  actual_metadata: &MetadataMap,
  context: &CoreMatchingContext
) -> anyhow::Result<(MetadataMatchResult, Vec<String>)> {
  if expected_metadata.is_empty() {
    Ok((MetadataMatchResult::ok(), vec![]))
  } else if actual_metadata.is_empty() {
    let mut output = vec![];
    let bold = Style::new().bold();
    let mismatches = expected_metadata.iter()
      .filter(|(k, _)| !is_special_metadata_key(k))
      .map(|(k, v)| {
        output.push(format!("          key '{}' ({})", bold.paint(k), Red.paint("FAILED")));
        Mismatch::MetadataMismatch {
          key: k.to_string(),
          expected: v.to_string(),
          actual: "".to_string(),
          mismatch: format!("Expected metadata with key '{}' but was missing", k)
        }
      })
      .collect();
    Ok((MetadataMatchResult::mismatches(mismatches), output))
  } else {
    let mut mismatches = vec![];
    let mut output = vec![];
    let bold = Style::new().bold();

    for (key, expected_value) in expected_metadata {
      if let Some(actual_value) = actual_metadata.get(key) {
        let out = match_metadata_value(&mut mismatches, key, expected_value, actual_value, context);
        output.push(out);
      } else if !is_special_metadata_key(key) {
        output.push(format!("          key '{}' ({})", bold.paint(key), Red.paint("FAILED")));
        mismatches.push(Mismatch::MetadataMismatch { key: key.clone(),
          expected: expected_value.to_string(),
          actual: "".to_string(),
          mismatch: format!("Expected metadata value with key '{}' but was missing", key) }
        );
      }
    }

    if mismatches.is_empty() {
      Ok((MetadataMatchResult::ok(), output))
    } else {
      Ok((MetadataMatchResult::mismatches(mismatches), output))
    }
  }
}

fn is_special_metadata_key(key: &String) -> bool {
  let key = key.to_lowercase();
  key == "content-type" || key == "contenttype"
}

fn match_metadata_value(
  mismatches: &mut Vec<Mismatch>,
  key: &String,
  expected: &serde_json::Value,
  actual: &MetadataValue<Ascii>,
  context: &CoreMatchingContext
) -> String {
  let path = DocPath::root().join(key);
  let expected = json_to_string(expected);
  let bold = Style::new().bold();
  match actual.to_str() {
    Ok(actual) => {
      if context.matcher_is_defined(&path) {
        let matchers = context.select_best_matcher(&path);
        let result = if let Err(errors) = matchers::match_values(&path, &matchers, &expected, &actual.to_string()) {
          for mismatch in errors {
            mismatches.push(Mismatch::MetadataMismatch {
              key: key.clone(),
              expected: expected.clone(),
              actual: actual.to_string(),
              mismatch: format!("Comparison of metadata key '{}' failed: {}", key, mismatch)
            });
          }
          Red.paint("FAILED")
        } else {
          Green.paint("OK")
        };
        format!("        key '{}' matching with {} [{}]", bold.paint(key),
          bold.paint(matchers.rules.iter()
            .map(|r| matching_rule_description(r))
            .join(", ")
          ), result)
      } else if key == "grpc-status" {
        let actual_status = string_to_code(actual, "").unwrap_or(Status::unknown(""));
        if let Some(expected_status) = string_to_code(expected.as_str(), "") {
          let result = if expected_status.code() != actual_status.code() {
            mismatches.push(Mismatch::MetadataMismatch {
              key: key.clone(),
              expected,
              actual: actual.to_string(),
              mismatch: format!("Comparison of metadata key '{}' failed: expected {} but received {}", key,
                                code_desc(&expected_status.code()), code_desc(&actual_status.code()))
            });
            Red.paint("FAILED")
          } else {
            Green.paint("OK")
          };
          format!("        key '{}' with value '{}' [{}]", bold.paint(key), bold.paint(code_desc(&expected_status.code())), result)
        } else {
          format!("        key '{}' with value '{}' [{}]", bold.paint(key), bold.paint("OK"), Green.paint("OK"))
        }
      } else {
        if let Err(err) = Matches::matches_with(&expected, actual, &MatchingRule::Equality, false) {
          mismatches.push(Mismatch::MetadataMismatch {
            key: key.clone(),
            expected,
            actual: actual.to_string(),
            mismatch: format!("Comparison of metadata key '{}' failed: {}", key, err)
          });
        }
        format!("        key '{}' with value '{}' [{}]", bold.paint(key), bold.paint(actual), Red.paint("FAILED"))
      }
    }
    Err(err) => {
      mismatches.push(Mismatch::MetadataMismatch {
        key: key.clone(),
        expected,
        actual: "".to_string(),
        mismatch: format!("Could not convert actual value with key '{}' to a string - {}", key, err)
      });
      format!("      key '{}' [{}]", bold.paint(key), Red.paint("FAILED"))
    }
  }
}

// TODO: This should move into the Pact-Rust repo
fn matching_rule_description(rule: &MatchingRule) -> String {
  match rule {
    MatchingRule::Regex(r) => format!("regex '{}'", r),
    MatchingRule::MinType(min) => format!("type with min length {}", min),
    MatchingRule::MaxType(max) => format!("type with max length {}", max),
    MatchingRule::MinMaxType(min, max) => format!("type with length between {} and {}", min, max),
    MatchingRule::Timestamp(f) => format!("date-time with format '{}'", f),
    MatchingRule::Time(f) => format!("time with format '{}'", f),
    MatchingRule::Date(f) => format!("date with format '{}'", f),
    MatchingRule::Include(s) => format!("string that includes '{}'", s),
    MatchingRule::ContentType(ct) => format!("data with content type '{}'", ct),
    MatchingRule::StatusCode(sc) => format!("HTTP status {}", sc),
    MatchingRule::EachKey(m) => format!("each key matching {}", matching_def_description(m)),
    MatchingRule::EachValue(m) => format!("each key matching {}", matching_def_description(m)),
    _ => rule.name()
  }
}

// TODO: This should move into the Pact-Rust repo
fn matching_def_description(md: &MatchingRuleDefinition) -> String {
  md.rules.iter()
    .map(|def| {
      match def {
        Either::Left(m) => matching_rule_description(m),
        Either::Right(def) => format!("an message like '{}'", def.name)
      }
    })
    .join(", ")
}

pub fn grpc_status(response_contents: &MessageContents) -> Option<Status> {
  if let Some(value) = response_contents.metadata.get("grpc-status") {
    let status = json_to_string(value);
    let message = response_contents.metadata.get("grpc-message")
      .map(json_to_string)
      .unwrap_or("No message set".to_string());
    string_to_code(status.as_str(), message.as_str())
  } else {
    None
  }
}

pub fn string_to_code(status: &str, message: &str) -> Option<Status> {
  match status {
    // Taken from https://grpc.github.io/grpc/core/md_doc_statuscodes.html
    "OK" => None,
    "CANCELLED" => Some(Status::cancelled(message)),
    "UNKNOWN" => Some(Status::unknown(message)),
    "INVALID_ARGUMENT" => Some(Status::invalid_argument(message)),
    "DEADLINE_EXCEEDED" => Some(Status::deadline_exceeded(message)),
    "NOT_FOUND" => Some(Status::not_found(message)),
    "ALREADY_EXISTS" => Some(Status::already_exists(message)),
    "PERMISSION_DENIED" => Some(Status::permission_denied(message)),
    "RESOURCE_EXHAUSTED" => Some(Status::resource_exhausted(message)),
    "FAILED_PRECONDITION" => Some(Status::failed_precondition(message)),
    "ABORTED" => Some(Status::aborted(message)),
    "OUT_OF_RANGE" => Some(Status::out_of_range(message)),
    "UNIMPLEMENTED" => Some(Status::unimplemented(message)),
    "INTERNAL" => Some(Status::internal(message)),
    "UNAVAILABLE" => Some(Status::unavailable(message)),
    "DATA_LOSS" => Some(Status::data_loss(message)),
    "UNAUTHENTICATED" => Some(Status::unauthenticated(message)),
    _ => {
      let code = Code::from_bytes(status.as_bytes());
      if code == Code::Ok {
        None
      } else {
        Some(Status::new(code, message))
      }
    }
  }
}

fn code_desc(code: &Code) -> String {
  match code {
    Code::Ok => "OK",
    Code::Cancelled => "CANCELLED",
    Code::Unknown => "UNKNOWN",
    Code::InvalidArgument => "INVALID_ARGUMENT",
    Code::DeadlineExceeded => "DEADLINE_EXCEEDED",
    Code::NotFound => "NOT_FOUND",
    Code::AlreadyExists => "ALREADY_EXISTS",
    Code::PermissionDenied => "PERMISSION_DENIED",
    Code::ResourceExhausted => "RESOURCE_EXHAUSTED",
    Code::FailedPrecondition => "FAILED_PRECONDITION",
    Code::Aborted => "ABORTED",
    Code::OutOfRange => "OUT_OF_RANGE",
    Code::Unimplemented => "UNIMPLEMENTED",
    Code::Internal => "INTERNAL",
    Code::Unavailable => "UNAVAILABLE",
    Code::DataLoss => "DATA_LOSS",
    Code::Unauthenticated => "UNAUTHENTICATED"
  }.to_string()
}

#[cfg(test)]
mod tests {
  use expectest::prelude::*;
  use maplit::{btreemap, hashmap};
  use pact_matching::{CoreMatchingContext, DiffConfig, Mismatch};
  use pact_models::matchingrules;
  use pact_models::matchingrules::MatchingRule;
  use pact_models::path_exp::DocPath;
  use pact_models::v4::message_parts::MessageContents;
  use prost_types::{Struct, Value, value};
  use serde_json::json;
  use tonic::Code;
  use tonic::metadata::MetadataMap;

  use crate::metadata::{compare_metadata, grpc_status, process_metadata};
  use crate::utils::prost_string;

  #[test]
  fn process_metadata_returns_none_if_there_is_no_metadata() {
    let result = process_metadata(None).unwrap();
    expect!(result).to(be_none());
  }

  #[test]
  fn process_metadata_returns_an_error_if_configuration_provided_is_not_a_map_form() {
    let config = Value {
      kind: Some(value::Kind::BoolValue(true))
    };
    let result = process_metadata(Some(&config));
    expect!(result).to(be_err());
  }

  #[test]
  fn process_metadata_builds_a_map_of_metadata_from_the_provided_config() {
    let config = Value {
      kind: Some(value::Kind::StructValue(Struct {
        fields: btreemap!{
          "A".to_string() => prost_string("a"),
          "B".to_string() => prost_string("b"),
          "C".to_string() => prost_string("c")
        }
      }))
    };
    let result = process_metadata(Some(&config)).unwrap().unwrap();
    expect!(result.values).to(be_equal_to(hashmap!{
      "A".to_string() => "a".to_string(),
      "B".to_string() => "b".to_string(),
      "C".to_string() => "c".to_string()
    }));
  }

  #[test]
  fn process_metadata_handles_any_matching_rules_in_the_values() {
    let config = Value {
      kind: Some(value::Kind::StructValue(Struct {
        fields: btreemap!{
          "A".to_string() => prost_string("a"),
          "B".to_string() => prost_string("matching(boolean, true)"),
          "C".to_string() => prost_string("c")
        }
      }))
    };
    let result = process_metadata(Some(&config)).unwrap().unwrap();
    expect!(result.values).to(be_equal_to(hashmap!{
      "A".to_string() => "a".to_string(),
      "B".to_string() => "true".to_string(),
      "C".to_string() => "c".to_string()
    }));
    let rules = result.matching_rules.rules.get(&DocPath::new("B").unwrap());
    let rules_list = rules.unwrap();
    expect!(&rules_list.rules).to(be_equal_to(&vec![
      MatchingRule::Boolean
    ]));
  }

  #[test]
  fn compare_metadata_returns_ok_if_there_is_no_expected_metadata() {
    let expected = hashmap!{};
    let mut actual = MetadataMap::new();
    actual.insert("x-test", "test".parse().expect("Expected a value"));
    let context = CoreMatchingContext::default();

    let (result, _) = compare_metadata(&expected, &actual, &context).unwrap();
    expect!(result.result).to(be_true());
    expect!(result.mismatches.is_empty()).to(be_true());
  }

  #[test]
  fn compare_metadata_returns_a_mismatch_for_each_expected_key_if_there_is_no_actual_metadata_values() {
    let expected = hashmap!{
      "x-a".to_string() => serde_json::Value::String("A".to_string()),
      "x-b".to_string() => serde_json::Value::String("B".to_string())
    };
    let actual = MetadataMap::new();
    let context = CoreMatchingContext::default();

    let (result, _) = compare_metadata(&expected, &actual, &context).unwrap();
    expect!(result.result).to(be_false());
    expect!(result.mismatches.len()).to(be_equal_to(2));
  }

  #[test]
  fn compare_metadata_returns_a_mismatch_for_each_missing_key() {
    let expected = hashmap!{
      "x-a".to_string() => serde_json::Value::String("A".to_string()),
      "x-b".to_string() => serde_json::Value::String("B".to_string())
    };
    let mut actual = MetadataMap::new();
    actual.insert("x-a", "A".parse().expect("Expected a value"));
    let context = CoreMatchingContext::default();

    let (result, _) = compare_metadata(&expected, &actual, &context).unwrap();
    expect!(result.result).to(be_false());
    expect!(result.mismatches.len()).to(be_equal_to(1));
  }

  #[test]
  fn compare_metadata_returns_a_mismatch_for_each_unequal_value() {
    let expected = hashmap!{
      "x-a".to_string() => serde_json::Value::String("A".to_string()),
      "x-b".to_string() => serde_json::Value::String("B".to_string())
    };
    let mut actual = MetadataMap::new();
    actual.insert("x-a", "A".parse().expect("Expected a value"));
    actual.insert("x-b", "A".parse().expect("Expected a value"));
    let context = CoreMatchingContext::default();

    let (result, _) = compare_metadata(&expected, &actual, &context).unwrap();
    expect!(result.result).to(be_false());
    expect!(result.mismatches.len()).to(be_equal_to(1));
  }

  #[test]
  fn compare_metadata_using_matchers() {
    let expected = hashmap!{
      "x-a".to_string() => serde_json::Value::String("200".to_string()),
      "x-b".to_string() => serde_json::Value::String("100".to_string())
    };
    let mut actual = MetadataMap::new();
    actual.insert("x-a", "100".parse().expect("Expected a value"));
    actual.insert("x-b", "A".parse().expect("Expected a value"));
    let context = CoreMatchingContext::new(
      DiffConfig::NoUnexpectedKeys,
      &matchingrules! {
        "metadata" => {
          "x-a" => [ MatchingRule::Regex("^[0-9]+$".to_string()) ],
          "x-b" => [ MatchingRule::Regex("^[0-9]+$".to_string()) ]
        }
      }.rules_for_category("metadata").unwrap(),
      &hashmap!{}
    );

    let (result, _) = compare_metadata(&expected, &actual, &context).unwrap();
    expect!(result.result).to(be_false());
    expect!(result.mismatches.len()).to(be_equal_to(1));
    expect!(result.mismatches.iter().map(|m| {
      match m {
        Mismatch::MetadataMismatch { key, .. } => key.clone(),
        _ => m.description()
      }
    }).collect::<Vec<String>>()).to(be_equal_to(vec!["x-b".to_string()]));
  }

  #[test]
  fn compare_metadata_when_checking_missing_keys_ignores_pact_special_values() {
    let expected = hashmap!{
      "content-type".to_string() => serde_json::Value::String("A".to_string()),
      "contentType".to_string() => serde_json::Value::String("B".to_string())
    };
    let mut actual = MetadataMap::new();
    actual.insert("x-a", "A".parse().expect("Expected a value"));
    let context = CoreMatchingContext::default();

    let (result, _) = compare_metadata(&expected, &actual, &context).unwrap();
    expect!(result.result).to(be_true());
    expect!(result.mismatches.len()).to(be_equal_to(0));
  }

  #[test]
  fn grpc_status_test_no_status_set() {
    let message = MessageContents {
      contents: Default::default(),
      metadata: hashmap!{},
      matching_rules: Default::default(),
      generators: Default::default(),
    };
    expect!(grpc_status(&message)).to(be_none());
  }

  fn setup_message(status: &str, message: Option<&str>) -> MessageContents {
    if let Some(message) = message {
      MessageContents {
        metadata: hashmap!{
          "grpc-status".to_string() => json!(status),
          "grpc-message".to_string() => json!(message)
        },
        .. MessageContents::default()
      }
    } else {
      MessageContents {
        metadata: hashmap!{ "grpc-status".to_string() => json!(status) },
        .. MessageContents::default()
      }
    }
  }

  #[test]
  fn grpc_status_test_status_set_by_value() {
    let message = setup_message("OK", None);
    expect!(grpc_status(&message)).to(be_none());

    let message = setup_message("CANCELLED", None);
    expect!(grpc_status(&message).unwrap().code()).to(be_equal_to(Code::Cancelled));
    let message = setup_message("UNKNOWN", Some("it went bang, Mate!"));
    let status = grpc_status(&message).unwrap();
    expect!(status.code()).to(be_equal_to(Code::Unknown));
    expect!(status.message()).to(be_equal_to("it went bang, Mate!"));

    let message = setup_message("10", None);
    expect!(grpc_status(&message).unwrap().code()).to(be_equal_to(Code::Aborted));
  }

  #[test]
  fn grpc_status_test_invalid_status() {
    let message = setup_message("GGGH", None);
    expect!(grpc_status(&message).unwrap().code()).to(be_equal_to(Code::Unknown));

    let message = setup_message("33", None);
    expect!(grpc_status(&message).unwrap().code()).to(be_equal_to(Code::Unknown));
  }
}
