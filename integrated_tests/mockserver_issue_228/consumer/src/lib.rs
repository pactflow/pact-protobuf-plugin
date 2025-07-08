tonic::include_proto!("list_appender");

#[cfg(test)]
mod tests {
  use std::path::Path;

  use expectest::prelude::*;
  use pact_consumer::mock_server::StartMockServerAsync;
  use pact_consumer::prelude::*;
  use serde_json::json;

  use crate::list_appender_client::ListAppenderClient;

  use super::*;

  #[test_log::test(tokio::test(flavor = "multi_thread"))]
  async fn test_proto_client() {
    let proto_file = Path::new("../list_appender.proto")
      .canonicalize().unwrap().to_string_lossy().to_string();

    let mut pact_builder = PactBuilderAsync::new_v4("list-appender-consumer", "list-appender");
    let mock_server = pact_builder
      .using_plugin("protobuf", None).await
      .synchronous_message_interaction("something to append", |mut i| {
        let proto_file = proto_file.clone();
        async move {
          i.contents_from(json!({
            "pact:proto": proto_file,
            "pact:content-type": "application/protobuf",
            "pact:proto-service": "ListAppender/append",

            "request": {
              "start": 0,
              "additional": [1,2,3]
            },
            "response": {
              "value": [0,1,2,3]
            }
          })).await;
          i
        }
      })
      .await
      .synchronous_message_interaction("nothing to append", |mut i| {
        let proto_file = proto_file.clone();
        async move {
          i.contents_from(json!({
            "pact:proto": proto_file,
            "pact:content-type": "application/protobuf",
            "pact:proto-service": "ListAppender/append",

            "request": {
              "start": 0,
              "additional": []
            },
            "response": {
              "value": [0]
            }
          })).await;
          i
        }
      })
      .await
      .start_mock_server_async(Some("protobuf/transport/grpc"), None)
      .await;

    let url = mock_server.url();
    let mut client = ListAppenderClient::connect(url.to_string()).await.unwrap();

    let request_message = AppendRequest {
      start: 0,
      additional: vec![1, 2, 3]
    };
    let response = client.append(tonic::Request::new(request_message)).await;
    let (_, response_message, _) = response.unwrap().into_parts();
    expect!(response_message.value).to(be_equal_to(vec![0, 1, 2, 3]));

    let request_message = AppendRequest {
      start: 0,
      additional: vec![]
    };
    let response = client.append(tonic::Request::new(request_message)).await;
    let (_, response_message, _) = response.unwrap().into_parts();
    expect!(response_message.value).to(be_equal_to(vec![0]));
  }
}
