pub mod pb {
  tonic::include_proto!("catalog");
}

pub use pb::*;

/// Provider that returns MORE formats values than the consumer expects.
/// Consumer says ["HARDCOVER"], but we return ["HARDCOVER", "PAPERBACK", "AUDIOBOOK"].
#[derive(Default)]
pub struct CatalogServiceImpl {}

#[tonic::async_trait]
impl pb::catalog_service_server::CatalogService for CatalogServiceImpl {
  async fn get_catalog_entry(
    &self,
    _request: tonic::Request<CatalogRequest>,
  ) -> Result<tonic::Response<CatalogResponse>, tonic::Status> {
    Ok(tonic::Response::new(CatalogResponse {
      title: "REFERENCE".to_string(),
      languages: vec!["EN".to_string()],
      formats: vec![
        "HARDCOVER".to_string(),
        "PAPERBACK".to_string(),
        "AUDIOBOOK".to_string(),
      ],
    }))
  }
}

/// V2 provider with repeated MESSAGE field (FormatDetail).
/// Returns multiple FormatDetail entries — consumer only cares about one.
#[derive(Default)]
pub struct CatalogServiceV2Impl {}

#[tonic::async_trait]
impl pb::catalog_service_v2_server::CatalogServiceV2 for CatalogServiceV2Impl {
  async fn get_catalog_entry_v2(
    &self,
    _request: tonic::Request<CatalogRequest>,
  ) -> Result<tonic::Response<CatalogResponseV2>, tonic::Status> {
    Ok(tonic::Response::new(CatalogResponseV2 {
      title: "REFERENCE".to_string(),
      formats: vec![
        FormatDetail { r#type: "HARDCOVER".into(), isbn: "978-0000000003".into() },
        FormatDetail { r#type: "PAPERBACK".into(), isbn: "978-0000000001".into() },
        FormatDetail { r#type: "AUDIOBOOK".into(), isbn: "978-0000000002".into() },
      ],
    }))
  }
}

#[cfg(test)]
mod tests {
  use std::collections::HashMap;
  use std::net::SocketAddr;
  use std::sync::Arc;

  use async_trait::async_trait;
  use maplit::hashmap;
  use pact_models::prelude::ProviderState;
  use pact_verifier::{
    FilterInfo, NullRequestFilterExecutor, PactSource, ProviderInfo,
    ProviderTransport, VerificationOptions, verify_provider_async,
  };
  use pact_verifier::callback_executors::ProviderStateExecutor;
  use reqwest::Client;
  use serde_json::Value;
  use tonic::transport::Server;

  use super::*;
  use super::pb::catalog_service_server::CatalogServiceServer;
  use super::pb::catalog_service_v2_server::CatalogServiceV2Server;

  #[derive(Debug)]
  struct NoopProviderStateExecutor;

  #[async_trait]
  impl ProviderStateExecutor for NoopProviderStateExecutor {
    async fn call(
      self: Arc<Self>,
      _interaction_id: Option<String>,
      _provider_state: &ProviderState,
      _setup: bool,
      _client: Option<&Client>,
    ) -> anyhow::Result<HashMap<String, Value>> {
      Ok(hashmap! {})
    }
    fn teardown(self: &Self) -> bool { false }
  }

  async fn start_grpc_provider() -> SocketAddr {
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
      Server::builder()
        .add_service(CatalogServiceServer::new(CatalogServiceImpl::default()))
        .add_service(CatalogServiceV2Server::new(CatalogServiceV2Impl::default()))
        .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
        .await
        .unwrap();
    });

    addr
  }

  /// Verify a specific interaction from the consumer pact against the real provider.
  /// The provider returns ["HARDCOVER", "PAPERBACK", "AUDIOBOOK"] for formats.
  async fn verify_interaction(interaction_desc: &str) -> bool {
    let addr = start_grpc_provider().await;

    let pact_file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
      .join("../consumer/target/pacts/repeated-field-consumer-repeated-field-provider.json")
      .canonicalize()
      .expect("Run consumer tests first to generate pact file");

    #[allow(deprecated)]
    let provider_info = ProviderInfo {
      name: "repeated-field-provider".to_string(),
      host: "127.0.0.1".to_string(),
      port: Some(addr.port()),
      transports: vec![ProviderTransport {
        transport: "grpc".to_string(),
        port: Some(addr.port()),
        path: None,
        scheme: None,
      }],
      .. ProviderInfo::default()
    };

    let source = PactSource::File(pact_file.to_string_lossy().to_string());
    let filter = FilterInfo::Description(interaction_desc.to_string());

    let options: VerificationOptions<NullRequestFilterExecutor> = VerificationOptions::default();
    let ps_executor = NoopProviderStateExecutor;

    let result = verify_provider_async(
      provider_info,
      vec![source],
      filter,
      vec![],
      &options,
      None,
      &Arc::new(ps_executor),
      None,
    ).await;

    match result {
      Ok(res) => {
        println!("\n=== Verification result for '{}': {} ===", interaction_desc, if res.result { "PASS" } else { "FAIL" });
        if !res.result {
          for error in &res.errors {
            println!("  ERROR: {:?}", error);
          }
        }
        res.result
      }
      Err(e) => {
        println!("\n=== Verification ERROR for '{}': {:?} ===", interaction_desc, e);
        false
      }
    }
  }

  // ========================================================
  // Test 1: Exact match — should FAIL
  // Consumer expects ["HARDCOVER"], provider returns ["HARDCOVER", "PAPERBACK", "AUDIOBOOK"]
  // ========================================================
  #[test_log::test(tokio::test(flavor = "multi_thread", worker_threads = 1))]
  async fn verify_exact_match_fails() {
    let _ = env_logger::builder().is_test(true).try_init();
    let result = verify_interaction("get catalog entry - exact match").await;
    assert!(!result, "Expected verification to FAIL (exact match with different array length)");
  }

  // ========================================================
  // Test 2: eachValue(matching(type, ...)) — should PASS
  // The type matcher should accept any string values regardless of count
  // ========================================================
  #[test_log::test(tokio::test(flavor = "multi_thread", worker_threads = 1))]
  async fn verify_each_value_type_passes() {
    let _ = env_logger::builder().is_test(true).try_init();
    let result = verify_interaction("get catalog entry - eachValue type").await;
    assert!(result, "Expected verification to PASS (eachValue type matching accepts any strings)");
  }

  // ========================================================
  // Test 3: atLeast(1), atMost(3), eachValue(matching(type, ...)) — should PASS
  // Provider returns 3 elements which is within [1, 3] range
  // ========================================================
  #[test_log::test(tokio::test(flavor = "multi_thread", worker_threads = 1))]
  async fn verify_at_least_at_most_passes() {
    let _ = env_logger::builder().is_test(true).try_init();
    let result = verify_interaction("get catalog entry - atLeast atMost").await;
    assert!(result, "Expected verification to PASS (3 elements within atLeast(1) atMost(3))");
  }

  // ========================================================
  // Test 4: atLeast(1) only — should PASS
  // Provider returns 3 elements, which is >= 1
  // KNOWN BUG: atLeast alone puts MinType on $.formats.* but nothing on $.formats,
  // so compare_repeated_field falls to exact matching. Tracked separately.
  // ========================================================
  #[test_log::test(tokio::test(flavor = "multi_thread", worker_threads = 1))]
  async fn verify_at_least_only_fails_known_bug() {
    let _ = env_logger::builder().is_test(true).try_init();
    let result = verify_interaction("get catalog entry - atLeast only").await;
    assert!(!result, "Known bug: atLeast alone does exact matching (should pass but doesn't)");
  }

  // ========================================================
  // Test 5: arrayContains(matching(equalTo, 'HARDCOVER')) — should PASS
  // Provider returns ["HARDCOVER", "PAPERBACK", "AUDIOBOOK"]
  // which CONTAINS "HARDCOVER", so arrayContains should be satisfied.
  // ========================================================
  #[test_log::test(tokio::test(flavor = "multi_thread", worker_threads = 1))]
  async fn verify_array_contains_passes() {
    let _ = env_logger::builder().is_test(true).try_init();
    let result = verify_interaction("get catalog entry - arrayContains equalTo").await;
    assert!(result, "Expected verification to PASS (array contains HARDCOVER)");
  }

  // ========================================================
  // Test 6: arrayContains(matching(equalTo, 'MISSING_FORMAT')) — should FAIL
  // Provider returns ["HARDCOVER", "PAPERBACK", "AUDIOBOOK"]
  // which does NOT contain "MISSING_FORMAT".
  // ========================================================
  #[test_log::test(tokio::test(flavor = "multi_thread", worker_threads = 1))]
  async fn verify_array_contains_fails_when_missing() {
    let _ = env_logger::builder().is_test(true).try_init();
    let result = verify_interaction("get catalog entry - arrayContains missing").await;
    assert!(!result, "Expected verification to FAIL (array does not contain MISSING_FORMAT)");
  }

  // ========================================================
  // Test 7: arrayContains with reference form on repeated MESSAGE field — should PASS
  // Consumer expects formats to contain a FormatDetail with type="HARDCOVER".
  // Provider returns [HARDCOVER, PAPERBACK, AUDIOBOOK] FormatDetails.
  // The isbn field uses type matching, so any string value is accepted.
  // ========================================================
  #[test_log::test(tokio::test(flavor = "multi_thread", worker_threads = 1))]
  async fn verify_array_contains_ref_passes() {
    let _ = env_logger::builder().is_test(true).try_init();
    let result = verify_interaction("get catalog entry v2 - arrayContains ref").await;
    assert!(result, "Expected verification to PASS (formats contains a HARDCOVER FormatDetail)");
  }
}
