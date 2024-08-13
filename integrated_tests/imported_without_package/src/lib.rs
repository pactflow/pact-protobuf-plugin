tonic::include_proto!("mod");

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::primary::primary_client::PrimaryClient;
    use pact_consumer::mock_server::StartMockServerAsync;
    use pact_consumer::prelude::*;
    use serde_json::json;
    use tonic::IntoRequest;
    use tracing::info;

    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn test_proto_client() {
        let _ = env_logger::builder().is_test(true).try_init();

        let mut pact_builder: PactBuilderAsync = PactBuilderAsync::new_v4("grpc-consumer-rust", "imported_message");
        let mock_server = pact_builder
            .using_plugin("protobuf", None)
            .await
            .synchronous_message_interaction(
                "package namespace not respected",
                |mut i| async move {
                    let proto_file = Path::new("primary/service.proto")
                        .canonicalize()
                        .unwrap()
                        .to_string_lossy()
                        .to_string();
                    let proto_include = Path::new(".")
                        .canonicalize()
                        .unwrap()
                        .to_string_lossy()
                        .to_string();
                    info!("proto_file: {}", proto_file);
                    info!("proto_include: {}", proto_include);
                    i.contents_from(json!({
                        "pact:proto": proto_file,
                        "pact:proto-service": "Primary/GetRectangle",
                        "pact:content-type": "application/grpc",
                        "pact:protobuf-config": {
                            "additionalIncludes": [ proto_include ]
                        },
                        "request": {
                            "x": "matching(number, 180)",
                            "y": "matching(number, 200)",
                            "width": "matching(number, 10)",
                            "length": "matching(number, 20)",
                            "tag": {
                                "name": "matching(type, 'name')",
                                "value": "matching(type, 'value')"
                            }
                        },
                        "response": {
                            "rectangle": {
                                "lo": {
                                    "latitude": "matching(number, 180)",
                                    "longitude": "matching(number, 99)"
                                },
                                "hi": {
                                    "latitude": "matching(number, 200)",
                                    "longitude": "matching(number, 99)"
                                }
                            }
                        }
                    }))
                    .await;
                    i
                },
            )
            .await
            .start_mock_server_async(Some("protobuf/transport/grpc"), None)
            .await;

        let url = mock_server.url();

        let mut client = PrimaryClient::connect(url.to_string()).await.unwrap();
        let request_message = primary::RectangleLocationRequest {
            x: 180,
            y: 200,
            width: 10,
            length: 20,
            tag: Some(Tag{
                name: "name".to_string(),
                value: "value".to_string()
            })
        };
        let response = client.get_rectangle(request_message.into_request()).await;
        let _response_message = response.unwrap();
    }
}
