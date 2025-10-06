tonic::include_proto!("sample");

#[cfg(test)]
mod tests {
  use std::path::Path;

  use expectest::prelude::*;
  use pact_consumer::prelude::*;
  use pact_consumer::mock_server::StartMockServerAsync;
  use serde_json::json;

  use super::*;

  #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
  async fn test_nested_messages() {
    let _ = env_logger::builder().is_test(true).try_init();

    let mut pact_builder = PactBuilderAsync::new_v4("grpc-consumer-rust", "nested_messages");
    let mock_server = pact_builder
      .using_plugin("protobuf", None).await
      .synchronous_message_interaction("request with nested messages", |mut i| async move {
        let proto_file = Path::new("sample.proto")
          .canonicalize().unwrap().to_string_lossy().to_string();
        i.contents_from(json!({
          "pact:proto": proto_file,
          "pact:content-type": "application/protobuf",
          "pact:proto-service": "SampleService/GetEntity",

          "request": {
            "id": "matching(type, '123')"
          },
          "response": {
            "entity": {
              "id": "matching(type, '123')",
              "details": {
                "name": {
                  "first": "matching(type, 'John')",
                  "last": "matching(type, 'Doe')"
                }
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
    let mut client = sample_service_client::SampleServiceClient::connect(url.to_string()).await.unwrap();

    let request_message = GetEntityRequest {
      id: "123".to_string()
    };
    
    let response = client.get_entity(tonic::Request::new(request_message)).await;
    let response_message = response.unwrap();
    
    // Verify the response structure - this tests that nested messages work correctly
    let entity = response_message.get_ref().entity.as_ref().unwrap();
    expect!(entity.id.as_str()).to(be_equal_to("123"));
    
    let details = entity.details.as_ref().unwrap();
    let name = details.name.as_ref().unwrap();
    expect!(name.first.as_str()).to(be_equal_to("John"));
    expect!(name.last.as_str()).to(be_equal_to("Doe"));
  }
}
