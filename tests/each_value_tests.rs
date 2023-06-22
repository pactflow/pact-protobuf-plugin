use std::path::Path;

use pretty_assertions::assert_eq;
use pact_consumer::builders::PactBuilderAsync;
use pact_models::matchingrules;
use pact_models::matchingrules::MatchingRule;
use pact_models::matchingrules::expressions::{MatchingRuleDefinition, ValueType};
use serde_json::json;

#[test_log::test(tokio::test(flavor = "multi_thread"))]
async fn each_value_test() {
  let mut pact_builder = PactBuilderAsync::new_v4("each_value", "protobuf-plugin");
  pact_builder
    .using_plugin("protobuf", None).await
    .synchronous_message_interaction("each value with reference", |mut i| async move {
      let proto_file = Path::new("tests/each_value.proto")
        .canonicalize().unwrap().to_string_lossy().to_string();
      i.contents_from(json!({
        "pact:proto": proto_file,
        "pact:content-type": "application/protobuf",
        "pact:proto-service": "Test/GetTest",

        "request": {
          "in": "matching(boolean, true)"
        },

        "response": {
          "resource_permissions": {
            "pact:match": "eachValue(matching($'ResourceUserPermission'))",
            "ResourceUserPermission": {
              "resource": {
                "application_resource": "matching(type, 'foo')",
                "permissions": "eachValue(matching(type, 'foo'))",
                "groups": "eachValue(matching(regex, '[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}|\\*', '00000000000000000000000000000000'))"
              },
              "effect": {
                "result": "ENFORCE_EFFECT_ALLOW"
              }
            }
          }
        }
      })).await;
      i
    })
    .await;

  let pact = pact_builder.build().as_v4_pact().unwrap();
  let interaction = dbg!(pact).interactions.first().unwrap().as_v4_sync_message().unwrap();
  let response = interaction.response.first().unwrap();

  let each_value = MatchingRule::EachValue(MatchingRuleDefinition::new("foo".to_string(), ValueType::Unknown, MatchingRule::Type, None));
  let each_value_groups = MatchingRule::EachValue(MatchingRuleDefinition::new(
    "00000000000000000000000000000000".to_string(), ValueType::Unknown,
    MatchingRule::Regex("[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}|\\*".to_string()), None));
  let matching_rules = matchingrules! {
    "body" => {
      "$.resource_permissions.*.resource.application_resource" => [ MatchingRule::Type ],
      "$.resource_permissions" => [ MatchingRule::Values ],
      "$.resource_permissions.*" => [ MatchingRule::Type ],
      "$.resource_permissions.*.resource.permissions" => [ each_value ],
      "$.resource_permissions.*.resource.groups" => [ each_value_groups ]
    }
  };
  assert_eq!(&matching_rules, &response.matching_rules);
}
