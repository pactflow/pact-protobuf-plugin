tonic::include_proto!("catalog");

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
  // Consumer expects formats = ["HARDCOVER"].
  // Mock server returns exactly that.
  // Consumer test passes (trivially).
  //
  // But when the PROVIDER returns ["HARDCOVER", "PAPERBACK"], verification
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
      .synchronous_message_interaction("get catalog entry - exact match", |mut i| async move {
        let proto_file = Path::new("../catalog.proto")
          .canonicalize().unwrap().to_string_lossy().to_string();
        i.contents_from(json!({
          "pact:proto": proto_file,
          "pact:content-type": "application/protobuf",
          "pact:proto-service": "CatalogService/GetCatalogEntry",

          "request": {
            "shelf": "matching(type, 'main')",
            "section": "matching(type, 'fiction')"
          },
          "response": {
            "title": "matching(type, 'REFERENCE')",
            "languages": ["EN"],
            "formats": ["HARDCOVER"]
          }
        })).await;
        i
      })
      .await
      .start_mock_server_async(Some("protobuf/transport/grpc"), None)
      .await;

    let url = mock_server.url();
    let mut client = catalog_service_client::CatalogServiceClient::connect(
      url.to_string()
    ).await.unwrap();

    let response = client.get_catalog_entry(tonic::Request::new(CatalogRequest {
      shelf: "main".to_string(),
      section: "fiction".to_string(),
    })).await;

    let msg = response.unwrap().into_inner();
    expect!(msg.title.as_str()).to(be_equal_to("REFERENCE"));
    expect!(msg.formats).to(be_equal_to(vec!["HARDCOVER".to_string()]));
  }

  // ==========================================================================
  // Test 2: eachValue(matching(type, ...)) on repeated field.
  //
  // This should:
  // - Accept any number of string elements (type matching)
  // - NOT enforce exact values or count
  //
  // Reported as not working; this pins down the actual behaviour.
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
      .synchronous_message_interaction("get catalog entry - eachValue type", |mut i| async move {
        let proto_file = Path::new("../catalog.proto")
          .canonicalize().unwrap().to_string_lossy().to_string();
        i.contents_from(json!({
          "pact:proto": proto_file,
          "pact:content-type": "application/protobuf",
          "pact:proto-service": "CatalogService/GetCatalogEntry",

          "request": {
            "shelf": "matching(type, 'main')",
            "section": "matching(type, 'fiction')"
          },
          "response": {
            "title": "matching(type, 'REFERENCE')",
            "formats": "eachValue(matching(type, 'HARDCOVER'))"
          }
        })).await;
        i
      })
      .await
      .start_mock_server_async(Some("protobuf/transport/grpc"), None)
      .await;

    let url = mock_server.url();
    let mut client = catalog_service_client::CatalogServiceClient::connect(
      url.to_string()
    ).await.unwrap();

    let response = client.get_catalog_entry(tonic::Request::new(CatalogRequest {
      shelf: "main".to_string(),
      section: "fiction".to_string(),
    })).await;

    let msg = response.unwrap().into_inner();
    expect!(msg.title.as_str()).to(be_equal_to("REFERENCE"));
    // Mock should return the example value from the matcher
    expect!(msg.formats).to(be_equal_to(vec!["HARDCOVER".to_string()]));
  }

  // ==========================================================================
  // Test 3: atLeast + atMost + eachValue on repeated field.
  //
  // This is the combination users reach for first, and the reported symptom was
  // "we couldn't get the atLeast / atMost assertions to work correctly".
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
      .synchronous_message_interaction("get catalog entry - atLeast atMost", |mut i| async move {
        let proto_file = Path::new("../catalog.proto")
          .canonicalize().unwrap().to_string_lossy().to_string();
        i.contents_from(json!({
          "pact:proto": proto_file,
          "pact:content-type": "application/protobuf",
          "pact:proto-service": "CatalogService/GetCatalogEntry",

          "request": {
            "shelf": "matching(type, 'main')",
            "section": "matching(type, 'fiction')"
          },
          "response": {
            "title": "matching(type, 'REFERENCE')",
            "formats": "atLeast(1), atMost(3), eachValue(matching(type, 'HARDCOVER'))"
          }
        })).await;
        i
      })
      .await
      .start_mock_server_async(Some("protobuf/transport/grpc"), None)
      .await;

    let url = mock_server.url();
    let mut client = catalog_service_client::CatalogServiceClient::connect(
      url.to_string()
    ).await.unwrap();

    let response = client.get_catalog_entry(tonic::Request::new(CatalogRequest {
      shelf: "main".to_string(),
      section: "fiction".to_string(),
    })).await;

    let msg = response.unwrap().into_inner();
    expect!(msg.title.as_str()).to(be_equal_to("REFERENCE"));
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
      .synchronous_message_interaction("get catalog entry - atLeast only", |mut i| async move {
        let proto_file = Path::new("../catalog.proto")
          .canonicalize().unwrap().to_string_lossy().to_string();
        i.contents_from(json!({
          "pact:proto": proto_file,
          "pact:content-type": "application/protobuf",
          "pact:proto-service": "CatalogService/GetCatalogEntry",

          "request": {
            "shelf": "matching(type, 'main')",
            "section": "matching(type, 'fiction')"
          },
          "response": {
            "title": "matching(type, 'REFERENCE')",
            "formats": "atLeast(1)"
          }
        })).await;
        i
      })
      .await
      .start_mock_server_async(Some("protobuf/transport/grpc"), None)
      .await;

    let url = mock_server.url();
    let mut client = catalog_service_client::CatalogServiceClient::connect(
      url.to_string()
    ).await.unwrap();

    let response = client.get_catalog_entry(tonic::Request::new(CatalogRequest {
      shelf: "main".to_string(),
      section: "fiction".to_string(),
    })).await;

    let msg = response.unwrap().into_inner();
    expect!(msg.title.as_str()).to(be_equal_to("REFERENCE"));
  }

  // ==========================================================================
  // Test 5: arrayContains(matching(equalTo, 'HARDCOVER')) on repeated field.
  //
  // This is the NEW matcher from Task 5. The consumer says:
  // "I need the formats array to CONTAIN at least one element equal to HARDCOVER"
  //
  // Mock server returns what the consumer specified (just ["HARDCOVER"]).
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
      .synchronous_message_interaction("get catalog entry - arrayContains equalTo", |mut i| async move {
        let proto_file = Path::new("../catalog.proto")
          .canonicalize().unwrap().to_string_lossy().to_string();
        i.contents_from(json!({
          "pact:proto": proto_file,
          "pact:content-type": "application/protobuf",
          "pact:proto-service": "CatalogService/GetCatalogEntry",

          "request": {
            "shelf": "matching(type, 'main')",
            "section": "matching(type, 'fiction')"
          },
          "response": {
            "title": "matching(type, 'REFERENCE')",
            "formats": "arrayContains(matching(equalTo, 'HARDCOVER'))"
          }
        })).await;
        i
      })
      .await
      .start_mock_server_async(Some("protobuf/transport/grpc"), None)
      .await;

    let url = mock_server.url();
    let mut client = catalog_service_client::CatalogServiceClient::connect(
      url.to_string()
    ).await.unwrap();

    let response = client.get_catalog_entry(tonic::Request::new(CatalogRequest {
      shelf: "main".to_string(),
      section: "fiction".to_string(),
    })).await;

    let msg = response.unwrap().into_inner();
    expect!(msg.title.as_str()).to(be_equal_to("REFERENCE"));
    expect!(msg.formats).to(be_equal_to(vec!["HARDCOVER".to_string()]));
  }

  // ==========================================================================
  // Test 6: arrayContains(matching(equalTo, 'MISSING_FORMAT')) on repeated field.
  //
  // Consumer expects MISSING_FORMAT in the array. Mock server returns it.
  // But the PROVIDER returns ["HARDCOVER", "PAPERBACK", "AUDIOBOOK"],
  // which does NOT contain MISSING_FORMAT. So provider verification should FAIL.
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
      .synchronous_message_interaction("get catalog entry - arrayContains missing", |mut i| async move {
        let proto_file = Path::new("../catalog.proto")
          .canonicalize().unwrap().to_string_lossy().to_string();
        i.contents_from(json!({
          "pact:proto": proto_file,
          "pact:content-type": "application/protobuf",
          "pact:proto-service": "CatalogService/GetCatalogEntry",

          "request": {
            "shelf": "matching(type, 'main')",
            "section": "matching(type, 'fiction')"
          },
          "response": {
            "title": "matching(type, 'REFERENCE')",
            "formats": "arrayContains(matching(equalTo, 'MISSING_FORMAT'))"
          }
        })).await;
        i
      })
      .await
      .start_mock_server_async(Some("protobuf/transport/grpc"), None)
      .await;

    let url = mock_server.url();
    let mut client = catalog_service_client::CatalogServiceClient::connect(
      url.to_string()
    ).await.unwrap();

    let response = client.get_catalog_entry(tonic::Request::new(CatalogRequest {
      shelf: "main".to_string(),
      section: "fiction".to_string(),
    })).await;

    let msg = response.unwrap().into_inner();
    expect!(msg.title.as_str()).to(be_equal_to("REFERENCE"));
    // Mock returns MISSING_FORMAT as specified; the real test is provider verification
    expect!(msg.formats).to(be_equal_to(vec!["MISSING_FORMAT".to_string()]));
  }

  // ==========================================================================
  // Test 7: arrayContains with reference form on repeated MESSAGE field.
  //
  // This is the NEW reference form:
  //   "pact:match": "arrayContains(matching($'publicNet'))"
  // where publicNet is a sibling key defining the expected message structure.
  //
  // The consumer says: "The formats array must contain at least one
  // FormatDetail where type='HARDCOVER' and isbn matches any string"
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
      .synchronous_message_interaction("get catalog entry v2 - arrayContains ref", |mut i| async move {
        let proto_file = Path::new("../catalog.proto")
          .canonicalize().unwrap().to_string_lossy().to_string();
        i.contents_from(json!({
          "pact:proto": proto_file,
          "pact:content-type": "application/protobuf",
          "pact:proto-service": "CatalogServiceV2/GetCatalogEntryV2",
          "request": {
            "shelf": "matching(type, 'main')",
            "section": "matching(type, 'fiction')"
          },
          "response": {
            "title": "matching(type, 'REFERENCE')",
            "formats": {
              "pact:match": "arrayContains(matching($'publicNet'))",
              "publicNet": {
                "type": "matching(equalTo, 'HARDCOVER')",
                "isbn": "matching(type, '978-0000000003')"
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
    let mut client = catalog_service_v2_client::CatalogServiceV2Client::connect(
      url.to_string()
    ).await.unwrap();

    let response = client.get_catalog_entry_v2(tonic::Request::new(CatalogRequest {
      shelf: "main".to_string(),
      section: "fiction".to_string(),
    })).await;

    let msg = response.unwrap().into_inner();
    expect!(msg.title.as_str()).to(be_equal_to("REFERENCE"));
    assert!(!msg.formats.is_empty());
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
      .synchronous_message_interaction("get catalog entry v2 - arrayContains multi ref", |mut i| async move {
        let proto_file = Path::new("../catalog.proto")
          .canonicalize().unwrap().to_string_lossy().to_string();
        i.contents_from(json!({
          "pact:proto": proto_file,
          "pact:content-type": "application/protobuf",
          "pact:proto-service": "CatalogServiceV2/GetCatalogEntryV2",
          "request": {
            "shelf": "matching(type, 'main')",
            "section": "matching(type, 'fiction')"
          },
          "response": {
            "title": "matching(type, 'REFERENCE')",
            "formats": {
              "pact:match": "arrayContains(matching($'publicNet'), matching($'plNet'))",
              "publicNet": {
                "type": "matching(equalTo, 'HARDCOVER')",
                "isbn": "matching(type, '978-0000000003')"
              },
              "plNet": {
                "type": "matching(equalTo, 'PAPERBACK')",
                "isbn": "matching(type, '978-0000000001')"
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
    let mut client = catalog_service_v2_client::CatalogServiceV2Client::connect(
      url.to_string()
    ).await.unwrap();

    let response = client.get_catalog_entry_v2(tonic::Request::new(CatalogRequest {
      shelf: "main".to_string(),
      section: "fiction".to_string(),
    })).await;

    let msg = response.unwrap().into_inner();
    expect!(msg.title.as_str()).to(be_equal_to("REFERENCE"));
    // Both variants must be present in the example
    assert_eq!(msg.formats.len(), 2, "Expected 2 example elements (one per variant), got {}", msg.formats.len());
    let types: Vec<&str> = msg.formats.iter().map(|n| n.r#type.as_str()).collect();
    assert!(types.contains(&"HARDCOVER"), "Expected HARDCOVER variant in {:?}", types);
    assert!(types.contains(&"PAPERBACK"), "Expected PAPERBACK variant in {:?}", types);
  }
}
