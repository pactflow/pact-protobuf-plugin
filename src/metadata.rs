//! Module for dealing with gRPC metadata (as per https://grpc.io/docs/what-is-grpc/core-concepts/#metadata).

use std::collections::HashMap;

use anyhow::anyhow;
use itertools::Either;
use maplit::hashmap;
use pact_matching::{CoreMatchingContext, matchers, MatchingContext, Mismatch};
use pact_matching::matchers::Matches;
use pact_models::generators::Generator;
use pact_models::json_utils::json_to_string;
use pact_models::matchingrules::{MatchingRule, MatchingRuleCategory, RuleLogic};
use pact_models::matchingrules::expressions::{is_matcher_def, parse_matcher_def};
use pact_models::path_exp::DocPath;
use prost_types::Value;
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
        let str_value = json_to_string(value);
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
) -> anyhow::Result<MetadataMatchResult> {
  if expected_metadata.is_empty() {
    Ok(MetadataMatchResult::ok())
  } else if actual_metadata.is_empty() {
    Ok(
      MetadataMatchResult::mismatches(expected_metadata.iter()
        .filter(|(k, _)| !is_special_metadata_key(k))
        .map(|(k, v)| {
          Mismatch::MetadataMismatch {
            key: k.to_string(),
            expected: v.to_string(),
            actual: "".to_string(),
            mismatch: format!("Expected metadata with key '{}' but was missing", k)
          }
        })
        .collect()
      )
    )
  } else {
    let mut mismatches = vec![];

    for (key, expected_value) in expected_metadata {
      if let Some(actual_value) = actual_metadata.get(key) {
        match_metadata_value(&mut mismatches, key, expected_value, actual_value, context);
      } else if !is_special_metadata_key(key) {
        mismatches.push(Mismatch::MetadataMismatch { key: key.clone(),
          expected: expected_value.to_string(),
          actual: "".to_string(),
          mismatch: format!("Expected metadata value with key '{}' but was missing", key) }
        );
      }
    }

    if mismatches.is_empty() {
      Ok(MetadataMatchResult::ok())
    } else {
      Ok(MetadataMatchResult::mismatches(mismatches))
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
) {
  let path = DocPath::root().join(key);
  let expected = json_to_string(expected);
  match actual.to_str() {
    Ok(actual) => {
      if context.matcher_is_defined(&path) {
        if let Err(errors)  = matchers::match_values(&path, &context.select_best_matcher(&path), &expected, &actual.to_string()) {
          for mismatch in errors {
            mismatches.push(Mismatch::MetadataMismatch {
              key: key.clone(),
              expected: expected.clone(),
              actual: actual.to_string(),
              mismatch: format!("Comparison of metadata key '{}' failed: {}", key, mismatch)
            });
          }
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
      };
    }
    Err(err) => {
      mismatches.push(Mismatch::MetadataMismatch {
        key: key.clone(),
        expected,
        actual: "".to_string(),
        mismatch: format!("Could not convert actual value with key '{}' to a string - {}", key, err)
      });
    }
  }
}

#[cfg(test)]
mod tests {
  use expectest::prelude::*;
  use maplit::{btreemap, hashmap};
  use pact_matching::{CoreMatchingContext, DiffConfig, Mismatch};
  use pact_models::matchingrules;
  use pact_models::matchingrules::MatchingRule;
  use pact_models::path_exp::DocPath;
  use prost_types::{Struct, Value, value};
  use tonic::metadata::MetadataMap;

  use crate::metadata::{compare_metadata, process_metadata};
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

    let result = compare_metadata(&expected, &actual, &context).unwrap();
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

    let result = compare_metadata(&expected, &actual, &context).unwrap();
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

    let result = compare_metadata(&expected, &actual, &context).unwrap();
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

    let result = compare_metadata(&expected, &actual, &context).unwrap();
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

    let result = compare_metadata(&expected, &actual, &context).unwrap();
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

    let result = compare_metadata(&expected, &actual, &context).unwrap();
    expect!(result.result).to(be_true());
    expect!(result.mismatches.len()).to(be_equal_to(0));
  }
}
