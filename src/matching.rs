//! Functions for matching Protobuf messages

use std::collections::HashMap;
use anyhow::anyhow;

use bytes::Bytes;
use log::{debug, trace, warn};
use maplit::hashmap;
use pact_matching::{BodyMatchResult, DiffConfig, MatchingContext, Mismatch};
use pact_models::content_types::ContentType;
use pact_models::matchingrules::{MatchingRule, MatchingRules};
use pact_models::path_exp::DocPath;
use pact_models::prelude::{MatchingRuleCategory, RuleLogic};
use pact_plugin_driver::proto::{Body, CompareContentsRequest};
use pact_plugin_driver::utils::{proto_struct_to_json, proto_value_to_json};
use prost_types::{DescriptorProto, FieldDescriptorProto, FileDescriptorSet};

use crate::message_decoder::{decode_message, ProtobufField};
use crate::utils::{find_message_type_by_name, is_map_field};

/// Match a single Protobuf message
pub fn match_message(
  message_name: &str,
  descriptors: &FileDescriptorSet,
  request: &CompareContentsRequest
) -> anyhow::Result<BodyMatchResult> {
  debug!("Looking for message '{}'", message_name);
  let message_descriptor = find_message_type_by_name(message_name, descriptors)?;

  let mut expected_message_bytes = request.expected.as_ref()
    .map(|body| body.content.clone().map(|content| Bytes::from(content)))
    .flatten()
    .unwrap_or_default();
  let expected_message = decode_message(&mut expected_message_bytes, &message_descriptor)?;
  debug!("expected message = {:?}", expected_message);

  let mut actual_message_bytes = request.actual.as_ref()
    .map(|body| body.content.clone().map(|content| Bytes::from(content)))
    .flatten()
    .unwrap_or_default();
  let actual_message = decode_message(&mut actual_message_bytes, &message_descriptor)?;
  debug!("actual message = {:?}", expected_message);

  let mut matching_rules = MatchingRuleCategory::empty("body");
  for (key, rules) in &request.rules {
    for rule in &rules.rule {
      let values = rule.values.as_ref().map(|v| proto_struct_to_json(v)).unwrap_or_default();
      matching_rules.add_rule(DocPath::new(key)?,
                              MatchingRule::create(&rule.r#type, &values)?,
                              RuleLogic::And
      );
    }
  }

  let plugin_config = hashmap!{};
  let diff_config = if request.allow_unexpected_keys {
    DiffConfig::AllowUnexpectedKeys
  } else {
    DiffConfig::NoUnexpectedKeys
  };
  let context = MatchingContext::new(diff_config, &matching_rules, &plugin_config);

  compare(&message_descriptor, &expected_message, &actual_message, context,
          &expected_message_bytes)
}

/// Match a Protobuf service call, which has an input and output message
pub fn match_service(
  service_name: &str,
  descriptors: &FileDescriptorSet,
  request: &CompareContentsRequest
) -> anyhow::Result<BodyMatchResult> {
  let (service, method) = service_name.split_once('/')
    .ok_or_else(|| anyhow!("Service name '{}' is not valid, it should be of the form <SERVICE>/<METHOD>", service_name))?;
  let service_descriptor = descriptors.file.iter().map(|descriptor| {
    descriptor.service.iter().find(|p| p.name.clone().unwrap_or_default() == service)
  })
    .filter(|result| result.is_some())
    .next()
    .flatten()
    .ok_or_else(|| anyhow!("Did not find a descriptor for service '{}'", service_name))?;

  let method_descriptor = service_descriptor.method.iter().find(|method_desc| {
    method_desc.name.clone().unwrap_or_default() == method
  }).ok_or_else(|| anyhow!("Did not find the method {} in the Protobuf file descriptor for service '{}'", method, service))?;

  let expected_content_type = ContentType::parse(
    request.expected.as_ref().map(|body| body.content_type.clone())
      .ok_or_else(|| anyhow!("Expected content type is not set"))?
      .as_str()
  ).map_err(|err| anyhow!(err))?;
  let expected_message_type = expected_content_type.attributes.get("message");
  let message_type = if let Some(message_type) = expected_message_type {
    let input_type = method_descriptor.input_type.clone().unwrap_or_default();
    if input_type == message_type.as_str() {
      input_type
    } else {
      method_descriptor.output_type.clone().unwrap_or_default()
    }
  } else {
    method_descriptor.output_type.clone().unwrap_or_default()
  };

  match_message(message_type.as_str(), descriptors, request)
}

fn compare(
  message_descriptor: &DescriptorProto,
  expected_message: &Vec<ProtobufField>,
  actual_message: &Vec<ProtobufField>,
  matching_context: MatchingContext,
  expected_message_bytes: &Bytes
) -> anyhow::Result<BodyMatchResult> {
  if expected_message.is_empty() {
    Ok(BodyMatchResult::Ok)
  } else if actual_message.is_empty() {
    Ok(BodyMatchResult::BodyMismatches(hashmap!{
      "$".to_string() => vec![Mismatch::BodyMismatch {
        path: "$".to_string(),
        expected: Some(expected_message_bytes.clone()),
        actual: None,
        mismatch: format!("Expected message '{}' but was missing or empty", message_descriptor.name.clone().unwrap_or_default())
      }]
    }))
  } else {
    compare_message(DocPath::root(), expected_message, actual_message, matching_context, message_descriptor)
  }
}

fn compare_message(
  path: DocPath,
  expected_message: &Vec<ProtobufField>,
  actual_message: &Vec<ProtobufField>,
  matching_context: MatchingContext,
  message_descriptor: &DescriptorProto,
) -> anyhow::Result<BodyMatchResult> {
  trace!("compareMessage({}, {:?}, {:?})", path, expected_message, actual_message);

  let mut results = vec![];
  for field in expected_message {
    if let Some(field_descriptor) = find_field_descriptor(field, message_descriptor) {
      let field_name = field_descriptor.name
        .unwrap_or_else(|| {
          warn!("Field number {} does not have a field name in the descriptor, will use the number", field.field_num);
          field.field_num.to_string()
        })?;
      let field_path = path.clone().push_field(field_name);
      if is_map_field(message_descriptor, &field_descriptor) {
        results.extend(compare_map_field(field_path, field, field_descriptor, actual_message, context));
      } else if field.is_repeated() {
        results.extend(compare_repeated_field(field_path, field, field_descriptor, actual_message, context));
      } else if !message_has_field(actual_message, field) {
        results.push(BodyItemMatchResult(constructPath(fieldPath), listOf(
          BodyMismatch(field.name, null, "Expected field '${field.name}' but was missing",
            constructPath(fieldPath),
            generateProtoDiff(expected, actual))
        )));
      } else {
        results.extend(compare_field(field_path, field, field_descriptor, actual.getField(field), { generateProtoDiff(expected, actual) }, context))
      }
    } else {
      warn!("Did not find a field descriptor for field number {} in the expected message, skipping it", field.field_num);
    }
  }

  //       if (!context.allowUnexpectedKeys) {
  //         actual.allFields.forEach { (field, _) ->
  //           val fieldPath = path + field.name
  //           if (!field.isRepeated && !expected.hasField(field)) {
  //             result.add(BodyItemMatchResult(constructPath(fieldPath), listOf(
  //               BodyMismatch(null, field.name, "Received unexpected field '${field.name}'",
  //                 constructPath(fieldPath),
  //                 generateProtoDiff(expected, actual))
  //             )))
  //           }
  //         }
  //       }

  todo!()
}

fn find_field_descriptor(field: &ProtobufField, descriptor: &DescriptorProto) -> Option<FieldDescriptorProto> {
  descriptor.field.iter()
    .find(|field_desc| field_desc.number.unwrap_or_default() == field.field_num as i32)
    .cloned()
}
