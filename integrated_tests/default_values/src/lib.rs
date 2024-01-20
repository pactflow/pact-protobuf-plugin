tonic::include_proto!("pactissue");

#[cfg(test)]
mod tests {
  use std::path::Path;

  use expectest::prelude::*;
  use pact_consumer::prelude::*;
  use pact_consumer::mock_server::StartMockServerAsync;
  use serde_json::json;

  use super::*;

  #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
  async fn test_proto_client() {
    let _ = env_logger::builder().is_test(true).try_init();

    let mut pact_builder = PactBuilderAsync::new_v4("grpc-consumer-rust", "default_values");
    let mock_server = pact_builder
      .using_plugin("protobuf", None).await
      .synchronous_message_interaction("default values request", |mut i| async move {
        let proto_file = Path::new("default_value.proto")
          .canonicalize().unwrap().to_string_lossy().to_string();
        i.contents_from(json!({
          "pact:proto": proto_file,
          "pact:content-type": "application/protobuf",
          "pact:proto-service": "Test/GetTest",

          "request": {
            "in": "matching(boolean, false)",
            "e": "matching(type, 'A')",
            "s": "matching(type, '')"
          },
          "response": {
            "out": "matching(boolean, false)",
            "e": "matching(type, 'A')"
          }
        })).await;
        i
      })
      .await
      .start_mock_server_async(Some("protobuf/transport/grpc"))
      .await;

    let url = mock_server.url();
    let mut client = test_client::TestClient::connect(url.to_string()).await.unwrap();

    let request_message = MessageIn {
      r#in: false,
      e: TestDefault::A.into(),
      s: String::default()
    };
    let response = client.get_test(tonic::Request::new(request_message)).await;
    let response_message = response.unwrap();
    expect!(response_message.get_ref().out).to(be_false());
  }
}
