//! Module for processing and comparing protobuf messages

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use anyhow::anyhow;
use itertools::{Either, Itertools};
use log::{debug, LevelFilter, max_level, trace, warn};
use maplit::{btreemap, hashmap};
use pact_models::generators::Generator;
use pact_models::json_utils::json_to_string;
use pact_models::matchingrules;
use pact_models::matchingrules::MatchingRuleCategory;
use pact_models::matchingrules::expressions::{is_matcher_def, MatchingReference, parse_matcher_def, ValueType};
use pact_models::path_exp::DocPath;
use pact_models::prelude::RuleLogic;
use pact_plugin_driver::proto::{
  Body,
  InteractionResponse,
  PluginConfiguration,
  MatchingRules,
  MatchingRule
};
use pact_plugin_driver::proto::body::ContentTypeHint;
use pact_plugin_driver::proto::interaction_response::MarkupType;
use pact_plugin_driver::utils::{proto_value_to_json, proto_value_to_string, to_proto_struct};
use prost_types::{DescriptorProto, field_descriptor_proto, FieldDescriptorProto, FileDescriptorProto, ServiceDescriptorProto};
use prost_types::field_descriptor_proto::Type;
use prost_types::value::Kind;
use serde_json::{json, Value};
use tokio::fs::File;
use tokio::io::AsyncReadExt;

use crate::message_builder::{MessageBuilder, MessageFieldValue, MessageFieldValueType, RType};
use crate::protoc::Protoc;
use crate::utils::{find_enum_value_by_name, find_message_type_by_name, find_message_type_in_file_descriptor, find_nested_type, is_map_field, is_repeated, last_name, proto_struct_to_btreemap, proto_type_name};

/// Process the provided protobuf file and configure the interaction
pub(crate) async fn process_proto(
  proto_file: String,
  protoc: &Protoc,
  config: BTreeMap<String, prost_types::Value>
) -> anyhow::Result<(Vec<InteractionResponse>, PluginConfiguration)> {
  debug!("Parsing proto file '{}'", proto_file);
  let proto_file = Path::new(proto_file.as_str());
  let (descriptors, digest, descriptor_bytes) = protoc.parse_proto_file(proto_file).await?;
  debug!("Parsed proto file OK, file descriptors = {:?}", descriptors.file.iter().map(|file| file.name.as_ref()).collect_vec());

  let file_descriptors: HashMap<String, &FileDescriptorProto> = descriptors.file
    .iter().map(|des| (des.name.clone().unwrap_or_default(), des))
    .collect();
  let file_name = &*proto_file.file_name().unwrap_or_default().to_string_lossy();
  let descriptor = match file_descriptors.get(file_name) {
    None => return Err(anyhow!("Did not find a file proto descriptor for the provided proto file '{}'", file_name)),
    Some(des) => *des
  };

  if max_level() >= LevelFilter::Trace {
    trace!("All message types in proto descriptor");
    for message_type in &descriptor.message_type {
      trace!("  {:?}", message_type.name);
    }
  }

  let descriptor_encoded = base64::encode(&descriptor_bytes);
  let descriptor_hash = format!("{:x}", md5::compute(&descriptor_bytes));
  let mut interactions = vec![];

  if let Some(message_type) = config.get("pact:message-type") {
    let message = proto_value_to_string(message_type)
      .ok_or_else(|| anyhow!("Did not get a valid value for 'pact:message-type'. It should be a string"))?;
    debug!("Configuring a Protobuf message {}", message);
    let result = configure_protobuf_message(message.as_str(), config, descriptor, &file_descriptors, proto_file, descriptor_hash.as_str())?;
    interactions.push(result);
  } else if let Some(service_name) = config.get("pact:proto-service") {
    let service_name = proto_value_to_string(service_name)
      .ok_or_else(|| anyhow!("Did not get a valid value for 'pact:proto-service'. It should be a string"))?;
    debug!("Configuring a Protobuf service {}", service_name);
    let (request_part, response_part) = configure_protobuf_service(service_name, config, descriptor, &file_descriptors, proto_file, descriptor_hash.as_str())?;
    interactions.push(request_part);
    interactions.push(response_part);
  }

  let mut f = File::open(proto_file).await?;
  let mut file_contents = String::new();
  f.read_to_string(&mut file_contents).await?;

  let digest_str = format!("{:x}", digest);
  let plugin_config = PluginConfiguration {
    interaction_configuration: None,
    pact_configuration: Some(to_proto_struct(hashmap!{
      digest_str => json!({
        "protoFile": file_contents,
        "protoDescriptors": descriptor_encoded
      })
    }))
  };

  Ok((interactions, plugin_config))
}

/// Configure the interaction for a Protobuf service method, which has an input and output message
fn configure_protobuf_service(
  service_name: String,
  config: BTreeMap<String, prost_types::Value>,
  descriptor: &FileDescriptorProto,
  all_descriptors: &HashMap<String, &FileDescriptorProto>,
  proto_file: &Path,
  descriptor_hash: &str
) -> anyhow::Result<(InteractionResponse, InteractionResponse)> {
  debug!("Looking for service and method with name '{}'", service_name);
  let service_and_proc = service_name.split_once('/')
    .ok_or_else(|| anyhow!("Service name '{}' is not valid, it should be of the form <SERVICE>/<METHOD>", service_name))?;
  let service_descriptor = descriptor.service
    .iter().find(|p| p.name.clone().unwrap_or_default() == service_and_proc.0)
    .ok_or_else(|| anyhow!("Did not find a descriptor for service '{}'", service_name))?;
  construct_protobuf_interaction_for_service(service_descriptor, config, service_and_proc.0,
    service_and_proc.1, all_descriptors, descriptor)
    .map(|(request, response)| {
      let plugin_configuration = Some(PluginConfiguration {
        interaction_configuration: Some(to_proto_struct(hashmap! {
            "service".to_string() => Value::String(service_name.to_string()),
            "descriptorKey".to_string() => Value::String(descriptor_hash.to_string())
          })),
        pact_configuration: None
      });
      (
        InteractionResponse { plugin_configuration: plugin_configuration.clone(), .. request },
        InteractionResponse { plugin_configuration, .. response }
      )
    })
}

/// Constructs an interaction for the given Protobuf service descriptor
fn construct_protobuf_interaction_for_service(
  descriptor: &ServiceDescriptorProto,
  config: BTreeMap<String, prost_types::Value>,
  service_name: &str,
  method_name: &str,
  all_descriptors: &HashMap<String, &FileDescriptorProto>,
  file_descriptor: &FileDescriptorProto
) -> anyhow::Result<(InteractionResponse, InteractionResponse)> {
  if !config.contains_key("response") {
    return Err(anyhow!("A Protobuf service requires a 'response' configuration"))
  }

  let method_descriptor = descriptor.method.iter()
    .find(|m| m.name.clone().unwrap_or_default() == method_name)
    .ok_or_else(|| anyhow!("Did not find a method descriptor for method '{}' in service '{}'", method_name, service_name))?;

  let input_name = method_descriptor.input_type.as_ref().ok_or_else(|| anyhow!("Input message name is empty for service {}/{}", service_name, method_name))?;
  let output_name = method_descriptor.output_type.as_ref().ok_or_else(|| anyhow!("Input message name is empty for service {}/{}", service_name, method_name))?;
  let input_message_name = last_name(input_name.as_str());
  let request_descriptor = find_message_descriptor(input_message_name, all_descriptors)?;
  let output_message_name = last_name(output_name.as_str());
  let response_descriptor = find_message_descriptor(output_message_name, all_descriptors)?;

  let request_part = config.get("request").map(|request_config| {
    request_config.kind.as_ref().map(|kind| {
      match kind {
        Kind::StructValue(s) => Some(proto_struct_to_btreemap(s)),
        _ => None
      }
    }).flatten()
  })
    .flatten()
    .map(|config| construct_protobuf_interaction_for_message(&request_descriptor,
      config, input_message_name, "request", file_descriptor))
    .ok_or_else(|| anyhow!("A Protobuf service requires a 'request' configuration in map format"))??;

  let response_part = config.get("response").map(|response_config| {
    response_config.kind.as_ref().map(|kind| {
      match kind {
        Kind::StructValue(s) => Some(proto_struct_to_btreemap(s)),
        _ => None
      }
    }).flatten()
  })
    .flatten()
    .map(|config| construct_protobuf_interaction_for_message(&response_descriptor,
       config, output_message_name, "response", file_descriptor))
    .ok_or_else(|| anyhow!("A Protobuf service requires a 'response' configuration in map format"))??;

  Ok((request_part, response_part))
}

fn find_message_descriptor(message_name: &str, all_descriptors: &HashMap<String, &FileDescriptorProto>) -> anyhow::Result<DescriptorProto> {
  all_descriptors.values().map(|descriptor| {
    descriptor.message_type.iter()
      .find(|p| p.name.clone().unwrap_or_default() == message_name)
  }).find(|d| d.is_some())
    .flatten()
    .cloned()
    .ok_or_else(|| anyhow!("Did not find the descriptor for message {}", message_name))
}

/// Configure the interaction for a single Protobuf message
fn configure_protobuf_message(
  message_name: &str,
  config: BTreeMap<String, prost_types::Value>,
  descriptor: &FileDescriptorProto,
  all_descriptors: &HashMap<String, &FileDescriptorProto>,
  proto_file: &Path,
  descriptor_hash: &str
) -> anyhow::Result<InteractionResponse> {
  trace!(">> configure_protobuf_message({}, {:?}, {})", message_name, proto_file, descriptor_hash);
  debug!("Looking for message of type '{}'", message_name);
  let message_descriptor = descriptor.message_type
    .iter().find(|p| p.name.clone().unwrap_or_default() == message_name)
    .ok_or_else(|| anyhow!("Did not find a descriptor for message '{}'", message_name))?;
  construct_protobuf_interaction_for_message(message_descriptor, config, message_name, "", descriptor)
    .map(|interaction| {
      InteractionResponse {
        plugin_configuration: Some(PluginConfiguration {
          interaction_configuration: Some(to_proto_struct(hashmap!{
            "message".to_string() => Value::String(message_name.to_string()),
            "descriptorKey".to_string() => Value::String(descriptor_hash.to_string())
          })),
          pact_configuration: None
        }),
        .. interaction
      }
    })
}

/// Constructs an interaction for the given Protobuf message descriptor
fn construct_protobuf_interaction_for_message(
  message_descriptor: &DescriptorProto,
  config: BTreeMap<String, prost_types::Value>,
  message_name: &str,
  message_part: &str,
  file_descriptor: &FileDescriptorProto
) -> anyhow::Result<InteractionResponse> {
  trace!(">> construct_protobuf_interaction_for_message({}, {}, {:?}, {:?})", message_name,
    message_part, file_descriptor.name, config.keys());

  let mut message_builder = MessageBuilder::new(message_descriptor, message_name, file_descriptor);
  let mut matching_rules = MatchingRuleCategory::empty("body");
  let mut generators = hashmap!{};

  debug!("Building message {} from Protobuf descriptor", message_name);
  let mut path = DocPath::root();
  if !message_part.is_empty() {
    path.push_field(message_part);
  }

  for (key, value) in &config {
    if !key.starts_with("pact:") {
      debug!("Building field for key {}", key);
      construct_message_field(&mut message_builder, &mut matching_rules, &mut generators, key, &proto_value_to_json(value), &path)?;
    }
  }

  debug!("Constructing response to return");
  trace!("Final message builder: {:?}", message_builder);
  trace!("matching rules: {:?}", matching_rules);
  trace!("generators: {:?}", generators);

  let rules = matching_rules.rules.iter().map(|(path, rule_list)| {
    (path.to_string(), MatchingRules {
      rule: rule_list.rules.iter().map(|rule| {
        let rule_values = rule.values();
        let values = if rule_values.is_empty() {
          None
        } else {
          Some(to_proto_struct(rule_values.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()))
        };
        MatchingRule {
          r#type: rule.name(),
          values
        }
      }).collect()
    })
  }).collect();

  let generators = generators.iter().map(|(path, generator)| {
    let gen_values = generator.values();
    let values = if gen_values.is_empty() {
      None
    } else {
      Some(to_proto_struct(gen_values.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()))
    };
    (path.to_string(), pact_plugin_driver::proto::Generator {
      r#type: generator.name(),
      values
    })
  }).collect();

  Ok(InteractionResponse {
    contents: Some(Body {
      content_type: format!("application/protobuf;message={}", message_name),
      content: Some(message_builder.encode_message()?.to_vec()),
      content_type_hint: ContentTypeHint::Binary as i32,
    }),
    rules,
    generators,
    interaction_markup: message_builder.generate_markup("")?,
    interaction_markup_type: MarkupType::CommonMark as i32,
    part_name: message_part.to_string(),
    .. InteractionResponse::default()
  })
}

/// Construct a single field for a message from the provided config
fn construct_message_field(
  message_builder: &mut MessageBuilder,
  mut matching_rules: &mut MatchingRuleCategory,
  mut generators: &mut HashMap<String, Generator>,
  field_name: &str,
  value: &Value,
  path: &DocPath
) -> anyhow::Result<()> {
  trace!(">> construct_message_field({}, {}, {:?}, {:?}, {:?})", field_name, path, message_builder, matching_rules, generators);
  if !field_name.starts_with("pact:") {
    if let Some(field) = message_builder.field_by_name(field_name)  {
      match field.r#type {
        Some(r#type) => if r#type == field_descriptor_proto::Type::Message as i32 {
          // Embedded message
          build_embedded_message_field_value(message_builder, path, &field, field_name, value, &mut matching_rules, &mut generators)?;
        } else {
          // Non-embedded message field (singular value)
          build_field_value(path, message_builder, MessageFieldValueType::Normal, &field, field_name, value, &mut matching_rules, &mut generators)?;
        }
        None => {
          return Err(anyhow!("Message {} field {} is of an unknown type", message_builder.message_name, field_name))
        }
      }
    } else {
      let fields: HashSet<String> = message_builder.descriptor.field.iter()
        .map(|field| field.name.clone().unwrap_or_default())
        .collect();
      return Err(anyhow!("Message {} has no field {}. Fields are {:?}", message_builder.message_name, field_name, fields))
    }
  }
  trace!(">> construct_message_field done ({}, {})", field_name, path);
  Ok(())
}

/// Constructs the field value for a field in a message.
fn build_embedded_message_field_value(
  message_builder: &mut MessageBuilder,
  path: &DocPath,
  field_descriptor: &FieldDescriptorProto,
  field: &str,
  value: &Value,
  matching_rules: &mut MatchingRuleCategory,
  generators: &mut HashMap<String, Generator>
) -> anyhow::Result<()> {
  trace!(">> build_embedded_message_field_value({:?}, {}, {}, {:?}, {:?}, {:?})", message_builder, path, field, value, matching_rules, generators);

  if is_repeated(field_descriptor) && !is_map_field(&message_builder.descriptor, field_descriptor) {
    debug!("{} is a repeated field", field);

    match value {
      Value::Array(list) => {
        for (index, item) in list.iter().enumerate() {
          let index_path = path.join(index.to_string());
          build_single_embedded_field_value(&index_path, message_builder, MessageFieldValueType::Repeated, field_descriptor, field, item,
            matching_rules, generators)?;
        }
        Ok(())
      }
      Value::Object(map) => if let Some(definition) = map.get("pact:match") {
        let definition = json_to_string(definition);
        debug!("Configuring repeated field from a matcher definition expression '{}'", definition);
        let mrd = parse_matcher_def( definition.as_str())?;

        let each_value = mrd.rules.iter()
          .filter_map(|rule| rule.clone().left())
          .find_map(|rule| match rule {
            matchingrules::MatchingRule::EachValue(def) => Some(def),
            _ => None
          });
        if let Some(each_value_def) = &each_value {
          debug!("Found each like matcher");
          if mrd.rules.len() > 1 {
            warn!("{}: each value matcher can not be combined with other matchers, ignoring the other matching rules", path);
          }

          matching_rules.add_rule(path.clone(), matchingrules::MatchingRule::Values, RuleLogic::And);
          matching_rules.add_rule(path.join("*"), matchingrules::MatchingRule::Type, RuleLogic::And);

          match each_value_def.rules.first() {
            Some(either) => match either {
              Either::Left(_) => {
                matching_rules.add_rule(path.clone(), matchingrules::MatchingRule::EachValue(each_value_def.clone()), RuleLogic::And);
                if let Some(generator) = &each_value_def.generator {
                  generators.insert(path.to_string(), generator.clone());
                }
                let constructed_value = value_for_type(field, each_value_def.value.as_str(), field_descriptor, &message_builder.descriptor)?;
                message_builder.set_field_value(field_descriptor, field, constructed_value);
                Ok(())
              }
              Either::Right(reference) => if let Some(field_value) = map.get(reference.name.as_str()) {
                build_single_embedded_field_value(&path, message_builder, MessageFieldValueType::Repeated,
                  field_descriptor, field, field_value, matching_rules, generators).map(|_| ())
              } else {
                Err(anyhow!("Expression '{}' refers to non-existent item '{}'", definition, reference.name))
              }
            }
            None => Err(anyhow!("Got an EachValue matcher with no associated matching rules to apply"))
          }
        } else {
          if !mrd.rules.is_empty() {
            for rule in &mrd.rules {
              match rule {
                Either::Left(rule) => matching_rules.add_rule(path.clone(), rule.clone(), RuleLogic::And),
                Either::Right(mr) => return Err(anyhow!("References can only be used with an EachValue matcher - {:?}", mr))
              }
            }
          }
          if let Some(generator) = mrd.generator {
            generators.insert(path.to_string(), generator);
          }

          let constructed = value_for_type(field, mrd.value.as_str(), field_descriptor, &message_builder.descriptor)?;
          message_builder.add_repeated_field_value(field_descriptor, field, constructed);

          Ok(())
        }
      } else {
        build_single_embedded_field_value(path, message_builder, MessageFieldValueType::Repeated, field_descriptor, field, value,
          matching_rules, generators).map(|_| ())
      }
      _ => build_single_embedded_field_value(path, message_builder, MessageFieldValueType::Repeated, field_descriptor, field, value,
        matching_rules, generators).map(|_| ())
    }
  } else {
    build_single_embedded_field_value(path, message_builder, MessageFieldValueType::Normal, field_descriptor, field, value,
      matching_rules, generators)
      .map(|_| ())
  }
}

/// Construct a non-repeated embedded message field
fn build_single_embedded_field_value(
  path: &DocPath,
  message_builder: &mut MessageBuilder,
  field_type: MessageFieldValueType,
  field_descriptor: &FieldDescriptorProto,
  field: &str,
  value: &Value,
  matching_rules: &mut MatchingRuleCategory,
  generators: &mut HashMap<String, Generator>
) -> anyhow::Result<Option<MessageFieldValue>> {
  trace!(">> build_single_embedded_field_value('{}', {:?}, {}, {:?}, {:?}, {:?})", path, field_descriptor.name,
    field, value, matching_rules, generators);

  debug!("Configuring message field '{}' (type {:?})", field, field_descriptor.type_name);
  let type_name = field_descriptor.type_name.clone().unwrap_or_default();
  match type_name.as_str() {
    ".google.protobuf.BytesValue" => {
      debug!("Field is a Protobuf BytesValue");
      if let Value::String(_) = value {
        build_field_value(path, message_builder, field_type, field_descriptor, field, value, matching_rules, generators)
      } else {
        Err(anyhow!("Fields of type google.protobuf.BytesValue must be configured with a single string value"))
      }
    }
    ".google.protobuf.Struct" => {
      debug!("Field is a Protobuf Struct");
      build_struct_field(path, message_builder, field_type, field_descriptor, field, value, matching_rules, generators)
    }
    _ => if is_map_field(&message_builder.descriptor, field_descriptor) {
      debug!("Message field '{}' is a Map field", field);
      build_map_field(path, message_builder, field_descriptor, field, value, matching_rules, generators)?;
      Ok(None)
    } else {
      if let Value::Object(config) = value {
        debug!("Configuring the message from config {:?}", config);
        let message_name = last_name(type_name.as_str());
        let embedded_type = find_message_type_in_file_descriptor(message_name, &message_builder.file_descriptor)?;
        let mut embedded_builder = MessageBuilder::new(&embedded_type, message_name, &message_builder.file_descriptor);

        if let Some(definition) = config.get("pact:match") {
          let mrd = parse_matcher_def(json_to_string(definition).as_str())?;
          // when (val ruleDefinition = MatchingRuleDefinition.parseMatchingRuleDefinition(definition)) {
          //   is Ok -> for (rule in ruleDefinition.value.rules) {
          //     when (rule) {
          //       is Either.A -> TODO()
          //       is Either.B -> TODO()
          //     }
          //   }
          todo!()
        } else {
          for (key, value) in config {
            if !key.starts_with("pact:") {
              let field_path = path.join(key);
              construct_message_field(&mut embedded_builder, matching_rules, generators, key, value, &field_path)?;
            }
          }
          let field_value = MessageFieldValue {
            name: field.to_string(),
            raw_value: None,
            rtype: RType::Message(Box::new(embedded_builder))
          };
          message_builder.set_field_value(field_descriptor, field, field_value.clone());
          Ok(Some(field_value))
        }
      } else {
        Err(anyhow!("For message fields, you need to define a Map of expected fields, got {:?}", value))
      }
    }
  }
}

/// Create a field value of type google.protobuf.Struct
fn build_struct_field(
  path: &DocPath,
  message_builder: &mut MessageBuilder,
  field_type: MessageFieldValueType,
  field_descriptor: &FieldDescriptorProto,
  field_name: &str,
  field_value: &Value,
  matching_rules: &mut MatchingRuleCategory,
  generators: &mut HashMap<String, Generator>
) -> anyhow::Result<Option<MessageFieldValue>> {
  trace!(">> build_struct_field('{}', {}, {:?}, {:?}, {:?}, {:?})", path, field_name, field_value,
    message_builder, matching_rules, generators);

  match field_value {
    Value::Object(map) => if let Some(matching_def) = map.get("pact:match") {
      //       if (fieldsMap.containsKey("pact:match")) {
      //         val expression = fieldsMap["pact:match"]!!.stringValue
      //         when (val ruleDefinition = MatchingRuleDefinition.parseMatchingRuleDefinition(expression)) {
      //           is Ok -> TODO()
      //           is Err -> {
      //             logger.error { "'$expression' is not a valid matching rule definition - ${ruleDefinition.error}" }
      //             throw RuntimeException("'$expression' is not a valid matching rule definition - ${ruleDefinition.error}")
      //           }
      //         }
      //       }
      todo!()
    } else {
      let mut fields = btreemap!{};
      for (key, value) in map {
        let field_path = path.join(key);
        let proto_value = build_proto_value(path, value, matching_rules, generators)?;
        fields.insert(key.clone(), proto_value);
      }

      let s = prost_types::Struct { fields };
      let message_field_value = MessageFieldValue {
        name: field_name.to_string(),
        raw_value: None,
        rtype: RType::Struct(s)
      };
      match field_type {
        MessageFieldValueType::Repeated => message_builder.add_repeated_field_value(field_descriptor, field_name, message_field_value.clone()),
        _ => message_builder.set_field_value(field_descriptor, field_name, message_field_value.clone())
      };
      Ok(Some(message_field_value))
    }
    _ => Err(anyhow!("google.protobuf.Struct fields need to be configured with a Map, got {:?}", field_value))
  }
}

fn build_proto_value(
  path: &DocPath,
  value: &Value,
  matching_rules: &mut MatchingRuleCategory,
  generators: &mut HashMap<String, Generator>
) -> anyhow::Result<prost_types::Value> {
  trace!(">> build_proto_value('{}', {:?}, {:?}, {:?})", path, value, matching_rules, generators);
  match value {
    Value::Null => Ok(prost_types::Value { kind: Some(prost_types::value::Kind::NullValue(0)) }),
    Value::Bool(b) => Ok(prost_types::Value { kind: Some(prost_types::value::Kind::BoolValue(*b)) }),
    Value::Number(n) => if let Some(f) = n.as_f64() {
      Ok(prost_types::Value { kind: Some(prost_types::value::Kind::NumberValue(f)) })
    } else if let Some(f) = n.as_u64() {
      Ok(prost_types::Value { kind: Some(prost_types::value::Kind::NumberValue(f as f64)) })
    } else if let Some(f) = n.as_i64() {
      Ok(prost_types::Value { kind: Some(prost_types::value::Kind::NumberValue(f as f64)) })
    } else {
      Err(anyhow!("Got an invalid number (not f64, i64 or u64)"))
    },
    Value::String(s) => if is_matcher_def(s.as_str()) {
      let mrd = parse_matcher_def(s.as_str())?;
      if !mrd.rules.is_empty() {
        for rule in &mrd.rules {
          match rule {
            Either::Left(rule) => matching_rules.add_rule(path.clone(), rule.clone(), RuleLogic::And),
            Either::Right(mr) => return Err(anyhow!("Was expecting a value, but got a matching reference {:?}", mr))
          }
        }
      }
      if let Some(generator) = mrd.generator {
        generators.insert(path.to_string(), generator);
      }

      match mrd.value_type {
        ValueType::Unknown | ValueType::String => Ok(prost_types::Value { kind: Some(prost_types::value::Kind::StringValue(mrd.value.clone())) }),
        ValueType::Number | ValueType::Decimal => {
          let num: f64 = mrd.value.parse()?;
          Ok(prost_types::Value { kind: Some(prost_types::value::Kind::NumberValue(num)) })
        }
        ValueType::Integer => {
          let num: i64 = mrd.value.parse()?;
          Ok(prost_types::Value { kind: Some(prost_types::value::Kind::NumberValue(num as f64)) })
        }
        ValueType::Boolean => {
          let b: bool = mrd.value.parse()?;
          Ok(prost_types::Value { kind: Some(prost_types::value::Kind::BoolValue(b)) })
        }
      }
    } else {
      Ok(prost_types::Value { kind: Some(prost_types::value::Kind::StringValue(s.clone())) })
    }
    Value::Array(a) => {
      let mut values = a.iter().enumerate().map(|(index, v)| {
        let index_path = path.join(index.to_string());
        build_proto_value(&index_path, v, matching_rules, generators)
      });
      if let Some(err) = values.find_map(|v| v.err()) {
        return Err(anyhow!("Could not construct a Protobuf list value - {}", err))
      }
      // Unwrap here is safe as the previous statement would catch an error
      let list = prost_types::ListValue { values: values.map(|v| v.unwrap()).collect() };
      Ok(prost_types::Value { kind: Some(prost_types::value::Kind::ListValue(list)) })
    }
    Value::Object(map) => {
      let mut fields = btreemap!{};
      for (key, value) in map {
        let field_path = path.join(key);
        let proto_value = build_proto_value(path, value, matching_rules, generators)?;
        fields.insert(key.clone(), proto_value);
      }

      let s = prost_types::Struct { fields };
      Ok(prost_types::Value { kind: Some(prost_types::value::Kind::StructValue(s)) })
    }
  }
}

/// Constructs a message map field. Map fields are repeated fields with an embedded entry message
/// type which has a key and value field.
fn build_map_field(
  path: &DocPath,
  message_builder: &mut MessageBuilder,
  field_descriptor: &FieldDescriptorProto,
  field: &str,
  value: &Value,
  matching_rules: &mut MatchingRuleCategory,
  generators: &mut HashMap<String, Generator>
) -> anyhow::Result<()> {
  trace!(">> build_map_field('{}', {}, {:?}, {:?})", path, field, value, message_builder);
  let field_type = field_descriptor.type_name.clone().unwrap_or_default();
  trace!("build_map_field: field_type = {}", field_type);

  if let Value::Object(config) = value {
    if let Some(definition) = config.get("pact:match") {
      debug!("Parsing matching rule definition {:?}", definition);
      let definition = json_to_string(definition);
      let mrd = parse_matcher_def(definition.as_str())?;
      if !mrd.rules.is_empty() {
        for rule in &mrd.rules {
          match rule {
            Either::Left(rule) => {
              matching_rules.add_rule(path.clone(), rule.clone(), RuleLogic::And)
            },
            Either::Right(mr) => todo!()
          }
        }
      }
      if let Some(generator) = mrd.generator {
        generators.insert(path.to_string(), generator);
      }
    }

    if let Some(map_type) = find_nested_type(&message_builder.descriptor, field_descriptor) {
      let message_name = map_type.name.clone().unwrap_or_default();
      let key_descriptor = map_type.field.iter()
        .find(|f| f.name.clone().unwrap_or_default() == "key")
        .ok_or_else(|| anyhow!("Did not find the key field in the descriptor for the map field"))?;
      let value_descriptor = map_type.field.iter()
        .find(|f| f.name.clone().unwrap_or_default() == "value")
        .ok_or_else(|| anyhow!("Did not find the value field in the descriptor for the map field"))?;

      let mut embedded_builder = MessageBuilder::new(&map_type, message_name.as_str(), &message_builder.file_descriptor);
      for (inner_field, value) in config {
        if inner_field != "pact:match" {
          let entry_path = path.join(inner_field);

          let key_value = build_field_value(&entry_path, &mut embedded_builder, MessageFieldValueType::Normal,
            &key_descriptor, "key", &Value::String(inner_field.clone()), matching_rules, generators)?
            .ok_or_else(|| anyhow!("Was not able to construct map key value {:?}", key_descriptor.type_name))?;
          let value_value = build_single_embedded_field_value(&entry_path, &mut embedded_builder, MessageFieldValueType::Normal,
            &value_descriptor, "value", value, matching_rules, generators)?
            .ok_or_else(|| anyhow!("Was not able to construct map value value {:?}", value_descriptor.type_name))?;
          message_builder.add_map_field_value(field_descriptor, field, key_value, value_value);
        }
      }
      Ok(())
    } else {
      Err(anyhow!("Did not find the nested map type {:?} in the message descriptor nested types", field_descriptor.type_name))
    }
  } else {
    Err(anyhow!("Map fields need to be configured with a Map, got {:?}", value))
  }
}

/// Constructs a simple message field (non-repeated or map) from the configuration value and
/// updates the matching rules and generators for it.
fn build_field_value(
  path: &DocPath,
  message_builder: &mut MessageBuilder,
  field_type: MessageFieldValueType,
  descriptor: &FieldDescriptorProto,
  field_name: &str,
  value: &Value,
  matching_rules: &mut MatchingRuleCategory,
  generators: &mut HashMap<String, Generator>
) -> anyhow::Result<Option<MessageFieldValue>> {
  trace!(">> build_field_value({}, {}, {:?})", path, field_name, value);

  match value {
    Value::Null => Ok(None),
    Value::String(s) => {
      let field_path = path.join(field_name);

      let constructed_value = if is_matcher_def(s.as_str()) {
        let mrd = parse_matcher_def(s.as_str())?;
        if !mrd.rules.is_empty() {
          for rule in &mrd.rules {
            match rule {
              Either::Left(rule) => matching_rules.add_rule(field_path.clone(), rule.clone(), RuleLogic::And),
              Either::Right(mr) => return Err(anyhow!("Was expecting a value for '{}', but got a matching reference {:?}", field_path, mr))
            }
          }
        }
        if let Some(generator) = mrd.generator {
          generators.insert(field_path.to_string(), generator);
        }
        value_for_type(field_name, mrd.value.as_str(), descriptor, &message_builder.descriptor)?
      } else {
        value_for_type(field_name, s.as_str(), descriptor, &message_builder.descriptor)?
      };

      debug!("Setting field {:?} to value {:?}", field_name, constructed_value);
      match field_type {
        MessageFieldValueType::Repeated => message_builder.add_repeated_field_value(descriptor, field_name, constructed_value.clone()),
        _ => message_builder.set_field_value(descriptor, field_name, constructed_value.clone()),
      };
      Ok(Some(constructed_value))
    }
    _ => Err(anyhow!("Field values must be configured with a string value, got {:?}", value))
  }
}

fn value_for_type(
  field_name: &str,
  field_value: &str,
  descriptor: &FieldDescriptorProto,
  message_descriptor: &DescriptorProto
) -> anyhow::Result<MessageFieldValue> {
  trace!("value_for_type({}, {}, _)", field_name, field_value);

  let type_name = descriptor.type_name.clone().unwrap_or_default();
  debug!("Creating value for type {:?} from '{}'", type_name, field_value);

  let t = descriptor.r#type();
  match t {
    Type::Double => MessageFieldValue::double(field_name, field_value),
    Type::Float => MessageFieldValue::float(field_name, field_value),
    Type::Int64 | Type::Sfixed64 | Type::Sint64 => MessageFieldValue::integer_64(field_name, field_value),
    Type::Uint64 | Type::Fixed64 => MessageFieldValue::uinteger_64(field_name, field_value),
    Type::Int32 | Type::Sfixed32 | Type::Sint32 => MessageFieldValue::integer_32(field_name, field_value),
    Type::Uint32 | Type::Fixed32 => MessageFieldValue::uinteger_32(field_name, field_value),
    Type::Bool => MessageFieldValue::boolean(field_name, field_value),
    Type::String => Ok(MessageFieldValue::string(field_name, field_value)),
    Type::Message => {
      if type_name == ".google.protobuf.BytesValue" {
        Ok(MessageFieldValue::bytes(field_name, field_value))
      } else {
        Err(anyhow!("value_for_type: Protobuf field {} has an unsupported type {:?} {}", field_name, t, type_name))
      }
    }
    Type::Bytes => Ok(MessageFieldValue::bytes(field_name, field_value)),
    Type::Enum => if let Some(n) = find_enum_value_by_name(message_descriptor, type_name.as_str(), field_value) {
      Ok(MessageFieldValue {
        name: field_name.to_string(),
        raw_value: Some(field_value.to_string()),
        rtype: RType::Integer32(n)
      })
    } else {
      Err(anyhow!("Protobuf enum value {} has no value {}", type_name, field_value))
    }
    _ => Err(anyhow!("Protobuf field {} has an unsupported type {:?}", field_name, t))
  }
}


#[cfg(test)]
mod tests {
  use expectest::prelude::*;
  use maplit::{btreemap, hashmap};
  use pact_plugin_driver::proto::{MatchingRules, MatchingRule};
  use pact_plugin_driver::proto::interaction_response::MarkupType;
  use prost_types::{DescriptorProto, field_descriptor_proto, FieldDescriptorProto, FileDescriptorProto};
  use prost_types::field_descriptor_proto::Type;
  use trim_margin::MarginTrimmable;

  use crate::message_builder::RType;
  use crate::protobuf::{construct_protobuf_interaction_for_message, value_for_type};

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
    let result = value_for_type("test", "test", &descriptor, &message_descriptor).unwrap();
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
    let result = value_for_type("test", "100", &descriptor, &message_descriptor).unwrap();
    expect!(result.name).to(be_equal_to("test"));
    expect!(result.raw_value).to(be_some().value("100".to_string()));
    expect!(result.rtype).to(be_equal_to(RType::UInteger64(100)));
  }

  #[test]
  fn construct_protobuf_interaction_for_message_test() {
    let file_descriptor = FileDescriptorProto {
      name: None,
      package: None,
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
      "implementation".to_string() => prost_types::Value { kind: Some(prost_types::value::Kind::StringValue("notEmpty('plugin-driver-rust')".to_string())) },
      "version".to_string() => prost_types::Value { kind: Some(prost_types::value::Kind::StringValue("matching(semver, '0.0.0')".to_string())) },
      "hash".to_string() => prost_types::Value { kind: Some(prost_types::value::Kind::StringValue("matching(integer, 1234)".to_string())) }
    };

    let result = construct_protobuf_interaction_for_message(&message_descriptor, config, "test_message", "", &file_descriptor).unwrap();

    let body = result.contents.as_ref().unwrap();
    expect!(body.content_type.as_str()).to(be_equal_to("application/protobuf;message=test_message"));
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
  }
}
