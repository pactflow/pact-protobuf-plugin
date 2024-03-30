tonic::include_proto!("pactissue");

#[cfg(test)]
mod tests {
    use std::path::Path;

    use expectest::prelude::*;
    use maplit::hashmap;
    use pact_consumer::mock_server::StartMockServerAsync;
    use pact_consumer::prelude::*;
    use serde_json::json;

    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn test_proto_client() {
        let _ = env_logger::builder().is_test(true).try_init();

        let mut pact_builder = PactBuilderAsync::new_v4("grpc-consumer-rust", "matching_maps");
        let mock_server = pact_builder
      .using_plugin("protobuf", None).await
      .synchronous_message_interaction("example request", |mut i| async move {
        let proto_file = Path::new("matching_maps.proto")
          .canonicalize().unwrap().to_string_lossy().to_string();
        i.contents_from(json!({
          "pact:proto": proto_file,
          "pact:content-type": "application/protobuf",
          "pact:proto-service": "ExampleService/GetSample",

          "request": {
            "values": {
              "pact:match": "eachKey(matching(regex, '\\d+', '100')), eachValue(matching($'example'))",
              "example": {
                "value": "matching(type, 'ABC')"
              },
            },
            "ok": "true"
          },
          "response": {
            "ok": "matching(boolean, true)"
          }
        })).await;
        i
      })
      .await
      .start_mock_server_async(Some("protobuf/transport/grpc"))
      .await;

        let url = mock_server.url();
        let mut client = example_service_client::ExampleServiceClient::connect(url.to_string())
            .await
            .unwrap();

        let request_message = ExampleRequest {
            values: hashmap! {
              "12324".to_string() => ExampleValue{
                value: "ABC".to_string(),
              },
              "2233211".to_string() => ExampleValue {
                value: "XYZ".to_string()
              }
            },
            ok: true,
        };
        let response = client
            .get_sample(tonic::Request::new(request_message))
            .await;
        let response_message = response.unwrap();
        expect!(response_message.get_ref().ok).to(be_true());
    }
}
