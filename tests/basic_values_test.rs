use std::path::Path;
use expectest::prelude::*;

use pact_consumer::builders::PactBuilderAsync;
use pact_models::json_utils::json_to_string;
use prost::encoding::WireType::{LengthDelimited, SixtyFourBit, Varint};
use serde_json::json;

use pact_protobuf_plugin::message_decoder::{decode_message, ProtobufField};
use pact_protobuf_plugin::message_decoder::ProtobufFieldData::{Boolean, Double, Integer32, String, UInteger32};
use pact_protobuf_plugin::utils::{find_message_type_by_name, get_descriptors_for_interaction, lookup_interaction_config};

#[test_log::test(tokio::test(flavor = "multi_thread"))]
async fn basic_values_test() {
  let mut pact_builder = PactBuilderAsync::new_v4("basic_values", "protobuf-plugin");
  pact_builder
    .using_plugin("protobuf", None).await
    .synchronous_message_interaction("message with basic values", |mut i| async move {
      let proto_file = Path::new("tests/basic_values.proto")
        .canonicalize().unwrap().to_string_lossy().to_string();
      i.contents_from(json!({
        "pact:proto": proto_file,
        "pact:content-type": "application/protobuf",
        "pact:proto-service": "Test/GetTest",

        "request": {
          "f1": true,
          "f2": -1122,
          "f3": 1122,
          "f4": 1122.33,
          "f5": "1122.33",
          // "f6": [1, 2, 3, 4]
        },

        "response": {
          "out": true
        }
      })).await;
      i
    })
    .await;

  let pact = pact_builder.build().as_v4_pact().unwrap();
  let plugin_config = pact.plugin_data.iter()
    .find(|data| data.name == "protobuf")
    .map(|data| &data.configuration)
    .unwrap()
    .iter()
    .map(|(k, v)| (k.clone(), v.clone()))
    .collect();
  let interaction = pact.interactions.first().unwrap().as_v4_sync_message().unwrap();
  let request = &interaction.request;
  let interaction_config = lookup_interaction_config(&interaction).unwrap();
  let descriptor_key = interaction_config.get("descriptorKey").map(json_to_string).unwrap();
  let fds = get_descriptors_for_interaction(descriptor_key.as_str(), &plugin_config).unwrap();
  let (message_descriptor, _) = find_message_type_by_name("MessageIn", &fds).unwrap();
  let mut buffer = request.contents.value().unwrap();

  let fields = decode_message(&mut buffer, &message_descriptor, &fds).unwrap();
  expect!(fields).to(be_equal_to(vec![
    ProtobufField { field_num: 1, field_name: "f1".to_string(), wire_type: Varint, data: Boolean(true) },
    ProtobufField { field_num: 2, field_name: "f2".to_string(), wire_type: Varint, data: Integer32(-1122) },
    ProtobufField { field_num: 3, field_name: "f3".to_string(), wire_type: Varint, data: UInteger32(1122) },
    ProtobufField { field_num: 4, field_name: "f4".to_string(), wire_type: SixtyFourBit, data: Double(1122.33) },
    ProtobufField { field_num: 5, field_name: "f5".to_string(), wire_type: LengthDelimited, data: String("1122.33".to_string()) }
  ]));
}
