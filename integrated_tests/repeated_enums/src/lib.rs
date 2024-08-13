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

    let mut pact_builder = PactBuilderAsync::new_v4("grpc-consumer-rust", "repeated_enum2");
    let mock_server = pact_builder
      .using_plugin("protobuf", None).await
      .synchronous_message_interaction("broken enum request", |mut i| async move {
        let proto_file = Path::new("repeated_enum.proto")
          .canonicalize().unwrap().to_string_lossy().to_string();
        i.contents_from(json!({
          "pact:proto": proto_file,
          "pact:content-type": "application/protobuf",
          "pact:proto-service": "BrokenSampleService/GetSample",

          "request": {
            "type": "notEmpty('TYPE1')"
          },
          "response": {
            "ok": "matching(boolean, true)"
          }
        })).await;
        i
      })
      .await
      .start_mock_server_async(Some("protobuf/transport/grpc"), None)
      .await;

    let url = mock_server.url();
    let mut client = broken_sample_service_client::BrokenSampleServiceClient::connect(url.to_string()).await.unwrap();

    let request_message = BrokenSampleRequest {
      r#type: vec![ broken_sample_request::Type::Type2.into() ]
    };
    let response = client.get_sample(tonic::Request::new(request_message)).await;
    let response_message = response.unwrap();
    expect!(response_message.get_ref().ok).to(be_true());
  }
}
