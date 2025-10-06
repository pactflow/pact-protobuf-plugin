use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use bytes::Bytes;
use expectest::prelude::*;
use lazy_static::lazy_static;
use maplit::{btreemap, hashmap};
use pact_models::{matchingrules, matchingrules_list};
use pact_models::matchingrules::expressions::{MatchingRuleDefinition, ValueType};
use pact_models::path_exp::DocPath;
use pact_models::prelude::MatchingRuleCategory;
use pact_plugin_driver::proto::{MatchingRule, MatchingRules};
use pact_plugin_driver::proto::interaction_response::MarkupType;
use pretty_assertions::assert_eq;
use prost::Message;
use prost_types::{
  DescriptorProto,
  field_descriptor_proto,
  FieldDescriptorProto,
  FileDescriptorProto,
  FileDescriptorSet,
  MethodDescriptorProto,
  MethodOptions,
  OneofDescriptorProto,
  ServiceDescriptorProto,
  Struct
};
use prost_types::field_descriptor_proto::{Label, Type};
use prost_types::value::Kind::{ListValue, NullValue, NumberValue, StringValue, StructValue};
use serde_json::{json, Value};
use trim_margin::MarginTrimmable;

use crate::message_builder::{MessageBuilder, MessageFieldValue, MessageFieldValueType, RType};
use crate::protobuf::{
  build_embedded_message_field_value,
  build_single_embedded_field_value,
  construct_message_field,
  construct_protobuf_interaction_for_message,
  construct_protobuf_interaction_for_service,
  configure_protobuf_service,
  request_part,
  response_part,
  value_for_type
};
use crate::utils::DescriptorCache;

mod build_field_value_tests;

#[test]
fn value_for_type_test() {
  let message_descriptor = DescriptorProto {
    name: None,
    field: vec![],
    extension: vec![],
    nested_type: vec![],
    enum_type: vec![],
    extension_range: vec![],
    oneof_decl: vec![],
    options: None,
    reserved_range: vec![],
    reserved_name: vec![]
  };
  let descriptor = FieldDescriptorProto {
    name: None,
    number: None,
    label: None,
    r#type: Some(Type::String as i32),
    type_name: Some("test".to_string()),
    extendee: None,
    default_value: None,
    oneof_index: None,
    json_name: None,
    options: None,
    proto3_optional: None
  };
  let result = value_for_type("test", "test", &descriptor, &message_descriptor, &DescriptorCache::new(prost_types::FileDescriptorSet { file: vec![] })).unwrap();
  expect!(result.name).to(be_equal_to("test"));
  expect!(result.raw_value).to(be_some().value("test".to_string()));
  expect!(result.rtype).to(be_equal_to(RType::String("test".to_string())));

  let descriptor = FieldDescriptorProto {
    name: None,
    number: None,
    label: None,
    r#type: Some(Type::Uint64 as i32),
    type_name: Some("uint64".to_string()),
    extendee: None,
    default_value: None,
    oneof_index: None,
    json_name: None,
    options: None,
    proto3_optional: None
  };
  let result = value_for_type("test", "100", &descriptor, &message_descriptor, &DescriptorCache::new(prost_types::FileDescriptorSet { file: vec![] })).unwrap();
  expect!(result.name).to(be_equal_to("test"));
  expect!(result.raw_value).to(be_some().value("100".to_string()));
  expect!(result.rtype).to(be_equal_to(RType::UInteger64(100)));
}

#[test]
fn construct_protobuf_interaction_for_message_test() {
  // construct_protobuf_interaction_for_message doesn't actually verify
  // that the message descriptor is part of a file descriptor
  // so it doesn't have to be here as well
  // It will still assume that the message came from this file descriptor and will use the package field from
  // the file descriptor as the message package too.
  let file_descriptor = FileDescriptorProto {
    name: Some("test_file.proto".to_string()),
    package: Some("test_package".to_string()),
    dependency: vec![],
    public_dependency: vec![],
    weak_dependency: vec![],
    message_type: vec![],
    enum_type: vec![],
    service: vec![],
    extension: vec![],
    options: None,
    source_code_info: None,
    syntax: None
  };
  let message_descriptor = DescriptorProto {
    name: Some("test_message".to_string()),
    field: vec![
      FieldDescriptorProto {
        name: Some("implementation".to_string()),
        number: Some(1),
        label: None,
        r#type: Some(field_descriptor_proto::Type::String as i32),
        type_name: Some("string".to_string()),
        extendee: None,
        default_value: None,
        oneof_index: None,
        json_name: None,
        options: None,
        proto3_optional: None
      },
      FieldDescriptorProto {
        name: Some("version".to_string()),
        number: Some(2),
        label: None,
        r#type: Some(field_descriptor_proto::Type::String as i32),
        type_name: Some("string".to_string()),
        extendee: None,
        default_value: None,
        oneof_index: None,
        json_name: None,
        options: None,
        proto3_optional: None
      },
      FieldDescriptorProto {
        name: Some("length".to_string()),
        number: Some(3),
        label: None,
        r#type: Some(field_descriptor_proto::Type::Int64 as i32),
        type_name: Some("int64".to_string()),
        extendee: None,
        default_value: None,
        oneof_index: None,
        json_name: None,
        options: None,
        proto3_optional: None
      },
      FieldDescriptorProto {
        name: Some("hash".to_string()),
        number: Some(4),
        label: None,
        r#type: Some(field_descriptor_proto::Type::Uint64 as i32),
        type_name: Some("uint64".to_string()),
        extendee: None,
        default_value: None,
        oneof_index: None,
        json_name: None,
        options: None,
        proto3_optional: None
      }
    ],
    extension: vec![],
    nested_type: vec![],
    enum_type: vec![],
    extension_range: vec![],
    oneof_decl: vec![],
    options: None,
    reserved_range: vec![],
    reserved_name: vec![]
  };
  let config = btreemap! {
      "implementation".to_string() => prost_types::Value { kind: Some(StringValue("notEmpty('plugin-driver-rust')".to_string())) },
      "version".to_string() => prost_types::Value { kind: Some(StringValue("matching(semver, '0.0.0')".to_string())) },
      "hash".to_string() => prost_types::Value { kind: Some(StringValue("matching(integer, 1234)".to_string())) }
    };

  let result = construct_protobuf_interaction_for_message(&message_descriptor, &config,
                                                          "", &file_descriptor, &DescriptorCache::new(prost_types::FileDescriptorSet { file: vec![file_descriptor.clone()] }), None).unwrap();

  let body = result.contents.as_ref().unwrap();
  expect!(body.content_type.as_str()).to(be_equal_to("application/protobuf;message=.test_package.test_message"));
  expect!(body.content_type_hint).to(be_equal_to(2));
  expect!(body.content.as_ref()).to(be_some().value(&vec![
    10, // field 1 length encoded (1 << 3 + 2 == 10)
    18, // 18 bytes
    112, 108, 117, 103, 105, 110, 45, 100, 114, 105, 118, 101, 114, 45, 114, 117, 115, 116,
    18, // field 2 length encoded (2 << 3 + 2 == 18)
    5, // 5 bytes
    48, 46, 48, 46, 48,
    32, // field 4 varint encoded (4 << 3 + 0 == 32)
    210, 9 // 9 << 7 + 210 == 1234
  ]));

  expect!(result.rules).to(be_equal_to(hashmap! {
      "$.implementation".to_string() => MatchingRules { rule: vec![ MatchingRule { r#type: "not-empty".to_string(), .. MatchingRule::default() } ] },
      "$.version".to_string() => MatchingRules { rule: vec![ MatchingRule { r#type: "semver".to_string(), .. MatchingRule::default() } ] },
      "$.hash".to_string() => MatchingRules { rule: vec![ MatchingRule { r#type: "integer".to_string(), .. MatchingRule::default() } ] }
    }));

  expect!(result.generators).to(be_equal_to(hashmap! {}));

  expect!(result.interaction_markup_type).to(be_equal_to(MarkupType::CommonMark as i32));
  expect!(result.interaction_markup).to(be_equal_to(
    "|```protobuf
      |message test_message {
      |    string implementation = 1;
      |    string version = 2;
      |    uint64 hash = 4;
      |}
      |```
      |".trim_margin().unwrap()));

  let interaction_config = result.plugin_configuration.unwrap().interaction_configuration.unwrap();
  expect!(interaction_config.fields).to(be_equal_to(btreemap!{
      "expectations".to_string() => prost_types::Value {
        kind: Some(StructValue(Struct {
          fields: config
        }))
      }
    }));
}

const DESCRIPTORS_FOR_EACH_VALUE_TEST: [u8; 267] = [
  10, 136, 2, 10, 12, 115, 105, 109, 112, 108, 101, 46, 112, 114, 111,
  116, 111, 34, 27, 10, 9, 77, 101, 115, 115, 97, 103, 101, 73, 110, 18, 14, 10, 2, 105, 110,
  24, 1, 32, 1, 40, 8, 82, 2, 105, 110, 34, 30, 10, 10, 77, 101, 115, 115, 97, 103, 101, 79,
  117, 116, 18, 16, 10, 3, 111, 117, 116, 24, 1, 32, 1, 40, 8, 82, 3, 111, 117, 116, 34, 39,
  10, 15, 86, 97, 108, 117, 101, 115, 77, 101, 115, 115, 97, 103, 101, 73, 110, 18, 20, 10, 5,
  118, 97, 108, 117, 101, 24, 1, 32, 3, 40, 9, 82, 5, 118, 97, 108, 117, 101, 34, 40, 10, 16,
  86, 97, 108, 117, 101, 115, 77, 101, 115, 115, 97, 103, 101, 79, 117, 116, 18, 20, 10, 5,
  118, 97, 108, 117, 101, 24, 1, 32, 3, 40, 9, 82, 5, 118, 97, 108, 117, 101, 50, 96, 10, 4,
  84, 101, 115, 116, 18, 36, 10, 7, 71, 101, 116, 84, 101, 115, 116, 18, 10, 46, 77, 101, 115,
  115, 97, 103, 101, 73, 110, 26, 11, 46, 77, 101, 115, 115, 97, 103, 101, 79, 117, 116, 34,
  0, 18, 50, 10, 9, 71, 101, 116, 86, 97, 108, 117, 101, 115, 18, 16, 46, 86, 97, 108, 117,
  101, 115, 77, 101, 115, 115, 97, 103, 101, 73, 110, 26, 17, 46, 86, 97, 108, 117, 101, 115,
  77, 101, 115, 115, 97, 103, 101, 79, 117, 116, 34, 0, 98, 6, 112, 114, 111, 116, 111, 51];

#[test_log::test]
fn construct_protobuf_interaction_for_message_with_each_value_matcher() {
  let fds = FileDescriptorSet::decode(DESCRIPTORS_FOR_EACH_VALUE_TEST.as_slice()).unwrap();
  let fs = fds.file.first().unwrap();
  let descriptor_cache = DescriptorCache::new(fds.clone());
  let config = btreemap! {
      "value".to_string() => prost_types::Value { kind: Some(prost_types::value::Kind::StringValue("eachValue(matching(type, '00000000000000000000000000000000'))".to_string())) }
    };
  let (message_descriptor, _) = descriptor_cache.find_message_descriptor_for_type(".ValuesMessageIn").unwrap();

  let result = construct_protobuf_interaction_for_message(
    &message_descriptor,
    &config,
    "",
    fs,
    &descriptor_cache,
    None
  ).unwrap();

  let body = result.contents.as_ref().unwrap();
  expect!(body.content_type.as_str()).to(be_equal_to("application/protobuf;message=.ValuesMessageIn"));
  expect!(body.content.as_ref()).to(be_some().value(&vec![
    10, // field 1 length encoded (1 << 3 + 2 == 10)
    32, // 32 bytes
    48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, // Lots of zeros
    48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48
  ]));

  let value_matcher = result.rules.get("$.value").unwrap().rule.first().unwrap();
  expect!(&value_matcher.r#type).to(be_equal_to("each-value"));
  let values = value_matcher.values.clone().unwrap();
  expect!(values.fields.get("value").unwrap().kind.clone().unwrap()).to(be_equal_to(
    StringValue("00000000000000000000000000000000".to_string())
  ));
  expect!(result.generators).to(be_equal_to(hashmap! {}));
}

#[test_log::test]
fn construct_message_field_with_message_with_each_value_matcher() {
  let fds = FileDescriptorSet::decode(DESCRIPTORS_FOR_EACH_VALUE_TEST.as_slice()).unwrap();
  let fs = fds.file.first().unwrap();
  let descriptor_cache = DescriptorCache::new(fds.clone());
  let (message_descriptor, _) = descriptor_cache.find_message_descriptor_for_type(".ValuesMessageIn").unwrap();
  let mut message_builder = MessageBuilder::new(&message_descriptor, "ValuesMessageIn", fs);
  let path = DocPath::new("$.value").unwrap();
  let mut matching_rules = MatchingRuleCategory::empty("body");
  let mut generators = hashmap!{};

  let result = construct_message_field(&mut message_builder, &mut matching_rules,
                                       &mut generators, "value", &Value::String("eachValue(matching(type, '00000000000000000000000000000000'))".to_string()),
                                       &path, &descriptor_cache);
  expect!(result).to(be_ok());

  let field = message_builder.fields.get("value");
  expect!(field).to(be_some());
  let inner = field.unwrap();
  expect!(inner.values.clone()).to(be_equal_to(vec![
    MessageFieldValue {
      name: "value".to_string(),
      raw_value: Some("00000000000000000000000000000000".to_string()),
      rtype: RType::String("00000000000000000000000000000000".to_string())
    }
  ]));

  expect!(matching_rules).to(be_equal_to(matchingrules_list! {
      "body";
      "$.value" => [
        pact_models::matchingrules::MatchingRule::EachValue(
          MatchingRuleDefinition::new("00000000000000000000000000000000".to_string(),
            ValueType::String, pact_models::matchingrules::MatchingRule::Type, None,
            "eachValue(matching(type, '00000000000000000000000000000000'))".to_string())
        )
      ]
    }));
}

#[test]
fn construct_protobuf_interaction_for_service_returns_error_on_invalid_request_type() {
  let string_descriptor = DescriptorProto {
    name: Some("StringValue".to_string()),
    field: vec![
      FieldDescriptorProto {
        name: Some("value".to_string()),
        number: Some(1),
        label: None,
        r#type: Some(field_descriptor_proto::Type::String as i32),
        type_name: Some("string".to_string()),
        extendee: None,
        default_value: None,
        oneof_index: None,
        json_name: None,
        options: None,
        proto3_optional: None
      }
    ],
    extension: vec![],
    nested_type: vec![],
    enum_type: vec![],
    extension_range: vec![],
    oneof_decl: vec![],
    options: None,
    reserved_range: vec![],
    reserved_name: vec![]
  };
  let message_descriptor = DescriptorProto {
    name: Some("test_message".to_string()),
    field: vec![
      FieldDescriptorProto {
        name: Some("value".to_string()),
        number: Some(1),
        label: None,
        r#type: Some(field_descriptor_proto::Type::String as i32),
        type_name: Some("string".to_string()),
        extendee: None,
        default_value: None,
        oneof_index: None,
        json_name: None,
        options: None,
        proto3_optional: None
      }
    ],
    extension: vec![],
    nested_type: vec![],
    enum_type: vec![],
    extension_range: vec![],
    oneof_decl: vec![],
    options: None,
    reserved_range: vec![],
    reserved_name: vec![]
  };
  let file_descriptor: FileDescriptorProto = FileDescriptorProto {
    name: Some("test_file.proto".to_string()),
    package: Some("test_package".to_string()),
    dependency: vec![],
    public_dependency: vec![],
    weak_dependency: vec![],
    message_type: vec![ string_descriptor, message_descriptor ],
    enum_type: vec![],
    service: vec![],
    extension: vec![],
    options: None,
    source_code_info: None,
    syntax: None
  };
  let service_descriptor = ServiceDescriptorProto {
    name: Some("test_service".to_string()),
    method: vec![
      MethodDescriptorProto {
        name: Some("call".to_string()),
        input_type: Some(".test_package.StringValue".to_string()),
        output_type: Some(".test_package.test_message".to_string()),
        options: None,
        client_streaming: None,
        server_streaming: None
      }
    ],
    options: None
  };

  let config = btreemap! {
      "request".to_string() => prost_types::Value { kind: Some(prost_types::value::Kind::BoolValue(true)) }
    };

  let result = construct_protobuf_interaction_for_service(
    &service_descriptor, &config, "call", &DescriptorCache::new(prost_types::FileDescriptorSet { file: vec![file_descriptor.clone()] }));
  expect!(result.as_ref()).to(be_err());
  expect!(result.unwrap_err().to_string()).to(
    be_equal_to("Request contents is of an un-processable type: BoolValue(true), it should be either a Struct or a StringValue")
  );
}

#[test_log::test]
fn construct_protobuf_interaction_for_service_supports_string_value_type() {
  let string_descriptor = DescriptorProto {
    name: Some("StringValue".to_string()),
    field: vec![
      FieldDescriptorProto {
        name: Some("value".to_string()),
        number: Some(1),
        label: None,
        r#type: Some(field_descriptor_proto::Type::String as i32),
        type_name: Some("string".to_string()),
        extendee: None,
        default_value: None,
        oneof_index: None,
        json_name: None,
        options: None,
        proto3_optional: None
      }
    ],
    extension: vec![],
    nested_type: vec![],
    enum_type: vec![],
    extension_range: vec![],
    oneof_decl: vec![],
    options: None,
    reserved_range: vec![],
    reserved_name: vec![]
  };
  let message_descriptor = DescriptorProto {
    name: Some("test_message".to_string()),
    field: vec![
      FieldDescriptorProto {
        name: Some("value".to_string()),
        number: Some(1),
        label: None,
        r#type: Some(field_descriptor_proto::Type::String as i32),
        type_name: Some("string".to_string()),
        extendee: None,
        default_value: None,
        oneof_index: None,
        json_name: None,
        options: None,
        proto3_optional: None
      }
    ],
    extension: vec![],
    nested_type: vec![],
    enum_type: vec![],
    extension_range: vec![],
    oneof_decl: vec![],
    options: None,
    reserved_range: vec![],
    reserved_name: vec![]
  };
  let file_descriptor = FileDescriptorProto {
    name: Some("test_file.proto".to_string()),
    package: Some("test_package".to_string()),
    dependency: vec![],
    public_dependency: vec![],
    weak_dependency: vec![],
    message_type: vec![ string_descriptor, message_descriptor ],
    enum_type: vec![],
    service: vec![],
    extension: vec![],
    options: None,
    source_code_info: None,
    syntax: None
  };
  let service_descriptor = ServiceDescriptorProto {
    name: Some("test_service".to_string()),
    method: vec![
      MethodDescriptorProto {
        name: Some("call".to_string()),
        input_type: Some(".test_package.StringValue".to_string()),
        output_type: Some(".test_package.test_message".to_string()),
        options: None,
        client_streaming: None,
        server_streaming: None
      }
    ],
    options: None
  };

  let config = btreemap! {
      "request".to_string() => prost_types::Value { kind: Some(prost_types::value::Kind::StringValue("true".to_string())) }
    };

  let result = construct_protobuf_interaction_for_service(
    &service_descriptor, &config, "call", &DescriptorCache::new(prost_types::FileDescriptorSet { file: vec![file_descriptor.clone()] }));
  expect!(result).to(be_ok());
}

#[test_log::test]
fn construct_protobuf_interaction_for_service_stores_the_expectations_against_the_interaction() {
  let string_descriptor = DescriptorProto {
    name: Some("StringValue".to_string()),
    field: vec![
      FieldDescriptorProto {
        name: Some("value".to_string()),
        number: Some(1),
        r#type: Some(field_descriptor_proto::Type::String as i32),
        type_name: Some("string".to_string()),
        .. FieldDescriptorProto::default()
      }
    ],
    .. DescriptorProto::default()
  };
  let message_descriptor = DescriptorProto {
    name: Some("test_message".to_string()),
    field: vec![
      FieldDescriptorProto {
        name: Some("value".to_string()),
        number: Some(1),
        r#type: Some(field_descriptor_proto::Type::String as i32),
        type_name: Some("string".to_string()),
        .. FieldDescriptorProto::default()
      }
    ],
    .. DescriptorProto::default()
  };

  let service_descriptor = ServiceDescriptorProto {
    name: Some("test_service".to_string()),
    method: vec![
      MethodDescriptorProto {
        name: Some("call".to_string()),
        input_type: Some(".test_package.StringValue".to_string()),
        output_type: Some(".test_package.test_message".to_string()),
        .. MethodDescriptorProto::default()
      }
    ],
    .. ServiceDescriptorProto::default()
  };
  let file_descriptor = FileDescriptorProto {
    name: Some("test_file.proto".to_string()),
    message_type: vec![ string_descriptor, message_descriptor ],
    service: vec![service_descriptor],
    package: Some("test_package".to_string()),
    .. FileDescriptorProto::default()
  };

  let config = btreemap! {
      "request".to_string() => prost_types::Value {
        kind: Some(prost_types::value::Kind::StringValue("true".to_string()))
      },
      "response".to_string() => prost_types::Value {
        kind: Some(prost_types::value::Kind::StringValue("true".to_string()))
      }
    };

  let (result, _) = configure_protobuf_service("test_service/call", &config, &file_descriptor,
                                               &DescriptorCache::new(prost_types::FileDescriptorSet { file: vec![file_descriptor.clone()] }), "xxx")
    .unwrap();
  let interaction_config = result.unwrap().plugin_configuration.unwrap().interaction_configuration.unwrap();
  expect!(interaction_config.fields.get("expectations").unwrap()).to(be_equal_to(
    &prost_types::Value {
      kind: Some(StructValue(Struct {
        fields: config
      }))
    }
  ));
}

lazy_static! {
    static ref FILE_DESCRIPTOR: FileDescriptorProto = FileDescriptorProto {
      name: Some("area_calculator.proto".to_string()),
      package: Some("area_calculator".to_string()),
      dependency: vec![],
      public_dependency: vec![],
      weak_dependency: vec![],
      message_type: vec![
        DescriptorProto {
          name: Some("ShapeMessage".to_string()),
          field: vec![
            FieldDescriptorProto {
              name: Some("square".to_string()),
              number: Some(1),
              label: Some(Label::Optional as i32),
              r#type: Some(Type::Message as i32),
              type_name: Some(".area_calculator.Square".to_string()),
              extendee: None,
              default_value: None,
              oneof_index: Some(0),
              json_name: Some("square".to_string()),
              options: None,
              proto3_optional: None
            },
            FieldDescriptorProto {
              name: Some("rectangle".to_string()),
              number: Some(2),
              label: Some(Label::Optional as i32),
              r#type: Some(Type::Message as i32),
              type_name: Some(".area_calculator.Rectangle".to_string()),
              extendee: None,
              default_value: None,
              oneof_index: Some(0),
              json_name: Some("rectangle".to_string()),
              options: None,
              proto3_optional: None
            }
          ],
          extension: vec![],
          nested_type: vec![],
          enum_type: vec![],
          extension_range: vec![],
          oneof_decl: vec![
            OneofDescriptorProto {
              name: Some("shape".to_string()),
              options: None
            }
          ],
          options: None,
          reserved_range: vec![],
          reserved_name: vec![]
        },
        DescriptorProto {
          name: Some("Square".to_string()),
          field: vec![
            FieldDescriptorProto {
              name: Some("edge_length".to_string()),
              number: Some(1),
              label: Some(Label::Optional as i32),
              r#type: Some(Type::Float as i32),
              type_name: None,
              extendee: None,
              default_value: None,
              oneof_index: None,
              json_name: Some("edgeLength".to_string()),
              options: None,
              proto3_optional: None
            }
          ],
          extension: vec![],
          nested_type: vec![],
          enum_type: vec![],
          extension_range: vec![],
          oneof_decl: vec![],
          options: None,
          reserved_range: vec![],
          reserved_name: vec![]
        },
        DescriptorProto {
          name: Some("Rectangle".to_string()),
          field: vec![
            FieldDescriptorProto {
              name: Some("length".to_string()),
              number: Some(1),
              label: Some(Label::Optional as i32),
              r#type: Some(Type::Float as i32),
              type_name: None,
              extendee: None,
              default_value: None,
              oneof_index: None,
              json_name: Some("length".to_string()),
              options: None,
              proto3_optional: None
            },
            FieldDescriptorProto {
              name: Some("width".to_string()),
              number: Some(2),
              label: Some(Label::Optional as i32),
              r#type: Some(Type::Float as i32),
              type_name: None,
              extendee: None,
              default_value: None,
              oneof_index: None,
              json_name: Some("width".to_string()),
              options: None,
              proto3_optional: None
            }
          ],
          extension: vec![],
          nested_type: vec![],
          enum_type: vec![],
          extension_range: vec![],
          oneof_decl: vec![],
          options: None,
          reserved_range: vec![],
          reserved_name: vec![]
        },
        DescriptorProto {
          name: Some("Area".to_string()),
          field: vec![
            FieldDescriptorProto {
              name: Some("id".to_string()),
              number: Some(1),
              label: Some(Label::Optional as i32),
              r#type: Some(Type::String as i32),
              type_name: None,
              extendee: None,
              default_value: None,
              oneof_index: None,
              json_name: Some("id".to_string()),
              options: None,
              proto3_optional: None
            },
            FieldDescriptorProto {
              name: Some("shape".to_string()),
              number: Some(2),
              label: Some(Label::Optional as i32),
              r#type: Some(Type::String as i32),
              type_name: None,
              extendee: None,
              default_value: None,
              oneof_index: None,
              json_name: Some("shape".to_string()),
              options: None,
              proto3_optional: None
            },
            FieldDescriptorProto {
              name: Some("value".to_string()),
              number: Some(3),
              label: Some(Label::Optional as i32),
              r#type: Some(Type::Float as i32),
              type_name: None,
              extendee: None,
              default_value: None,
              oneof_index: None,
              json_name: Some("value".to_string()),
              options: None,
              proto3_optional: None
            }
          ],
          extension: vec![],
          nested_type: vec![],
          enum_type: vec![],
          extension_range: vec![],
          oneof_decl: vec![],
          options: None,
          reserved_range: vec![],
          reserved_name: vec![]
        },
        DescriptorProto {
          name: Some("AreaResponse".to_string()),
          field: vec![
            FieldDescriptorProto {
              name: Some("value".to_string()),
              number: Some(1),
              label: Some(Label::Optional as i32),
              r#type: Some(Type::Message as i32),
              type_name: Some(".area_calculator.Area".to_string()),
              extendee: None,
              default_value: None,
              oneof_index: None,
              json_name: Some("value".to_string()),
              options: None,
              proto3_optional: None
            }
          ],
          extension: vec![],
          nested_type: vec![],
          enum_type: vec![],
          extension_range: vec![],
          oneof_decl: vec![],
          options: None,
          reserved_range: vec![],
          reserved_name: vec![]
        }
      ],
      enum_type: vec![],
      service: vec![
        ServiceDescriptorProto {
          name: Some("Calculator".to_string()),
          method: vec![
            MethodDescriptorProto {
              name: Some("calculateOne".to_string()),
              input_type: Some(".area_calculator.ShapeMessage".to_string()),
              output_type: Some(".area_calculator.AreaResponse".to_string()),
              options: Some(MethodOptions {
                deprecated: None,
                idempotency_level: None,
                uninterpreted_option: vec![]
              }),
              client_streaming: None,
              server_streaming: None
            },
            MethodDescriptorProto {
              name: Some("calculateMulti".to_string()),
              input_type: Some(".area_calculator.AreaRequest".to_string()),
              output_type: Some(".area_calculator.AreaResponse".to_string()),
              options: Some(MethodOptions {
                deprecated: None,
                idempotency_level: None,
                uninterpreted_option: vec![]
              }),
              client_streaming: None,
              server_streaming: None
            }
          ],
          options: None
        }
      ],
      extension: vec![],
      options: None,
      source_code_info: None,
      syntax: Some("proto3".to_string())
    };
  }

#[test_log::test]
fn build_embedded_message_field_value_with_repeated_field_configured_from_map_with_eachvalue_test() {
  let message_descriptor = DescriptorProto {
    name: Some("AreaResponse".to_string()),
    field: vec![
      FieldDescriptorProto {
        name: Some("value".to_string()),
        number: Some(1),
        label: Some(Label::Repeated as i32),
        r#type: Some(Type::Message as i32),
        type_name: Some(".area_calculator.Area".to_string()),
        extendee: None,
        default_value: None,
        oneof_index: None,
        json_name: None,
        options: None,
        proto3_optional: None
      }
    ],
    extension: vec![],
    nested_type: vec![],
    enum_type: vec![],
    extension_range: vec![],
    oneof_decl: vec![],
    options: None,
    reserved_range: vec![],
    reserved_name: vec![]
  };

  let mut message_builder = MessageBuilder::new(&message_descriptor, "AreaResponse", &FILE_DESCRIPTOR);
  let path = DocPath::new("$.value").unwrap();
  let field_descriptor = FieldDescriptorProto {
    name: Some("value".to_string()),
    number: Some(1),
    label: Some(Label::Repeated as i32),
    r#type: Some(Type::Message as i32),
    type_name: Some(".area_calculator.Area".to_string()),
    extendee: None,
    default_value: None,
    oneof_index: None,
    json_name: Some("value".to_string()),
    options: None,
    proto3_optional: None
  };
  let mut matching_rules = MatchingRuleCategory::empty("body");
  let mut generators = hashmap!{};
  let config = json!({
      "area": {
        "id": "matching(regex, '\\d+', '1234')",
        "shape": "matching(type, 'rectangle')",
        "value": "matching(number, 12)"
      },
      "pact:match": "eachValue(matching($'area'))"
    });

  let descriptor_cache = DescriptorCache::new(prost_types::FileDescriptorSet { file: vec![FILE_DESCRIPTOR.clone()] });

  let result = build_embedded_message_field_value(&mut message_builder, &path, &field_descriptor,
                                                  "value", &config, &mut matching_rules, &mut generators, &descriptor_cache
  );

  let expected_rules = matchingrules! {
       "body" => {
        "$.value" => [ pact_models::matchingrules::MatchingRule::Values ],
        "$.value.*" => [ pact_models::matchingrules::MatchingRule::Type ],
        "$.value.*.id" => [ pact_models::matchingrules::MatchingRule::Regex("\\d+".to_string()) ],
        "$.value.*.shape" => [ pact_models::matchingrules::MatchingRule::Type ],
        "$.value.*.value" => [ pact_models::matchingrules::MatchingRule::Number ]
      }
    }.rules_for_category("body").unwrap();
  expect!(result).to(be_ok());
  expect!(matching_rules).to(be_equal_to(expected_rules));
}

#[test_log::test]
fn build_embedded_message_field_value_with_repeated_field_configured_from_map_test() {
  let message_descriptor = DescriptorProto {
    name: Some("AreaResponse".to_string()),
    field: vec![
      FieldDescriptorProto {
        name: Some("value".to_string()),
        number: Some(1),
        label: Some(Label::Repeated as i32),
        r#type: Some(Type::Message as i32),
        type_name: Some(".area_calculator.Area".to_string()),
        extendee: None,
        default_value: None,
        oneof_index: None,
        json_name: None,
        options: None,
        proto3_optional: None
      }
    ],
    extension: vec![],
    nested_type: vec![],
    enum_type: vec![],
    extension_range: vec![],
    oneof_decl: vec![],
    options: None,
    reserved_range: vec![],
    reserved_name: vec![]
  };

  let mut message_builder = MessageBuilder::new(&message_descriptor, "AreaResponse", &FILE_DESCRIPTOR);
  let path = DocPath::new("$.value").unwrap();
  let field_descriptor = FieldDescriptorProto {
    name: Some("value".to_string()),
    number: Some(1),
    label: Some(Label::Repeated as i32),
    r#type: Some(Type::Message as i32),
    type_name: Some(".area_calculator.Area".to_string()),
    extendee: None,
    default_value: None,
    oneof_index: None,
    json_name: Some("value".to_string()),
    options: None,
    proto3_optional: None
  };
  let mut matching_rules = MatchingRuleCategory::empty("body");
  let mut generators = hashmap!{};
  let config = json!({
      "id": "matching(regex, '\\d+', '1234')",
      "shape": "matching(type, 'rectangle')",
      "value": "matching(number, 12)"
    });
  let descriptor_cache = DescriptorCache::new(prost_types::FileDescriptorSet { file: vec![FILE_DESCRIPTOR.clone()] });
  let result = build_embedded_message_field_value(&mut message_builder, &path, &field_descriptor,
                                                  "value", &config, &mut matching_rules, &mut generators, &descriptor_cache
  );

  let expected_rules = matchingrules! {
       "body" => {
        "$.value.*.id" => [ pact_models::matchingrules::MatchingRule::Regex("\\d+".to_string()) ],
        "$.value.*.shape" => [ pact_models::matchingrules::MatchingRule::Type ],
        "$.value.*.value" => [ pact_models::matchingrules::MatchingRule::Number ]
      }
    }.rules_for_category("body").unwrap();
  expect!(result).to(be_ok());
  expect!(matching_rules).to(be_equal_to(expected_rules));
}

pub const DESCRIPTOR_BYTES: &str = "CrYHCgxjb21tb24ucHJvdG8SD2FyZWFfY2FsY3VsYXRvciL9AwoPTGlzdGVuZXJDb\
    250ZXh0EiMKC2xpc3RlbmVyX2lkGAEgASgDQgIwAVIKbGlzdGVuZXJJZBIaCgh1c2VybmFtZRgCIAEoCVIIdXNlcm5hbW\
    USMgoVbGlzdGVuZXJfZGF0ZV9jcmVhdGVkGAMgASgDUhNsaXN0ZW5lckRhdGVDcmVhdGVkEjYKF2ZpbHRlcl9leHBsaWN\
    pdF9jb250ZW50GAQgASgIUhVmaWx0ZXJFeHBsaWNpdENvbnRlbnQSGQoIemlwX2NvZGUYBSABKAlSB3ppcENvZGUSIQoMY\
    291bnRyeV9jb2RlGAYgASgJUgtjb3VudHJ5Q29kZRIdCgpiaXJ0aF95ZWFyGAcgASgFUgliaXJ0aFllYXISFgoGZ2VuZGV\
    yGAggASgJUgZnZW5kZXISJwoPbGFzdF9leHBpcmF0aW9uGAkgASgDUg5sYXN0RXhwaXJhdGlvbhIuChNzcG9uc29yZWRfY\
    29tcF9uYW1lGAogASgJUhFzcG9uc29yZWRDb21wTmFtZRIdCgp1c2VkX3RyaWFsGAsgASgIUgl1c2VkVHJpYWwSKQoRdXN\
    lZF9pbl9hcHBfdHJpYWwYDCABKAhSDnVzZWRJbkFwcFRyaWFsEiUKDmxpc3RlbmVyX3N0YXRlGA0gASgJUg1saXN0ZW5lc\
    lN0YXRlIswBCg5TdGF0aW9uQ29udGV4dBI1ChdzdGF0aW9uX3NlZWRfcGFuZG9yYV9pZBgBIAEoCVIUc3RhdGlvblNlZWR\
    QYW5kb3JhSWQSLAoSc3RhdGlvbl9wYW5kb3JhX2lkGAIgASgJUhBzdGF0aW9uUGFuZG9yYUlkEiEKDHN0YXRpb25fdHlwZ\
    RgDIAEoCVILc3RhdGlvblR5cGUSMgoVaXNfYWR2ZXJ0aXNlcl9zdGF0aW9uGAQgASgIUhNpc0FkdmVydGlzZXJTdGF0aW9\
    uImsKBlN0YXR1cxI8CgtzdGF0dXNfY29kZRgBIAEoDjIbLmFyZWFfY2FsY3VsYXRvci5TdGF0dXNDb2RlUgpzdGF0dXNDb\
    2RlEiMKDWVycm9yX21lc3NhZ2UYAiABKAlSDGVycm9yTWVzc2FnZSo0CgpTdGF0dXNDb2RlEgYKAk9LEAASEwoPSU5WQUx\
    JRF9SRVFVRVNUEAESCQoFRVJST1IQAkIbUAFaF2lvLnBhY3QvYXJlYV9jYWxjdWxhdG9yYgZwcm90bzMK9QwKFWFyZWFfY\
    2FsY3VsYXRvci5wcm90bxIPYXJlYV9jYWxjdWxhdG9yGgxjb21tb24ucHJvdG8i0gMKDFNoYXBlTWVzc2FnZRIxCgZzcXV\
    hcmUYASABKAsyFy5hcmVhX2NhbGN1bGF0b3IuU3F1YXJlSABSBnNxdWFyZRI6CglyZWN0YW5nbGUYAiABKAsyGi5hcmVhX\
    2NhbGN1bGF0b3IuUmVjdGFuZ2xlSABSCXJlY3RhbmdsZRIxCgZjaXJjbGUYAyABKAsyFy5hcmVhX2NhbGN1bGF0b3IuQ2l\
    yY2xlSABSBmNpcmNsZRI3Cgh0cmlhbmdsZRgEIAEoCzIZLmFyZWFfY2FsY3VsYXRvci5UcmlhbmdsZUgAUgh0cmlhbmdsZ\
    RJGCg1wYXJhbGxlbG9ncmFtGAUgASgLMh4uYXJlYV9jYWxjdWxhdG9yLlBhcmFsbGVsb2dyYW1IAFINcGFyYWxsZWxvZ3J\
    hbRJHCg5kZXZpY2VfY29udGV4dBgGIAEoCzIeLmFyZWFfY2FsY3VsYXRvci5EZXZpY2VDb250ZXh0SABSDWRldmljZUNvb\
    nRleHQSTQoQbGlzdGVuZXJfY29udGV4dBgHIAEoCzIgLmFyZWFfY2FsY3VsYXRvci5MaXN0ZW5lckNvbnRleHRIAFIPbGl\
    zdGVuZXJDb250ZXh0QgcKBXNoYXBlIoIECg1EZXZpY2VDb250ZXh0EhsKCXZlbmRvcl9pZBgBIAEoBVIIdmVuZG9ySWQSH\
    woJZGV2aWNlX2lkGAIgASgDQgIwAVIIZGV2aWNlSWQSIQoMY2Fycmllcl9uYW1lGAMgASgJUgtjYXJyaWVyTmFtZRIdCgp\
    1c2VyX2FnZW50GAQgASgJUgl1c2VyQWdlbnQSIQoMbmV0d29ya190eXBlGAUgASgJUgtuZXR3b3JrVHlwZRIlCg5zeXN0Z\
    W1fdmVyc2lvbhgGIAEoCVINc3lzdGVtVmVyc2lvbhIfCgthcHBfdmVyc2lvbhgHIAEoCVIKYXBwVmVyc2lvbhIfCgt2ZW5\
    kb3JfbmFtZRgIIAEoCVIKdmVuZG9yTmFtZRIhCgxhY2Nlc3NvcnlfaWQYCSABKAlSC2FjY2Vzc29yeUlkEicKD2RldmljZ\
    V9jYXRlZ29yeRgKIAEoCVIOZGV2aWNlQ2F0ZWdvcnkSHwoLZGV2aWNlX3R5cGUYCyABKAlSCmRldmljZVR5cGUSKQoQcmV\
    wb3J0aW5nX3ZlbmRvchgMIAEoCVIPcmVwb3J0aW5nVmVuZG9yEiwKEmRldmljZV9hZF9jYXRlZ29yeRgNIAEoCVIQZGV2a\
    WNlQWRDYXRlZ29yeRIfCgtkZXZpY2VfY29kZRgOIAEoCVIKZGV2aWNlQ29kZSIpCgZTcXVhcmUSHwoLZWRnZV9sZW5ndGg\
    YASABKAJSCmVkZ2VMZW5ndGgiOQoJUmVjdGFuZ2xlEhYKBmxlbmd0aBgBIAEoAlIGbGVuZ3RoEhQKBXdpZHRoGAIgASgCU\
    gV3aWR0aCIgCgZDaXJjbGUSFgoGcmFkaXVzGAEgASgCUgZyYWRpdXMiTwoIVHJpYW5nbGUSFQoGZWRnZV9hGAEgASgCUgV\
    lZGdlQRIVCgZlZGdlX2IYAiABKAJSBWVkZ2VCEhUKBmVkZ2VfYxgDIAEoAlIFZWRnZUMiSAoNUGFyYWxsZWxvZ3JhbRIfC\
    gtiYXNlX2xlbmd0aBgBIAEoAlIKYmFzZUxlbmd0aBIWCgZoZWlnaHQYAiABKAJSBmhlaWdodCJECgtBcmVhUmVxdWVzdBI\
    1CgZzaGFwZXMYASADKAsyHS5hcmVhX2NhbGN1bGF0b3IuU2hhcGVNZXNzYWdlUgZzaGFwZXMiJAoMQXJlYVJlc3BvbnNlE\
    hQKBXZhbHVlGAEgAygCUgV2YWx1ZTKtAQoKQ2FsY3VsYXRvchJOCgxjYWxjdWxhdGVPbmUSHS5hcmVhX2NhbGN1bGF0b3I\
    uU2hhcGVNZXNzYWdlGh0uYXJlYV9jYWxjdWxhdG9yLkFyZWFSZXNwb25zZSIAEk8KDmNhbGN1bGF0ZU11bHRpEhwuYXJlY\
    V9jYWxjdWxhdG9yLkFyZWFSZXF1ZXN0Gh0uYXJlYV9jYWxjdWxhdG9yLkFyZWFSZXNwb25zZSIAQhxaF2lvLnBhY3QvYXJ\
    lYV9jYWxjdWxhdG9y0AIBYgZwcm90bzM=";

#[test_log::test]
fn build_embedded_message_field_value_with_field_from_different_proto_file() {
  let bytes = BASE64.decode(DESCRIPTOR_BYTES).unwrap();
  let bytes1 = Bytes::copy_from_slice(bytes.as_slice());
  let fds: FileDescriptorSet = FileDescriptorSet::decode(bytes1).unwrap();

  let main_descriptor = fds.file.iter()
    .find(|fd| fd.name.clone().unwrap_or_default() == "area_calculator.proto")
    .unwrap();
  let message_descriptor = main_descriptor.message_type.iter()
    .find(|md| md.name.clone().unwrap_or_default() == "ShapeMessage").unwrap();
  let mut message_builder = MessageBuilder::new(&message_descriptor, "ShapeMessage", main_descriptor);
  let path = DocPath::new("$.listener_context").unwrap();
  let field_descriptor = message_descriptor.field.iter()
    .find(|fd| fd.name.clone().unwrap_or_default() == "listener_context")
    .unwrap();
  let field_config = json!({
      "listener_id": "matching(number, 4)"
    });
  let mut matching_rules = MatchingRuleCategory::empty("body");
  let mut generators = hashmap!{};
  let descriptor_cache = DescriptorCache::new(fds.clone());

  let result = build_embedded_message_field_value(&mut message_builder, &path, field_descriptor,
                                                  "listener_context", &field_config, &mut matching_rules, &mut generators, &descriptor_cache
  );
  expect!(result).to(be_ok());
}

#[test_log::test]
fn build_single_embedded_field_value_with_field_from_different_proto_file() {
  let bytes = BASE64.decode(DESCRIPTOR_BYTES).unwrap();
  let bytes1 = Bytes::copy_from_slice(bytes.as_slice());
  let fds: FileDescriptorSet = FileDescriptorSet::decode(bytes1).unwrap();

  let main_descriptor = fds.file.iter()
    .find(|fd| fd.name.clone().unwrap_or_default() == "area_calculator.proto")
    .unwrap();
  let message_descriptor = main_descriptor.message_type.iter()
    .find(|md| md.name.clone().unwrap_or_default() == "ShapeMessage").unwrap();
  let mut message_builder = MessageBuilder::new(&message_descriptor, "ShapeMessage", main_descriptor);
  let path = DocPath::new("$.listener_context").unwrap();
  let field_descriptor = message_descriptor.field.iter()
    .find(|fd| fd.name.clone().unwrap_or_default() == "listener_context")
    .unwrap();
  let field_config = json!({
      "listener_id": "matching(number, 4)"
    });
  let mut matching_rules = MatchingRuleCategory::empty("body");
  let mut generators = hashmap!{};
  let descriptor_cache = DescriptorCache::new(fds.clone());

  let result = build_single_embedded_field_value(
    &path, &mut message_builder, MessageFieldValueType::Normal, field_descriptor,
    "listener_context", &field_config, &mut matching_rules, &mut generators, &descriptor_cache);
  expect!(result).to(be_ok());
}

pub(crate) const DESCRIPTOR_WITH_ENUM_BYTES: [u8; 1128] = [
  10, 229, 8, 10, 21, 97, 114, 101, 97, 95, 99, 97, 108, 99, 117, 108, 97, 116, 111, 114, 46,
  112, 114, 111, 116, 111, 18, 15, 97, 114, 101, 97, 95, 99, 97, 108, 99, 117, 108, 97, 116,
  111, 114, 34, 186, 2, 10, 12, 83, 104, 97, 112, 101, 77, 101, 115, 115, 97, 103, 101, 18,
  49, 10, 6, 115, 113, 117, 97, 114, 101, 24, 1, 32, 1, 40, 11, 50, 23, 46, 97, 114, 101, 97,
  95, 99, 97, 108, 99, 117, 108, 97, 116, 111, 114, 46, 83, 113, 117, 97, 114, 101, 72, 0, 82,
  6, 115, 113, 117, 97, 114, 101, 18, 58, 10, 9, 114, 101, 99, 116, 97, 110, 103, 108, 101, 24,
  2, 32, 1, 40, 11, 50, 26, 46, 97, 114, 101, 97, 95, 99, 97, 108, 99, 117, 108, 97, 116, 111,
  114, 46, 82, 101, 99, 116, 97, 110, 103, 108, 101, 72, 0, 82, 9, 114, 101, 99, 116, 97, 110,
  103, 108, 101, 18, 49, 10, 6, 99, 105, 114, 99, 108, 101, 24, 3, 32, 1, 40, 11, 50, 23, 46,
  97, 114, 101, 97, 95, 99, 97, 108, 99, 117, 108, 97, 116, 111, 114, 46, 67, 105, 114, 99,
  108, 101, 72, 0, 82, 6, 99, 105, 114, 99, 108, 101, 18, 55, 10, 8, 116, 114, 105, 97, 110,
  103, 108, 101, 24, 4, 32, 1, 40, 11, 50, 25, 46, 97, 114, 101, 97, 95, 99, 97, 108, 99, 117,
  108, 97, 116, 111, 114, 46, 84, 114, 105, 97, 110, 103, 108, 101, 72, 0, 82, 8, 116, 114,
  105, 97, 110, 103, 108, 101, 18, 70, 10, 13, 112, 97, 114, 97, 108, 108, 101, 108, 111, 103,
  114, 97, 109, 24, 5, 32, 1, 40, 11, 50, 30, 46, 97, 114, 101, 97, 95, 99, 97, 108, 99, 117,
  108, 97, 116, 111, 114, 46, 80, 97, 114, 97, 108, 108, 101, 108, 111, 103, 114, 97, 109, 72,
  0, 82, 13, 112, 97, 114, 97, 108, 108, 101, 108, 111, 103, 114, 97, 109, 66, 7, 10, 5, 115,
  104, 97, 112, 101, 34, 41, 10, 6, 83, 113, 117, 97, 114, 101, 18, 31, 10, 11, 101, 100, 103,
  101, 95, 108, 101, 110, 103, 116, 104, 24, 1, 32, 1, 40, 2, 82, 10, 101, 100, 103, 101, 76,
  101, 110, 103, 116, 104, 34, 125, 10, 9, 82, 101, 99, 116, 97, 110, 103, 108, 101, 18, 22,
  10, 6, 108, 101, 110, 103, 116, 104, 24, 1, 32, 1, 40, 2, 82, 6, 108, 101, 110, 103, 116,
  104, 18, 20, 10, 5, 119, 105, 100, 116, 104, 24, 2, 32, 1, 40, 2, 82, 5, 119, 105, 100, 116,
  104, 18, 66, 10, 13, 97, 100, 95, 98, 114, 101, 97, 107, 95, 116, 121, 112, 101, 24, 5, 32,
  1, 40, 14, 50, 30, 46, 97, 114, 101, 97, 95, 99, 97, 108, 99, 117, 108, 97, 116, 111, 114,
  46, 65, 100, 66, 114, 101, 97, 107, 65, 100, 84, 121, 112, 101, 82, 11, 97, 100, 66, 114,
  101, 97, 107, 84, 121, 112, 101, 34, 32, 10, 6, 67, 105, 114, 99, 108, 101, 18, 22, 10, 6,
  114, 97, 100, 105, 117, 115, 24, 1, 32, 1, 40, 2, 82, 6, 114, 97, 100, 105, 117, 115, 34, 79,
  10, 8, 84, 114, 105, 97, 110, 103, 108, 101, 18, 21, 10, 6, 101, 100, 103, 101, 95, 97, 24,
  1, 32, 1, 40, 2, 82, 5, 101, 100, 103, 101, 65, 18, 21, 10, 6, 101, 100, 103, 101, 95, 98,
  24, 2, 32, 1, 40, 2, 82, 5, 101, 100, 103, 101, 66, 18, 21, 10, 6, 101, 100, 103, 101, 95,
  99, 24, 3, 32, 1, 40, 2, 82, 5, 101, 100, 103, 101, 67, 34, 72, 10, 13, 80, 97, 114, 97,
  108, 108, 101, 108, 111, 103, 114, 97, 109, 18, 31, 10, 11, 98, 97, 115, 101, 95, 108, 101,
  110, 103, 116, 104, 24, 1, 32, 1, 40, 2, 82, 10, 98, 97, 115, 101, 76, 101, 110, 103, 116,
  104, 18, 22, 10, 6, 104, 101, 105, 103, 104, 116, 24, 2, 32, 1, 40, 2, 82, 6, 104, 101, 105,
  103, 104, 116, 34, 68, 10, 11, 65, 114, 101, 97, 82, 101, 113, 117, 101, 115, 116, 18, 53,
  10, 6, 115, 104, 97, 112, 101, 115, 24, 1, 32, 3, 40, 11, 50, 29, 46, 97, 114, 101, 97, 95,
  99, 97, 108, 99, 117, 108, 97, 116, 111, 114, 46, 83, 104, 97, 112, 101, 77, 101, 115, 115,
  97, 103, 101, 82, 6, 115, 104, 97, 112, 101, 115, 34, 36, 10, 12, 65, 114, 101, 97, 82, 101,
  115, 112, 111, 110, 115, 101, 18, 20, 10, 5, 118, 97, 108, 117, 101, 24, 1, 32, 3, 40, 2, 82,
  5, 118, 97, 108, 117, 101, 42, 85, 10, 13, 65, 100, 66, 114, 101, 97, 107, 65, 100, 84, 121,
  112, 101, 18, 28, 10, 24, 77, 73, 83, 83, 73, 78, 71, 95, 65, 68, 95, 66, 82, 69, 65, 75, 95,
  65, 68, 95, 84, 89, 80, 69, 16, 0, 18, 18, 10, 14, 65, 85, 68, 73, 79, 95, 65, 68, 95, 66, 82,
  69, 65, 75, 16, 1, 18, 18, 10, 14, 86, 73, 68, 69, 79, 95, 65, 68, 95, 66, 82, 69, 65, 75, 16,
  2, 50, 173, 1, 10, 10, 67, 97, 108, 99, 117, 108, 97, 116, 111, 114, 18, 78, 10, 12, 99, 97,
  108, 99, 117, 108, 97, 116, 101, 79, 110, 101, 18, 29, 46, 97, 114, 101, 97, 95, 99, 97, 108,
  99, 117, 108, 97, 116, 111, 114, 46, 83, 104, 97, 112, 101, 77, 101, 115, 115, 97, 103, 101,
  26, 29, 46, 97, 114, 101, 97, 95, 99, 97, 108, 99, 117, 108, 97, 116, 111, 114, 46, 65, 114,
  101, 97, 82, 101, 115, 112, 111, 110, 115, 101, 34, 0, 18, 79, 10, 14, 99, 97, 108, 99, 117,
  108, 97, 116, 101, 77, 117, 108, 116, 105, 18, 28, 46, 97, 114, 101, 97, 95, 99, 97, 108, 99,
  117, 108, 97, 116, 111, 114, 46, 65, 114, 101, 97, 82, 101, 113, 117, 101, 115, 116, 26, 29,
  46, 97, 114, 101, 97, 95, 99, 97, 108, 99, 117, 108, 97, 116, 111, 114, 46, 65, 114, 101, 97,
  82, 101, 115, 112, 111, 110, 115, 101, 34, 0, 66, 28, 90, 23, 105, 111, 46, 112, 97, 99, 116,
  47, 97, 114, 101, 97, 95, 99, 97, 108, 99, 117, 108, 97, 116, 111, 114, 208, 2, 1, 98, 6, 112,
  114, 111, 116, 111, 51];

#[test_log::test]
fn construct_message_field_with_global_enum_test() {
  let bytes: &[u8] = &DESCRIPTOR_WITH_ENUM_BYTES;
  let buffer = Bytes::from(bytes);
  let fds: FileDescriptorSet = FileDescriptorSet::decode(buffer).unwrap();

  let main_descriptor = fds.file.iter()
    .find(|fd| fd.name.clone().unwrap_or_default() == "area_calculator.proto")
    .unwrap();
  let message_descriptor = main_descriptor.message_type.iter()
    .find(|md| md.name.clone().unwrap_or_default() == "Rectangle").unwrap();
  let mut message_builder = MessageBuilder::new(&message_descriptor, "Rectangle", main_descriptor);
  let path = DocPath::new("$.rectangle.ad_break_type").unwrap();
  let mut matching_rules = MatchingRuleCategory::empty("body");
  let mut generators = hashmap!{};
  let descriptor_cache = DescriptorCache::new(fds.clone());

  let result = construct_message_field(&mut message_builder, &mut matching_rules,
                                       &mut generators, "ad_break_type", &Value::String("AUDIO_AD_BREAK".to_string()),
                                       &path, &descriptor_cache);
  expect!(result).to(be_ok());

  let field = message_builder.fields.get("ad_break_type");
  expect!(field).to(be_some());
}

pub(crate) const DESCRIPTOR_WITH_EMBEDDED_MESSAGE: [u8; 644] = [
  10, 129, 5, 10, 21, 97, 114, 101, 97, 95, 99, 97, 108, 99, 117, 108, 97, 116, 111, 114, 46,
  112, 114, 111, 116, 111, 18, 15, 97, 114, 101, 97, 95, 99, 97, 108, 99, 117, 108, 97, 116, 111,
  114, 34, 211, 2, 10, 14, 65, 100, 66, 114, 101, 97, 107, 82, 101, 113, 117, 101, 115, 116, 18,
  88, 10, 16, 97, 100, 95, 98, 114, 101, 97, 107, 95, 99, 111, 110, 116, 101, 120, 116, 24, 1,
  32, 3, 40, 11, 50, 46, 46, 97, 114, 101, 97, 95, 99, 97, 108, 99, 117, 108, 97, 116, 111, 114,
  46, 65, 100, 66, 114, 101, 97, 107, 82, 101, 113, 117, 101, 115, 116, 46, 65, 100, 66, 114,
  101, 97, 107, 67, 111, 110, 116, 101, 120, 116, 82, 14, 97, 100, 66, 114, 101, 97, 107, 67,
  111, 110, 116, 101, 120, 116, 26, 230, 1, 10, 14, 65, 100, 66, 114, 101, 97, 107, 67, 111,
  110, 116, 101, 120, 116, 18, 36, 10, 14, 102, 111, 114, 99, 101, 100, 95, 108, 105, 110, 101,
  95, 105, 100, 24, 1, 32, 1, 40, 9, 82, 12, 102, 111, 114, 99, 101, 100, 76, 105, 110, 101, 73,
  100, 18, 44, 10, 18, 102, 111, 114, 99, 101, 100, 95, 99, 114, 101, 97, 116, 105, 118, 101, 95,
  105, 100, 24, 2, 32, 1, 40, 9, 82, 16, 102, 111, 114, 99, 101, 100, 67, 114, 101, 97, 116, 105,
  118, 101, 73, 100, 18, 30, 10, 11, 97, 100, 95, 98, 114, 101, 97, 107, 95, 105, 100, 24, 3, 32,
  1, 40, 9, 82, 9, 97, 100, 66, 114, 101, 97, 107, 73, 100, 18, 28, 10, 9, 115, 101, 115, 115,
  105, 111, 110, 73, 100, 24, 4, 32, 1, 40, 9, 82, 9, 115, 101, 115, 115, 105, 111, 110, 73, 100,
  18, 66, 10, 13, 97, 100, 95, 98, 114, 101, 97, 107, 95, 116, 121, 112, 101, 24, 5, 32, 1, 40,
  14, 50, 30, 46, 97, 114, 101, 97, 95, 99, 97, 108, 99, 117, 108, 97, 116, 111, 114, 46, 65,
  100, 66, 114, 101, 97, 107, 65, 100, 84, 121, 112, 101, 82, 11, 97, 100, 66, 114, 101, 97, 107,
  84, 121, 112, 101, 34, 36, 10, 12, 65, 114, 101, 97, 82, 101, 115, 112, 111, 110, 115, 101, 18,
  20, 10, 5, 118, 97, 108, 117, 101, 24, 1, 32, 3, 40, 2, 82, 5, 118, 97, 108, 117, 101, 42, 85,
  10, 13, 65, 100, 66, 114, 101, 97, 107, 65, 100, 84, 121, 112, 101, 18, 28, 10, 24, 77, 73, 83,
  83, 73, 78, 71, 95, 65, 68, 95, 66, 82, 69, 65, 75, 95, 65, 68, 95, 84, 89, 80, 69, 16, 0, 18,
  18, 10, 14, 65, 85, 68, 73, 79, 95, 65, 68, 95, 66, 82, 69, 65, 75, 16, 1, 18, 18, 10, 14, 86,
  73, 68, 69, 79, 95, 65, 68, 95, 66, 82, 69, 65, 75, 16, 2, 50, 94, 10, 10, 67, 97, 108, 99,
  117, 108, 97, 116, 111, 114, 18, 80, 10, 12, 99, 97, 108, 99, 117, 108, 97, 116, 101, 79, 110,
  101, 18, 31, 46, 97, 114, 101, 97, 95, 99, 97, 108, 99, 117, 108, 97, 116, 111, 114, 46, 65,
  100, 66, 114, 101, 97, 107, 82, 101, 113, 117, 101, 115, 116, 26, 29, 46, 97, 114, 101, 97, 95,
  99, 97, 108, 99, 117, 108, 97, 116, 111, 114, 46, 65, 114, 101, 97, 82, 101, 115, 112, 111,
  110, 115, 101, 34, 0, 66, 28, 90, 23, 105, 111, 46, 112, 97, 99, 116, 47, 97, 114, 101, 97, 95,
  99, 97, 108, 99, 117, 108, 97, 116, 111, 114, 208, 2, 1, 98, 6, 112, 114, 111, 116, 111, 51
];

#[test_log::test]
fn build_single_embedded_field_value_with_embedded_message() {
  let bytes: &[u8] = &DESCRIPTOR_WITH_EMBEDDED_MESSAGE;
  let buffer = Bytes::from(bytes);
  let fds: FileDescriptorSet = FileDescriptorSet::decode(buffer).unwrap();

  let main_descriptor = fds.file.iter()
    .find(|fd| fd.name.clone().unwrap_or_default() == "area_calculator.proto")
    .unwrap();
  let message_descriptor = main_descriptor.message_type.iter()
    .find(|md| md.name.clone().unwrap_or_default() == "AdBreakRequest").unwrap();
  let mut message_builder = MessageBuilder::new(&message_descriptor, "AdBreakRequest", main_descriptor);
  let path = DocPath::new("$.ad_break_context").unwrap();
  let field_descriptor = message_descriptor.field.iter()
    .find(|fd| fd.name.clone().unwrap_or_default() == "ad_break_context")
    .unwrap();
  let field_config = json!({
      "ad_break_type": "AUDIO_AD_BREAK"
    });
  let mut matching_rules = MatchingRuleCategory::empty("body");
  let mut generators = hashmap!{};
  let descriptor_cache = DescriptorCache::new(fds.clone());

  let result = build_single_embedded_field_value(
    &path, &mut message_builder, MessageFieldValueType::Normal, field_descriptor,
    "ad_break_type", &field_config, &mut matching_rules, &mut generators, &descriptor_cache);
  expect!(result).to(be_ok());
}

#[test]
fn configuring_request_part_returns_the_config_as_is_if_the_service_part_is_for_the_request() {
  let config = btreemap!{
      "A".to_string() => prost_types::Value { kind: Some(NullValue(0)) }
    };
  let result = request_part(&config, "request").unwrap();
  expect!(result).to(be_equal_to(config));
}

#[test]
fn configuring_request_part_returns_empty_map_if_there_is_no_request_element() {
  let config = btreemap!{};
  let result = request_part(&config, "").unwrap();
  expect!(result).to(be_equal_to(config));
}

#[test]
fn configuring_request_part_returns_an_error_if_request_config_is_not_the_correct_form() {
  let config = btreemap!{
      "request".to_string() => prost_types::Value { kind: Some(NumberValue(0.0)) }
    };
  let result = request_part(&config, "");
  expect!(result).to(be_err());
}

#[test]
fn configuring_request_part_returns_any_struct_from_the_request_attribute() {
  let request_config = btreemap!{
      "A".to_string() => prost_types::Value { kind: Some(StringValue("B".to_string())) }
    };
  let config = btreemap!{
      "request".to_string() => prost_types::Value { kind: Some(StructValue(Struct {
          fields: request_config.clone()
        }))
      }
    };
  let result = request_part(&config, "").unwrap();
  expect!(result).to(be_equal_to(request_config));
}

#[test]
fn configuring_request_part_returns_a_struct_with_a_value_attribute_if_the_request_attribute_is_a_string() {
  let request_config = btreemap!{
      "value".to_string() => prost_types::Value { kind: Some(StringValue("B".to_string())) }
    };
  let config = btreemap!{
      "request".to_string() => prost_types::Value { kind: Some(StringValue("B".to_string())) }
    };
  let result = request_part(&config, "").unwrap();
  expect!(result).to(be_equal_to(request_config));
}

#[test]
fn configuring_response_part_returns_the_config_as_is_if_the_service_part_is_for_the_response() {
  let config = btreemap!{
      "A".to_string() => prost_types::Value { kind: Some(NullValue(0)) }
    };
  let result = response_part(&config, "response").unwrap();
  expect!(result).to(be_equal_to(vec![(config.clone(), None)]));
}

#[test]
fn configuring_response_part_returns_empty_map_if_there_is_no_response_elements() {
  let config = btreemap!{};
  let result = response_part(&config, "").unwrap();
  expect!(result).to(be_equal_to(vec![]));
}

#[test]
fn configuring_response_part_ignores_any_config_that_is_not_the_correct_form() {
  let config = btreemap!{
      "response".to_string() => prost_types::Value { kind: Some(NumberValue(0.0)) }
    };
  let result = response_part(&config, "").unwrap();
  expect!(result).to(be_equal_to(vec![]));
}

#[test]
fn configuring_response_part_returns_any_struct_from_the_response_attribute() {
  let response_config = btreemap!{
      "A".to_string() => prost_types::Value { kind: Some(StringValue("B".to_string())) }
    };
  let config = btreemap!{
      "response".to_string() => prost_types::Value { kind: Some(StructValue(Struct {
          fields: response_config.clone()
        }))
      }
    };
  let result = response_part(&config, "").unwrap();
  expect!(result).to(be_equal_to(vec![(response_config, None)]));
}

#[test]
fn configuring_response_part_returns_a_struct_with_a_value_attribute_if_the_response_attribute_is_a_string() {
  let response_config = btreemap!{
      "value".to_string() => prost_types::Value { kind: Some(StringValue("B".to_string())) }
    };
  let config = btreemap!{
      "response".to_string() => prost_types::Value { kind: Some(StringValue("B".to_string())) }
    };
  let result = response_part(&config, "").unwrap();
  expect!(result).to(be_equal_to(vec![(response_config, None)]));
}

#[test]
fn configuring_response_part_configures_each_item_if_the_response_attribute_is_a_list() {
  let response_config = btreemap!{
      "A".to_string() => prost_types::Value { kind: Some(StringValue("B".to_string())) }
    };
  let response_config2 = btreemap!{
      "C".to_string() => prost_types::Value { kind: Some(StringValue("D".to_string())) }
    };
  let config = btreemap!{
      "response".to_string() => prost_types::Value {
        kind: Some(ListValue(prost_types::ListValue {
          values: vec![
            prost_types::Value { kind: Some(StructValue(Struct {
                fields: response_config.clone()
              }))
            },
            prost_types::Value { kind: Some(StructValue(Struct {
                fields: response_config2.clone()
              }))
            }
          ]
        }))
      }
    };
  let result = response_part(&config, "").unwrap();
  expect!(result).to(be_equal_to(vec![
    (response_config, None),
    (response_config2, None)
  ]));
}

#[test]
fn configuring_response_part_returns_also_returns_metadata_from_the_response_metadata_attribute() {
  let response_config = btreemap!{
      "A".to_string() => prost_types::Value { kind: Some(StringValue("B".to_string())) }
    };
  let response_metadata_config = btreemap!{
      "C".to_string() => prost_types::Value { kind: Some(StringValue("D".to_string())) }
    };
  let config = btreemap!{
      "response".to_string() => prost_types::Value { kind: Some(StructValue(Struct {
          fields: response_config.clone()
        }))
      },
      "responseMetadata".to_string() => prost_types::Value { kind: Some(StructValue(Struct {
          fields: response_metadata_config.clone()
        }))
      }
    };
  let result = response_part(&config, "").unwrap();
  let expected_metadata = prost_types::Value {
    kind: Some(StructValue(Struct {
      fields: response_metadata_config.clone()
    }))
  };
  expect!(result).to(be_equal_to(vec![(response_config, Some(&expected_metadata))]));
}

#[test]
fn configuring_response_part_deals_with_the_case_where_there_is_only_metadata() {
  let response_metadata_config = btreemap!{
      "C".to_string() => prost_types::Value { kind: Some(StringValue("D".to_string())) }
    };
  let config = btreemap!{
      "responseMetadata".to_string() => prost_types::Value { kind: Some(StructValue(Struct {
          fields: response_metadata_config.clone()
        }))
      }
    };
  let result = response_part(&config, "").unwrap();
  let expected_metadata = prost_types::Value {
    kind: Some(StructValue(Struct {
      fields: response_metadata_config.clone()
    }))
  };
  expect!(result).to(be_equal_to(vec![(btreemap!{}, Some(&expected_metadata))]));
}

#[test]
fn path_parent() {
  let something = DocPath::root().join("something");
  let something_else = something.join("else");
  let something_star = something.join("*");
  let something_escaped = something.join("e l s e");
  let something_escaped2 = something_escaped.join("two");
  let something_star_child = something_star.join("child");

  expect!(super::parent(&something)).to(be_some().value(DocPath::root()));
  expect!(super::parent(&something_else)).to(be_some().value(something.clone()));
  expect!(super::parent(&something_star)).to(be_some().value(something.clone()));
  expect!(super::parent(&something_escaped)).to(be_some().value(something.clone()));
  expect!(super::parent(&something_escaped2)).to(be_some().value(something_escaped.clone()));
  expect!(super::parent(&something_star_child)).to(be_some().value(something_star.clone()));

  expect!(super::parent(&DocPath::root())).to(be_none());
  expect!(super::parent(&DocPath::empty())).to(be_none());
}

const DESCRIPTORS_MAP_WITH_PRIMITIVE_FIELDS: [u8; 334] = [
  10, 203, 2, 10, 10, 109, 97, 112, 115, 46, 112, 114, 111, 116, 111, 18, 4, 109, 97, 112, 115,
  34, 169, 1, 10, 13, 65, 99, 116, 105, 111, 110, 82, 101, 113, 117, 101, 115, 116, 18, 22, 10,
  6, 97, 99, 116, 105, 111, 110, 24, 1, 32, 1, 40, 9, 82, 6, 97, 99, 116, 105, 111, 110, 18, 52,
  10, 5, 112, 97, 114, 97, 109, 24, 2, 32, 3, 40, 11, 50, 30, 46, 109, 97, 112, 115, 46, 65, 99,
  116, 105, 111, 110, 82, 101, 113, 117, 101, 115, 116, 46, 80, 97, 114, 97, 109, 69, 110, 116,
  114, 121, 82, 5, 112, 97, 114, 97, 109, 18, 16, 10, 3, 105, 100, 115, 24, 3, 32, 3, 40, 9, 82,
  3, 105, 100, 115, 26, 56, 10, 10, 80, 97, 114, 97, 109, 69, 110, 116, 114, 121, 18, 16, 10, 3,
  107, 101, 121, 24, 1, 32, 1, 40, 9, 82, 3, 107, 101, 121, 18, 20, 10, 5, 118, 97, 108, 117, 101,
  24, 2, 32, 1, 40, 9, 82, 5, 118, 97, 108, 117, 101, 58, 2, 56, 1, 34, 56, 10, 14, 65, 99, 116,
  105, 111, 110, 82, 101, 115, 112, 111, 110, 115, 101, 18, 38, 10, 14, 114, 101, 115, 112, 111,
  110, 115, 101, 83, 116, 97, 116, 117, 115, 24, 1, 32, 1, 40, 8, 82, 14, 114, 101, 115, 112, 111,
  110, 115, 101, 83, 116, 97, 116, 117, 115, 50, 73, 10, 6, 79, 77, 67, 97, 108, 99, 18, 63, 10,
  18, 104, 97, 110, 100, 108, 101, 66, 97, 116, 99, 104, 82, 101, 113, 117, 101, 115, 116, 18, 19,
  46, 109, 97, 112, 115, 46, 65, 99, 116, 105, 111, 110, 82, 101, 113, 117, 101, 115, 116, 26, 20,
  46, 109, 97, 112, 115, 46, 65, 99, 116, 105, 111, 110, 82, 101, 115, 112, 111, 110, 115, 101,
  98, 6, 112, 114, 111, 116, 111, 51
];

#[test_log::test]
fn configure_message_with_map_with_primitive_fields() {
  let bytes: &[u8] = &DESCRIPTORS_MAP_WITH_PRIMITIVE_FIELDS;
  let buffer = Bytes::from(bytes);
  let fds: FileDescriptorSet = FileDescriptorSet::decode(buffer).unwrap();
  dbg!(&fds);

  let main_descriptor = fds.file.iter()
    .find(|fd| fd.name.clone().unwrap_or_default() == "maps.proto")
    .unwrap();
  let message_descriptor = main_descriptor.message_type.iter()
    .find(|md| md.name.clone().unwrap_or_default() == "ActionRequest").unwrap();
  let mut message_builder = MessageBuilder::new(&message_descriptor, "ActionRequest", main_descriptor);
  let path = DocPath::new("$.param").unwrap();
  let mut matching_rules = MatchingRuleCategory::empty("body");
  let mut generators = hashmap!{};
  let descriptor_cache = DescriptorCache::new(fds.clone());

  let result = construct_message_field(
    &mut message_builder,
    &mut matching_rules,
    &mut generators,
    "param",
    &json!({"apply":"Skip_holiday"}),
    &path,
    &descriptor_cache
  );

  expect!(result).to(be_ok());

  let constructed = message_builder.fields.get("param").unwrap();
  expect!(constructed.proto_type).to(be_equal_to(Type::Message));
  expect!(constructed.field_type).to(be_equal_to(MessageFieldValueType::Map));
  expect!(&constructed.values).to(be_equal_to(
    &vec![
      MessageFieldValue { name: "key".to_string(), raw_value: Some("apply".to_string()), rtype: RType::String("apply".to_string()) },
      MessageFieldValue { name: "value".to_string(), raw_value: Some("Skip_holiday".to_string()), rtype: RType::String("Skip_holiday".to_string()) }
    ]
  ));
}

#[test]
fn construct_protobuf_interaction_with_provider_state_generator() {
  let file_descriptor = FileDescriptorProto {
    name: Some("test_file".to_string()),
    package: Some("test_package".to_string()),
    .. FileDescriptorProto::default()
  };
  let message_descriptor = DescriptorProto {
    name: Some("test_message".to_string()),
    field: vec![
      FieldDescriptorProto {
        name: Some("implementation".to_string()),
        number: Some(1),
        r#type: Some(field_descriptor_proto::Type::String as i32),
        type_name: Some("string".to_string()),
        .. FieldDescriptorProto::default()
      }
    ],
    .. DescriptorProto::default()
  };
  let config = btreemap! {
      "implementation".to_string() => prost_types::Value {
        kind: Some(prost_types::value::Kind::StringValue("notEmpty(fromProviderState('exp', 'plugin-driver-rust'))".to_string()))
      }
    };

  let result = construct_protobuf_interaction_for_message(&message_descriptor, &config,
                                                          "", &file_descriptor, &DescriptorCache::new(prost_types::FileDescriptorSet { file: vec![file_descriptor.clone()] }), None).unwrap();

  let body = result.contents.as_ref().unwrap();
  expect!(body.content_type.as_str()).to(be_equal_to("application/protobuf;message=.test_package.test_message"));
  expect!(body.content_type_hint).to(be_equal_to(2));
  expect!(body.content.as_ref()).to(be_some().value(&vec![
    10, // field 1 length encoded (1 << 3 + 2 == 10)
    18, // 18 bytes
    112, 108, 117, 103, 105, 110, 45, 100, 114, 105, 118, 101, 114, 45, 114, 117, 115, 116
  ]));

  expect!(result.rules).to(be_equal_to(hashmap! {
      "$.implementation".to_string() => MatchingRules {
        rule: vec![
          MatchingRule { r#type: "not-empty".to_string(), .. MatchingRule::default() }
        ]
      }
    }));

  assert_eq!(result.generators, hashmap! {
      "$.implementation".to_string() => pact_plugin_driver::proto::Generator {
        r#type: "ProviderState".to_string(),
        values: Some(Struct {
          fields: btreemap!{
            "data_type".to_string() => prost_types::Value {
              kind: Some(StringValue("STRING".to_string()))
            },
            "expression".to_string() => prost_types::Value {
              kind: Some(StringValue("exp".to_string()))
            }
          }
        })
      }
    });
}
