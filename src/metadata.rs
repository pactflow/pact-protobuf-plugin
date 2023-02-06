//! Module for dealing with gRPC metadata (as per https://grpc.io/docs/what-is-grpc/core-concepts/#metadata).

use std::collections::HashMap;

use anyhow::anyhow;
use itertools::Either;
use maplit::hashmap;
use pact_models::generators::Generator;
use pact_models::json_utils::json_to_string;
use pact_models::matchingrules::{MatchingRuleCategory, RuleLogic};
use pact_models::matchingrules::expressions::{is_matcher_def, parse_matcher_def};
use pact_models::path_exp::DocPath;
use prost_types::Value;
use tracing::instrument;
use tracing::log::trace;

use crate::utils::proto_value_to_map;

#[derive(Clone, Debug)]
pub struct MessageMetadata {
  pub matching_rules: MatchingRuleCategory,
  pub generators: HashMap<String, Generator>,
  pub values: HashMap<String, String>,
}

#[instrument(ret)]
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

#[cfg(test)]
mod tests {
  use expectest::prelude::*;
  use maplit::{btreemap, hashmap};
  use pact_models::matchingrules::MatchingRule;
  use pact_models::path_exp::DocPath;
  use prost_types::{Struct, Value, value};

  use crate::metadata::process_metadata;
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
}
