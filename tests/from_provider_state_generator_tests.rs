use std::path::Path;

use pretty_assertions::assert_eq;
use pact_consumer::builders::PactBuilderAsync;
use pact_models::{generators, matchingrules};
use pact_models::generators::Generator;
use pact_models::matchingrules::MatchingRule;
use serde_json::json;

#[test_log::test(tokio::test(flavor = "multi_thread"))]
async fn from_provider_state_generator_test() {
  let mut pact_builder = PactBuilderAsync::new_v4("from_provider_state_generator", "protobuf-plugin");
  pact_builder
    .using_plugin("protobuf", None).await
    .synchronous_message_interaction("each value with reference", |mut i| async move {
      let proto_file = Path::new("tests/from_provider_state_generator.proto")
        .canonicalize().unwrap().to_string_lossy().to_string();
      i.contents_from(json!({
        "pact:proto": proto_file,
        "pact:content-type": "application/protobuf",
        "pact:proto-service": "Test/GetTest",

        "request": {
          "id": "matching(regex, '^[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}$', fromProviderState('${id}', 'edda9cf4-851c-4b5f-9998-6363c136a3ba'))"
        },

        "response": {
          "name": "hello world"
        }
      })).await;
      i
    })
    .await;

  let pact = pact_builder.build().as_v4_pact().unwrap();
  let interaction = pact.interactions.first().unwrap().as_v4_sync_message().unwrap();
  let request = interaction.request;

  let matching_rules = matchingrules! {
    "body" => {
      "$.id" => [ MatchingRule::Regex("^[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}$".to_string()) ]
    }
  };
  assert_eq!(&matching_rules, &request.matching_rules);
  let generators = generators! {"BODY" => {"$.id" => Generator::ProviderStateGenerator("${id}".to_string(), None)}};
  assert_eq!(&generators, &request.generators);
}
