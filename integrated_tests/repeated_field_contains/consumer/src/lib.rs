tonic::include_proto!("envmetadata");

#[cfg(test)]
mod tests {
  use std::path::Path;

  use expectest::prelude::*;
  use pact_consumer::prelude::*;
  use pact_consumer::mock_server::StartMockServerAsync;
  use serde_json::json;

  use super::*;

  // ==========================================================================
  // Test 1: BASELINE — No matchers on repeated field.
  //
  // Consumer expects networking = ["PUBLIC"].
  // Mock server returns exactly that.
  // Consumer test passes (trivially).
  //
  // But when the PROVIDER returns ["PUBLIC", "PRIVATE_LINK"], verification
  // would fail because of exact array matching. We can't test provider
  // verification from the consumer side, but we can generate the pact and
  // inspect it.
  // ==========================================================================
  #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
  async fn test_exact_match_baseline() {
    let _ = env_logger::builder().is_test(true).try_init();

    let mut pact_builder = PactBuilderAsync::new_v4(
      "repeated-field-consumer",
      "repeated-field-provider",
    );
    let mock_server = pact_builder
      .using_plugin("protobuf", None).await
      .synchronous_message_interaction("get env metadata - exact match", |mut i| async move {
        let proto_file = Path::new("../env_metadata.proto")
          .canonicalize().unwrap().to_string_lossy().to_string();
        i.contents_from(json!({
          "pact:proto": proto_file,
          "pact:content-type": "application/protobuf",
          "pact:proto-service": "EnvMetadataService/GetEnvMetadata",

          "request": {
            "cloud": "matching(type, 'aws')",
            "region": "matching(type, 'us-west-2')"
          },
          "response": {
            "type_name": "matching(type, 'DEDICATED')",
            "durability": ["LOW"],
            "networking": ["PUBLIC"]
          }
        })).await;
        i
      })
      .await
      .start_mock_server_async(Some("protobuf/transport/grpc"), None)
      .await;

    let url = mock_server.url();
    let mut client = env_metadata_service_client::EnvMetadataServiceClient::connect(
      url.to_string()
    ).await.unwrap();

    let response = client.get_env_metadata(tonic::Request::new(EnvMetadataRequest {
      cloud: "aws".to_string(),
      region: "us-west-2".to_string(),
    })).await;

    let msg = response.unwrap().into_inner();
    expect!(msg.type_name.as_str()).to(be_equal_to("DEDICATED"));
    expect!(msg.networking).to(be_equal_to(vec!["PUBLIC".to_string()]));
  }

  // ==========================================================================
  // Test 2: eachValue(matching(type, ...)) on repeated field.
  //
  // This should:
  // - Accept any number of string elements (type matching)
  // - NOT enforce exact values or count
  //
  // The Slack thread claims this doesn't work. Let's find out.
  // ==========================================================================
  #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
  async fn test_each_value_type_matcher() {
    let _ = env_logger::builder().is_test(true).try_init();

    let mut pact_builder = PactBuilderAsync::new_v4(
      "repeated-field-consumer",
      "repeated-field-provider",
    );
    let mock_server = pact_builder
      .using_plugin("protobuf", None).await
      .synchronous_message_interaction("get env metadata - eachValue type", |mut i| async move {
        let proto_file = Path::new("../env_metadata.proto")
          .canonicalize().unwrap().to_string_lossy().to_string();
        i.contents_from(json!({
          "pact:proto": proto_file,
          "pact:content-type": "application/protobuf",
          "pact:proto-service": "EnvMetadataService/GetEnvMetadata",

          "request": {
            "cloud": "matching(type, 'aws')",
            "region": "matching(type, 'us-west-2')"
          },
          "response": {
            "type_name": "matching(type, 'DEDICATED')",
            "networking": "eachValue(matching(type, 'PUBLIC'))"
          }
        })).await;
        i
      })
      .await
      .start_mock_server_async(Some("protobuf/transport/grpc"), None)
      .await;

    let url = mock_server.url();
    let mut client = env_metadata_service_client::EnvMetadataServiceClient::connect(
      url.to_string()
    ).await.unwrap();

    let response = client.get_env_metadata(tonic::Request::new(EnvMetadataRequest {
      cloud: "aws".to_string(),
      region: "us-west-2".to_string(),
    })).await;

    let msg = response.unwrap().into_inner();
    expect!(msg.type_name.as_str()).to(be_equal_to("DEDICATED"));
    // Mock should return the example value from the matcher
    expect!(msg.networking).to(be_equal_to(vec!["PUBLIC".to_string()]));
  }

  // ==========================================================================
  // Test 3: atLeast + atMost + eachValue on repeated field.
  //
  // This is what the CPK team tried. Their TODO says:
  // "We also couldn't get the atLeast / atMost assertions to work correctly"
  //
  // Let's see what happens.
  // ==========================================================================
  #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
  async fn test_at_least_at_most_each_value() {
    let _ = env_logger::builder().is_test(true).try_init();

    let mut pact_builder = PactBuilderAsync::new_v4(
      "repeated-field-consumer",
      "repeated-field-provider",
    );
    let mock_server = pact_builder
      .using_plugin("protobuf", None).await
      .synchronous_message_interaction("get env metadata - atLeast atMost", |mut i| async move {
        let proto_file = Path::new("../env_metadata.proto")
          .canonicalize().unwrap().to_string_lossy().to_string();
        i.contents_from(json!({
          "pact:proto": proto_file,
          "pact:content-type": "application/protobuf",
          "pact:proto-service": "EnvMetadataService/GetEnvMetadata",

          "request": {
            "cloud": "matching(type, 'aws')",
            "region": "matching(type, 'us-west-2')"
          },
          "response": {
            "type_name": "matching(type, 'DEDICATED')",
            "networking": "atLeast(1), atMost(3), eachValue(matching(type, 'PUBLIC'))"
          }
        })).await;
        i
      })
      .await
      .start_mock_server_async(Some("protobuf/transport/grpc"), None)
      .await;

    let url = mock_server.url();
    let mut client = env_metadata_service_client::EnvMetadataServiceClient::connect(
      url.to_string()
    ).await.unwrap();

    let response = client.get_env_metadata(tonic::Request::new(EnvMetadataRequest {
      cloud: "aws".to_string(),
      region: "us-west-2".to_string(),
    })).await;

    let msg = response.unwrap().into_inner();
    expect!(msg.type_name.as_str()).to(be_equal_to("DEDICATED"));
  }

  // ==========================================================================
  // Test 4: atLeast only (no eachValue) on repeated field.
  //
  // Simplest min-count assertion. If this fails, the problem is with
  // how atLeast is handled for repeated fields specifically.
  // ==========================================================================
  #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
  async fn test_at_least_only() {
    let _ = env_logger::builder().is_test(true).try_init();

    let mut pact_builder = PactBuilderAsync::new_v4(
      "repeated-field-consumer",
      "repeated-field-provider",
    );
    let mock_server = pact_builder
      .using_plugin("protobuf", None).await
      .synchronous_message_interaction("get env metadata - atLeast only", |mut i| async move {
        let proto_file = Path::new("../env_metadata.proto")
          .canonicalize().unwrap().to_string_lossy().to_string();
        i.contents_from(json!({
          "pact:proto": proto_file,
          "pact:content-type": "application/protobuf",
          "pact:proto-service": "EnvMetadataService/GetEnvMetadata",

          "request": {
            "cloud": "matching(type, 'aws')",
            "region": "matching(type, 'us-west-2')"
          },
          "response": {
            "type_name": "matching(type, 'DEDICATED')",
            "networking": "atLeast(1)"
          }
        })).await;
        i
      })
      .await
      .start_mock_server_async(Some("protobuf/transport/grpc"), None)
      .await;

    let url = mock_server.url();
    let mut client = env_metadata_service_client::EnvMetadataServiceClient::connect(
      url.to_string()
    ).await.unwrap();

    let response = client.get_env_metadata(tonic::Request::new(EnvMetadataRequest {
      cloud: "aws".to_string(),
      region: "us-west-2".to_string(),
    })).await;

    let msg = response.unwrap().into_inner();
    expect!(msg.type_name.as_str()).to(be_equal_to("DEDICATED"));
  }

  // ==========================================================================
  // Test 5: arrayContains(matching(equalTo, 'PUBLIC')) on repeated field.
  //
  // This is the NEW matcher from Task 5. The consumer says:
  // "I need the networking array to CONTAIN at least one element equal to PUBLIC"
  //
  // Mock server returns what the consumer specified (just ["PUBLIC"]).
  // The real test is provider verification (Test 7 below checks that).
  // ==========================================================================
  #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
  async fn test_array_contains_equal_to() {
    let _ = env_logger::builder().is_test(true).try_init();

    let mut pact_builder = PactBuilderAsync::new_v4(
      "repeated-field-consumer",
      "repeated-field-provider",
    );
    let mock_server = pact_builder
      .using_plugin("protobuf", None).await
      .synchronous_message_interaction("get env metadata - arrayContains equalTo", |mut i| async move {
        let proto_file = Path::new("../env_metadata.proto")
          .canonicalize().unwrap().to_string_lossy().to_string();
        i.contents_from(json!({
          "pact:proto": proto_file,
          "pact:content-type": "application/protobuf",
          "pact:proto-service": "EnvMetadataService/GetEnvMetadata",

          "request": {
            "cloud": "matching(type, 'aws')",
            "region": "matching(type, 'us-west-2')"
          },
          "response": {
            "type_name": "matching(type, 'DEDICATED')",
            "networking": "arrayContains(matching(equalTo, 'PUBLIC'))"
          }
        })).await;
        i
      })
      .await
      .start_mock_server_async(Some("protobuf/transport/grpc"), None)
      .await;

    let url = mock_server.url();
    let mut client = env_metadata_service_client::EnvMetadataServiceClient::connect(
      url.to_string()
    ).await.unwrap();

    let response = client.get_env_metadata(tonic::Request::new(EnvMetadataRequest {
      cloud: "aws".to_string(),
      region: "us-west-2".to_string(),
    })).await;

    let msg = response.unwrap().into_inner();
    expect!(msg.type_name.as_str()).to(be_equal_to("DEDICATED"));
    expect!(msg.networking).to(be_equal_to(vec!["PUBLIC".to_string()]));
  }

  // ==========================================================================
  // Test 6: arrayContains(matching(equalTo, 'NONEXISTENT')) on repeated field.
  //
  // Consumer expects NONEXISTENT in the array. Mock server returns it.
  // But the PROVIDER returns ["PUBLIC", "PRIVATE_LINK", "TRANSIT_GATEWAY"],
  // which does NOT contain NONEXISTENT. So provider verification should FAIL.
  // ==========================================================================
  #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
  async fn test_array_contains_missing_value() {
    let _ = env_logger::builder().is_test(true).try_init();

    let mut pact_builder = PactBuilderAsync::new_v4(
      "repeated-field-consumer",
      "repeated-field-provider",
    );
    let mock_server = pact_builder
      .using_plugin("protobuf", None).await
      .synchronous_message_interaction("get env metadata - arrayContains missing", |mut i| async move {
        let proto_file = Path::new("../env_metadata.proto")
          .canonicalize().unwrap().to_string_lossy().to_string();
        i.contents_from(json!({
          "pact:proto": proto_file,
          "pact:content-type": "application/protobuf",
          "pact:proto-service": "EnvMetadataService/GetEnvMetadata",

          "request": {
            "cloud": "matching(type, 'aws')",
            "region": "matching(type, 'us-west-2')"
          },
          "response": {
            "type_name": "matching(type, 'DEDICATED')",
            "networking": "arrayContains(matching(equalTo, 'NONEXISTENT'))"
          }
        })).await;
        i
      })
      .await
      .start_mock_server_async(Some("protobuf/transport/grpc"), None)
      .await;

    let url = mock_server.url();
    let mut client = env_metadata_service_client::EnvMetadataServiceClient::connect(
      url.to_string()
    ).await.unwrap();

    let response = client.get_env_metadata(tonic::Request::new(EnvMetadataRequest {
      cloud: "aws".to_string(),
      region: "us-west-2".to_string(),
    })).await;

    let msg = response.unwrap().into_inner();
    expect!(msg.type_name.as_str()).to(be_equal_to("DEDICATED"));
    // Mock returns NONEXISTENT as specified; the real test is provider verification
    expect!(msg.networking).to(be_equal_to(vec!["NONEXISTENT".to_string()]));
  }

  // ==========================================================================
  // Test 7: arrayContains with reference form on repeated MESSAGE field.
  //
  // This is the NEW reference form:
  //   "pact:match": "arrayContains(matching($'publicNet'))"
  // where publicNet is a sibling key defining the expected message structure.
  //
  // The consumer says: "The networking array must contain at least one
  // NetworkConfig where type='PUBLIC' and endpoint matches type 'https://...'"
  // ==========================================================================
  #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
  async fn test_array_contains_reference_form() {
    let _ = env_logger::builder().is_test(true).try_init();

    let mut pact_builder = PactBuilderAsync::new_v4(
      "repeated-field-consumer",
      "repeated-field-provider",
    );
    let mock_server = pact_builder
      .using_plugin("protobuf", None).await
      .synchronous_message_interaction("get env metadata v2 - arrayContains ref", |mut i| async move {
        let proto_file = Path::new("../env_metadata.proto")
          .canonicalize().unwrap().to_string_lossy().to_string();
        i.contents_from(json!({
          "pact:proto": proto_file,
          "pact:content-type": "application/protobuf",
          "pact:proto-service": "EnvMetadataServiceV2/GetEnvMetadataV2",
          "request": {
            "cloud": "matching(type, 'aws')",
            "region": "matching(type, 'us-west-2')"
          },
          "response": {
            "type_name": "matching(type, 'DEDICATED')",
            "networking": {
              "pact:match": "arrayContains(matching($'publicNet'))",
              "publicNet": {
                "type": "matching(equalTo, 'PUBLIC')",
                "endpoint": "matching(type, 'https://example.com')"
              }
            }
          }
        })).await;
        i
      })
      .await
      .start_mock_server_async(Some("protobuf/transport/grpc"), None)
      .await;

    let url = mock_server.url();
    let mut client = env_metadata_service_v2_client::EnvMetadataServiceV2Client::connect(
      url.to_string()
    ).await.unwrap();

    let response = client.get_env_metadata_v2(tonic::Request::new(EnvMetadataRequest {
      cloud: "aws".to_string(),
      region: "us-west-2".to_string(),
    })).await;

    let msg = response.unwrap().into_inner();
    expect!(msg.type_name.as_str()).to(be_equal_to("DEDICATED"));
    assert!(!msg.networking.is_empty());
  }

  // ==========================================================================
  // Test 8: arrayContains reference form with MULTIPLE variants on a repeated
  // message field. Previously the second variant overwrote the first because
  // build_single_embedded_field_value calls set_field_value internally.
  // ==========================================================================
  #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
  async fn test_array_contains_reference_form_multi_variant() {
    let _ = env_logger::builder().is_test(true).try_init();

    let mut pact_builder = PactBuilderAsync::new_v4(
      "repeated-field-consumer",
      "repeated-field-provider",
    );
    let mock_server = pact_builder
      .using_plugin("protobuf", None).await
      .synchronous_message_interaction("get env metadata v2 - arrayContains multi ref", |mut i| async move {
        let proto_file = Path::new("../env_metadata.proto")
          .canonicalize().unwrap().to_string_lossy().to_string();
        i.contents_from(json!({
          "pact:proto": proto_file,
          "pact:content-type": "application/protobuf",
          "pact:proto-service": "EnvMetadataServiceV2/GetEnvMetadataV2",
          "request": {
            "cloud": "matching(type, 'aws')",
            "region": "matching(type, 'us-west-2')"
          },
          "response": {
            "type_name": "matching(type, 'DEDICATED')",
            "networking": {
              "pact:match": "arrayContains(matching($'publicNet'), matching($'plNet'))",
              "publicNet": {
                "type": "matching(equalTo, 'PUBLIC')",
                "endpoint": "matching(type, 'https://example.com')"
              },
              "plNet": {
                "type": "matching(equalTo, 'PRIVATE_LINK')",
                "endpoint": "matching(type, 'vpce-abc123')"
              }
            }
          }
        })).await;
        i
      })
      .await
      .start_mock_server_async(Some("protobuf/transport/grpc"), None)
      .await;

    let url = mock_server.url();
    let mut client = env_metadata_service_v2_client::EnvMetadataServiceV2Client::connect(
      url.to_string()
    ).await.unwrap();

    let response = client.get_env_metadata_v2(tonic::Request::new(EnvMetadataRequest {
      cloud: "aws".to_string(),
      region: "us-west-2".to_string(),
    })).await;

    let msg = response.unwrap().into_inner();
    expect!(msg.type_name.as_str()).to(be_equal_to("DEDICATED"));
    // Both variants must be present in the example
    assert_eq!(msg.networking.len(), 2, "Expected 2 example elements (one per variant), got {}", msg.networking.len());
    let types: Vec<&str> = msg.networking.iter().map(|n| n.r#type.as_str()).collect();
    assert!(types.contains(&"PUBLIC"), "Expected PUBLIC variant in {:?}", types);
    assert!(types.contains(&"PRIVATE_LINK"), "Expected PRIVATE_LINK variant in {:?}", types);
  }
}
