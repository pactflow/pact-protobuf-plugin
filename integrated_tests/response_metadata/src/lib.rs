tonic::include_proto!("pactissue");

#[cfg(test)]
mod tests {
  use std::path::Path;

  use expectest::prelude::*;
  use pact_consumer::prelude::*;
  use pact_consumer::mock_server::StartMockServerAsync;

  use pact_models::{matchingrules::{Category, MatchingRule, MatchingRules, RuleLogic}, path_exp::DocPath};
  use serde_json::json;
  use super::*;

  #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
  async fn test_proto_client() {
    let _ = env_logger::builder().is_test(true).try_init();
    let proto_file = Path::new("response_metadata.proto")
      .canonicalize().unwrap().to_string_lossy().to_string();
    let request_json = json!({
      "pact:proto": proto_file,
      "pact:content-type": "application/protobuf",
      "pact:proto-service": "Test/GetTest",

      "request": {
        "s": "matching(type, '')"
      },
      "requestMetadata": {
        "key": "matching(type, 'value')"
      },
      "response": {
        "b": "matching(boolean, true)"
      },
      "responseMetadata": {
        "grpc-status": "matching(equalTo, 'NOT_FOUND')",
        "grpc-message": "matching(type, 'not found')"
      },
    });

    let mut expected_request_rules = MatchingRules::default();
    let body_cat = expected_request_rules.add_category(Category::BODY);
    body_cat.add_rule(DocPath::new_unwrap("$.s"), MatchingRule::Type, RuleLogic::And);
    let meta_cat = expected_request_rules.add_category(Category::METADATA);
    meta_cat.add_rule(DocPath::new_unwrap("key"), MatchingRule::Type, RuleLogic::And);

    let mut expected_response_rules = MatchingRules::default();
    let body_cat = expected_response_rules.add_category(Category::BODY);
    body_cat.add_rule(DocPath::new_unwrap("$.b"), MatchingRule::Boolean, RuleLogic::And);
    let meta_cat = expected_response_rules.add_category(Category::METADATA);
    meta_cat.add_rule(DocPath::new_unwrap("grpc-status"), MatchingRule::Equality, RuleLogic::And);
    meta_cat.add_rule(DocPath::new_unwrap("grpc-message"), MatchingRule::Type, RuleLogic::And);

    let mut pact_builder = PactBuilderAsync::new_v4("grpc-consumer-rust", "response_metadata");
    let builder_async = pact_builder
      .using_plugin("protobuf", None).await
      .output_dir("pacts")
      .synchronous_message_interaction("response metadata request", |mut i| async move {
        i.contents_from(request_json).await;
        i
      }).await;

    let pact = builder_async.build();
    let interactions = pact.interactions();

    expect!(interactions.len()).to(be_equal_to(1));
    let interaction = interactions[0].as_v4_sync_message().unwrap();
    expect!(interaction.request.matching_rules.clone()).to(be_equal_to(expected_request_rules));
    expect!(interaction.response.len()).to(be_equal_to(1));
    expect!(interaction.response[0].matching_rules.clone()).to(be_equal_to(expected_response_rules));

    let mock_server = builder_async
      .start_mock_server_async(Some("protobuf/transport/grpc"), None).await;
    let url = mock_server.url();
    let mut client = test_client::TestClient::connect(url.to_string()).await.unwrap();

    let request_message = MessageIn {
      s: String::default()
    };
    let mut request = tonic::Request::new(request_message);
    request.metadata_mut().insert("key", tonic::metadata::MetadataValue::from_static("value"));

    let response = client.get_test(request).await;
    expect!(response.is_err()).to(be_true());
    expect!(response.unwrap_err().message()).to(be_equal_to("not found"));
  }
}
