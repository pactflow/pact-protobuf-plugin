use bytes::Bytes;
use expectest::prelude::*;
use maplit::hashmap;
use pact_models::path_exp::DocPath;
use pact_models::prelude::*;
use prost::Message;
use prost_types::FileDescriptorSet;
use serde_json::{json, Value};

use crate::message_builder::{MessageBuilder, MessageFieldValue, MessageFieldValueType, RType};
use crate::protobuf::build_field_value;
use crate::protobuf::tests::DESCRIPTORS_FOR_EACH_VALUE_TEST;
use crate::utils::DescriptorCache;

#[test_log::test]
fn build_field_value_with_message_with_each_value_matcher() {
  let fds = FileDescriptorSet::decode(DESCRIPTORS_FOR_EACH_VALUE_TEST.as_slice()).unwrap();
  let fs = fds.file.first().unwrap();
  let descriptor_cache = DescriptorCache::new(fds.clone());
  let (message_descriptor, _) = descriptor_cache.find_message_descriptor_for_type(".ValuesMessageIn").unwrap();
  let field_descriptor = message_descriptor.field.first().unwrap();
  let mut message_builder = MessageBuilder::new(&message_descriptor, "ValuesMessageIn", fs);
  let path = DocPath::new("$.value").unwrap();
  let mut matching_rules = MatchingRuleCategory::empty("body");
  let mut generators = hashmap!{};
  let descriptor_cache = DescriptorCache::new(fds.clone());

  let result = build_field_value(
    &path, &mut message_builder, MessageFieldValueType::Repeated, field_descriptor,
    "value", &Value::String("eachValue(matching(type, '00000000000000000000000000000000'))".to_string()),
    &mut matching_rules, &mut generators, &descriptor_cache
  ).unwrap();

  expect!(result.as_ref()).to(be_some());
  let message_field_value = result.unwrap();
  expect!(message_field_value).to(be_equal_to(MessageFieldValue {
    name: "value".to_string(),
    raw_value: Some("00000000000000000000000000000000".to_string()),
    rtype: RType::String("00000000000000000000000000000000".to_string())
  }));
  let field_value = message_builder.fields.get("value").unwrap();
  expect!(field_value.values.as_ref()).to(be_equal_to(vec![
    MessageFieldValue {
      name: "value".to_string(),
      raw_value: Some("00000000000000000000000000000000".to_string()),
      rtype: RType::String("00000000000000000000000000000000".to_string())
    }
  ]));
}

const DESCRIPTORS_ROUTE_GUIDE_WITH_ENUM_BASIC: [u8; 320] = [
  10, 189, 2, 10, 15, 116, 101, 115, 116, 95, 101, 110, 117, 109, 46, 112, 114, 111, 116, 111,
  18, 13, 114, 111, 117, 116, 101, 103, 117, 105, 100, 101, 46, 118, 50, 34, 65, 10, 5, 80, 111,
  105, 110, 116, 18, 26, 10, 8, 108, 97, 116, 105, 116, 117, 100, 101, 24, 1, 32, 1, 40, 5, 82,
  8, 108, 97, 116, 105, 116, 117, 100, 101, 18, 28, 10, 9, 108, 111, 110, 103, 105, 116, 117,
  100, 101, 24, 2, 32, 1, 40, 5, 82, 9, 108, 111, 110, 103, 105, 116, 117, 100, 101, 34, 58, 10,
  7, 70, 101, 97, 116, 117, 114, 101, 18, 47, 10, 6, 114, 101, 115, 117, 108, 116, 24, 1, 32, 1,
  40, 14, 50, 23, 46, 114, 111, 117, 116, 101, 103, 117, 105, 100, 101, 46, 118, 50, 46, 84, 101,
  115, 116, 69, 110, 117, 109, 82, 6, 114, 101, 115, 117, 108, 116, 42, 56, 10, 8, 84, 101, 115,
  116, 69, 110, 117, 109, 18, 14, 10, 10, 86, 65, 76, 85, 69, 95, 90, 69, 82, 79, 16, 0, 18, 13,
  10, 9, 86, 65, 76, 85, 69, 95, 79, 78, 69, 16, 1, 18, 13, 10, 9, 86, 65, 76, 85, 69, 95, 84,
  87, 79, 16, 2, 50, 69, 10, 4, 84, 101, 115, 116, 18, 61, 10, 11, 71, 101, 116, 70, 101, 97,
  116, 117, 114, 101, 50, 18, 20, 46, 114, 111, 117, 116, 101, 103, 117, 105, 100, 101, 46, 118,
  50, 46, 80, 111, 105, 110, 116, 26, 22, 46, 114, 111, 117, 116, 101, 103, 117, 105, 100, 101,
  46, 118, 50, 46, 70, 101, 97, 116, 117, 114, 101, 34, 0, 66, 19, 90, 17, 105, 111, 46, 112,
  97, 99, 116, 47, 116, 101, 115, 116, 95, 101, 110, 117, 109, 98, 6, 112, 114, 111, 116, 111,
  51
];

#[test_log::test]
fn build_field_value_with_global_enum() {
  let bytes: &[u8] = &DESCRIPTORS_ROUTE_GUIDE_WITH_ENUM_BASIC;
  let buffer = Bytes::from(bytes);
  let fds: FileDescriptorSet = FileDescriptorSet::decode(buffer).unwrap();

  let main_descriptor = fds.file.iter()
    .find(|fd| fd.name.clone().unwrap_or_default() == "test_enum.proto")
    .unwrap();
  let message_descriptor = main_descriptor.message_type.iter()
    .find(|md| md.name.clone().unwrap_or_default() == "Feature").unwrap();
  let mut message_builder = MessageBuilder::new(&message_descriptor, "Feature", main_descriptor);
  let path = DocPath::new("$.result").unwrap();
  let field_descriptor = message_descriptor.field.iter()
    .find(|fd| fd.name.clone().unwrap_or_default() == "result")
    .unwrap();
  let field_config = json!("matching(type, 'VALUE_ONE')");
  let mut matching_rules = MatchingRuleCategory::empty("body");
  let mut generators = hashmap!{};
  let descriptor_cache = DescriptorCache::new(fds.clone());

  let result = build_field_value(&path, &mut message_builder,
    MessageFieldValueType::Normal, field_descriptor, "result", &field_config,
    &mut matching_rules, &mut generators, &descriptor_cache
  );
  expect!(result).to(be_ok());
}

#[test_log::test]
fn build_field_value_with_bytes_field() {
  // Descriptors from basic_values.proto
  let descriptor_bytes: &[u8] = &[10, 166, 2, 10, 18, 98, 97, 115, 105, 99, 95, 118, 97, 108, 117, 101,
    115, 46, 112, 114, 111, 116, 111, 18, 25, 99, 111, 109, 46, 112, 97, 99, 116, 46, 112, 114,
    111, 116, 111, 98, 117, 102, 46, 101, 120, 97, 109, 112, 108, 101, 34, 107, 10, 9, 77, 101,
    115, 115, 97, 103, 101, 73, 110, 18, 14, 10, 2, 102, 49, 24, 1, 32, 1, 40, 8, 82, 2, 102, 49,
    18, 14, 10, 2, 102, 50, 24, 2, 32, 1, 40, 5, 82, 2, 102, 50, 18, 14, 10, 2, 102, 51, 24, 3,
    32, 1, 40, 13, 82, 2, 102, 51, 18, 14, 10, 2, 102, 52, 24, 4, 32, 1, 40, 1, 82, 2, 102, 52,
    18, 14, 10, 2, 102, 53, 24, 5, 32, 1, 40, 9, 82, 2, 102, 53, 18, 14, 10, 2, 102, 54, 24, 6,
    32, 1, 40, 12, 82, 2, 102, 54, 34, 30, 10, 10, 77, 101, 115, 115, 97, 103, 101, 79, 117, 116,
    18, 16, 10, 3, 111, 117, 116, 24, 1, 32, 1, 40, 8, 82, 3, 111, 117, 116, 50, 96, 10, 4, 84,
    101, 115, 116, 18, 88, 10, 7, 71, 101, 116, 84, 101, 115, 116, 18, 36, 46, 99, 111, 109, 46,
    112, 97, 99, 116, 46, 112, 114, 111, 116, 111, 98, 117, 102, 46, 101, 120, 97, 109, 112, 108,
    101, 46, 77, 101, 115, 115, 97, 103, 101, 73, 110, 26, 37, 46, 99, 111, 109, 46, 112, 97, 99,
    116, 46, 112, 114, 111, 116, 111, 98, 117, 102, 46, 101, 120, 97, 109, 112, 108, 101, 46, 77,
    101, 115, 115, 97, 103, 101, 79, 117, 116, 34, 0, 98, 6, 112, 114, 111, 116, 111, 51];
  let fds = FileDescriptorSet::decode(Bytes::from(descriptor_bytes)).unwrap();

  let main_descriptor = fds.file.iter()
    .find(|fd| fd.name.clone().unwrap_or_default() == "basic_values.proto")
    .unwrap();
  let message_descriptor = main_descriptor.message_type.iter()
    .find(|md| md.name.clone().unwrap_or_default() == "MessageIn").unwrap();
  let mut message_builder = MessageBuilder::new(&message_descriptor, "MessageIn", main_descriptor);
  let path = DocPath::new("$.f6").unwrap();
  let field_descriptor = message_descriptor.field.iter()
    .find(|fd| fd.name.clone().unwrap_or_default() == "f6")
    .unwrap();
  let field_config = json!([1, 2, 3, 4, 5]);
  let mut matching_rules = MatchingRuleCategory::empty("body");
  let mut generators = hashmap!{};
  let descriptor_cache = DescriptorCache::new(fds.clone());

  let result = build_field_value(
    &path,
    &mut message_builder,
    MessageFieldValueType::Normal,
    field_descriptor,
    "f6",
    &field_config,
    &mut matching_rules,
    &mut generators,
    &descriptor_cache
  );
  expect!(result).to(be_ok().value(Some(MessageFieldValue {
    name: "f6".to_string(),
    raw_value: Some("[1,2,3,4,5]".to_string()),
    rtype: RType::Bytes(vec![1, 2, 3, 4, 5])
  })));

  let field_config = json!([1, 2, "3", 4, 5]);
  let result = build_field_value(
    &path,
    &mut message_builder,
    MessageFieldValueType::Normal,
    field_descriptor,
    "f6",
    &field_config,
    &mut matching_rules,
    &mut generators,
    &descriptor_cache
  );
  expect!(result.unwrap_err().to_string()).to(be_equal_to(
    "Byte arrays can only be constructed from arrays of numbers, got '[1,2,\"3\",4,5]'"));

  let field_config = json!("1234567890");
  let result = build_field_value(
    &path,
    &mut message_builder,
    MessageFieldValueType::Normal,
    field_descriptor,
    "f6",
    &field_config,
    &mut matching_rules,
    &mut generators,
    &descriptor_cache
  );
  expect!(result).to(be_ok().value(Some(MessageFieldValue {
    name: "f6".to_string(),
    raw_value: Some("1234567890".to_string()),
    rtype: RType::Bytes(vec![49, 50, 51, 52, 53, 54, 55, 56, 57, 48])
  })));

  let field_config = json!("AQIDBAU=");
  let result = build_field_value(
    &path,
    &mut message_builder,
    MessageFieldValueType::Normal,
    field_descriptor,
    "f6",
    &field_config,
    &mut matching_rules,
    &mut generators,
    &descriptor_cache
  );
  expect!(result).to(be_ok().value(Some(MessageFieldValue {
    name: "f6".to_string(),
    raw_value: Some("AQIDBAU=".to_string()),
    rtype: RType::Bytes(vec![1, 2, 3, 4, 5])
  })));
}
