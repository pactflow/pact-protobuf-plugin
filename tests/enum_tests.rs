use std::path::Path;

use expectest::prelude::*;
use maplit::hashmap;
use pact_consumer::builders::PactBuilderAsync;
use pact_matching::{BodyMatchResult, CoreMatchingContext, DiffConfig};
use pact_models::json_utils::json_to_string;
use pact_models::path_exp::DocPath;
use prost::encoding::WireType;
use serde_json::json;
use pact_protobuf_plugin::matching::compare_message;
use pact_protobuf_plugin::message_decoder::{ProtobufField, ProtobufFieldData};
use pact_protobuf_plugin::utils::{find_message_type_by_name, get_descriptors_for_interaction, lookup_interaction_config};

#[test_log::test(tokio::test(flavor = "multi_thread"))]
async fn repeated_enum_test() {
  let mut pact_builder = PactBuilderAsync::new_v4("repeated_enum", "protobuf-plugin");
  pact_builder
    .using_plugin("protobuf", None).await
    .message_interaction("get a list of enums", |mut i| async move {
      let proto_file = Path::new("tests/enum.proto")
        .canonicalize().unwrap().to_string_lossy().to_string();
      i.contents_from(json!({
        "pact:proto": proto_file,
        "pact:message-type": "MessageIn",
        "pact:content-type": "application/protobuf",

        "in": [
          "matching(equalTo, 'A')"
        ]
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

  for message in pact_builder.messages() {
    let interaction_config = lookup_interaction_config(&message).unwrap();
    let descriptor_key = interaction_config.get("descriptorKey")
      .map(json_to_string).unwrap();
    let fds = get_descriptors_for_interaction(descriptor_key.as_str(),
      &plugin_config).unwrap();
    let path = DocPath::root();
    let context = CoreMatchingContext::new(DiffConfig::NoUnexpectedKeys,
      &message.contents.matching_rules.rules_for_category("body").unwrap(), &hashmap!{});
    let (message_descriptor, fs) = find_message_type_by_name("MessageIn", &fds).unwrap();
    let enum_descriptor = fs.enum_type.first().unwrap();
    let expected = vec![
      ProtobufField {
        field_num: 1,
        field_name: "in".to_string(),
        wire_type: WireType::LengthDelimited,
        data: ProtobufFieldData::Enum(1, enum_descriptor.clone())
      }
    ];
    let actual = vec![
      ProtobufField {
        field_num: 1,
        field_name: "in".to_string(),
        wire_type: WireType::LengthDelimited,
        data: ProtobufFieldData::Enum(1, enum_descriptor.clone())
      },
      // ProtobufField {
      //   field_num: 1,
      //   field_name: "in".to_string(),
      //   wire_type: WireType::LengthDelimited,
      //   data: ProtobufFieldData::Enum(1, enum_descriptor.clone())
      // }
    ];

    let result = compare_message(
      path,
      &expected,
      &actual,
      &context,
      &message_descriptor,
      &fds,
    ).unwrap();

    expect!(result).to(be_equal_to(BodyMatchResult::Ok));
  }
}
