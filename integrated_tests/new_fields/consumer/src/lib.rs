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

    let mut pact_builder = PactBuilderAsync::new_v4("grpc-consumer-rust", "new_fields");
    let mock_server = pact_builder
      .using_plugin("protobuf", None).await
      .synchronous_message_interaction("initial values request", |mut i| async move {
        let proto_file = Path::new("../new_fields.proto")
          .canonicalize().unwrap().to_string_lossy().to_string();
        i.contents_from(json!({
          "pact:proto": proto_file,
          "pact:content-type": "application/protobuf",
          "pact:proto-service": "UserService/GetUser",

          "request": {
            "id": "matching(regex, '\\d+', '1234')"
          },
          "response": {
            "id": "matching(type, '89b9475a-09d0-47a9-a4bc-2b6b9d361db6')",
            "display_name": "matching(type, 'xX5im0n-P3ggXx')",
            "first_name": "matching(type, 'Simon')",
            "surname": "matching(type, 'Pegg')",
          }
        })).await;
        i
      })
      .await
      .start_mock_server_async(Some("protobuf/transport/grpc"))
      .await;

    let url = mock_server.url();
    let mut client = user_service_client::UserServiceClient::connect(url.to_string()).await.unwrap();

    let request_message = GetUserRequest {
      id: "1234".to_string()
    };
    let response = client.get_user(tonic::Request::new(request_message)).await;
    let response_message = response.unwrap();
    expect!(response_message.get_ref().id.as_str()).to(be_equal_to("89b9475a-09d0-47a9-a4bc-2b6b9d361db6"));
  }
}
