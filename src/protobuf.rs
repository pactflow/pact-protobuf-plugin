//! Module for processing and comparing protobuf messages

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use anyhow::anyhow;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use itertools::{Either, Itertools};
use maplit::{btreemap, hashmap};
use pact_models::generators::Generator;
use pact_models::json_utils::json_to_string;
use pact_models::matchingrules;
use pact_models::matchingrules::expressions::{is_matcher_def, MatchingRuleDefinition, parse_matcher_def, ValueType};
use pact_models::matchingrules::MatchingRuleCategory;
use pact_models::path_exp::DocPath;
use pact_models::prelude::RuleLogic;
use pact_plugin_driver::proto::{
  Body,
  InteractionResponse,
  MatchingRule,
  MatchingRules,
  PluginConfiguration
};
use pact_plugin_driver::proto::body::ContentTypeHint;
use pact_plugin_driver::proto::interaction_response::MarkupType;
use pact_plugin_driver::utils::{proto_value_to_json, proto_value_to_string, to_proto_struct};
use prost_types::{DescriptorProto, FieldDescriptorProto, FileDescriptorProto, ServiceDescriptorProto, Struct};
use prost_types::field_descriptor_proto::Type;
use prost_types::value::Kind;
use serde_json::{json, Value};
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tracing::{debug, error, instrument, trace, warn};
use tracing_core::LevelFilter;

use crate::message_builder::{MessageBuilder, MessageFieldValue, MessageFieldValueType, RType};
use crate::metadata::{MessageMetadata, process_metadata};
use crate::protoc::Protoc;
use crate::utils::{
  find_enum_value_by_name,
  find_enum_value_by_name_in_message,
  find_message_type_in_file_descriptors,
  find_nested_type,
  is_map_field,
  is_repeated_field,
  last_name,
  prost_string
};

/// Process the provided protobuf file and configure the interaction
pub(crate) async fn process_proto(
  proto_file: String,
  protoc: &Protoc,
  config: &BTreeMap<String, prost_types::Value>
) -> anyhow::Result<(Vec<InteractionResponse>, PluginConfiguration)> {
  debug!("Parsing proto file '{}'", proto_file);
  trace!(">> process_proto({proto_file}, {config:?})");

  let proto_file = Path::new(proto_file.as_str());
  let (descriptors, digest, descriptor_bytes) = protoc.parse_proto_file(proto_file).await?;
  debug!("Parsed proto file OK, file descriptors = {:?}", descriptors.file.iter().map(|file| file.name.as_ref()).collect_vec());
  trace!("Descriptor bytes {:?}", descriptor_bytes.as_slice());

  let file_descriptors: HashMap<String, &FileDescriptorProto> = descriptors.file
    .iter().map(|des| (des.name.clone().unwrap_or_default(), des))
    .collect();
  let file_name = &*proto_file.file_name().unwrap_or_default().to_string_lossy();
  let descriptor = match file_descriptors.get(file_name) {
    None => return Err(anyhow!("Did not find a file proto descriptor for the provided proto file '{}'", file_name)),
    Some(des) => *des
  };

  if LevelFilter::current() >= LevelFilter::TRACE {
    trace!("All message types in proto descriptor");
    for message_type in &descriptor.message_type {
      trace!("  {:?}", message_type.name);
    }
  }

  let descriptor_encoded = BASE64.encode(&descriptor_bytes);
  let descriptor_hash = format!("{:x}", md5::compute(&descriptor_bytes));
  let mut interactions = vec![];

  if let Some(message_type) = config.get("pact:message-type") {
    let message = proto_value_to_string(message_type)
      .ok_or_else(|| anyhow!("Did not get a valid value for 'pact:message-type'. It should be a string"))?;
    debug!("Configuring a Protobuf message {}", message);
    let result = configure_protobuf_message(message.as_str(), config, descriptor,
      descriptor_hash.as_str(), &file_descriptors)?;
    interactions.push(result);
  } else if let Some(service_name) = config.get("pact:proto-service") {
    let service_name = proto_value_to_string(service_name)
      .ok_or_else(|| anyhow!("Did not get a valid value for 'pact:proto-service'. It should be a string"))?;
    debug!("Configuring a Protobuf service {}", service_name);
    let (request_part, response_part) = configure_protobuf_service(service_name.as_str(), config, descriptor,
      &file_descriptors, descriptor_hash.as_str())?;
    if let Some(request_part) = request_part {
      interactions.push(request_part);
    }
    interactions.extend_from_slice(&response_part);
  }

  let mut f = File::open(proto_file).await?;
  let mut file_contents = String::new();
  f.read_to_string(&mut file_contents).await?;

  let digest_str = format!("{:x}", digest);
  let plugin_config = PluginConfiguration {
    interaction_configuration: None,
    pact_configuration: Some(to_proto_struct(&hashmap!{
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
  service_name: &str,
  config: &BTreeMap<String, prost_types::Value>,
  descriptor: &FileDescriptorProto,
  all_descriptors: &HashMap<String, &FileDescriptorProto>,
  descriptor_hash: &str
) -> anyhow::Result<(Option<InteractionResponse>, Vec<InteractionResponse>)> {
  trace!(">> configure_protobuf_service({service_name}, {config:?}, {descriptor_hash})");

  debug!("Looking for service and method with name '{}'", service_name);
  let (service, proc_name) = service_name.split_once('/')
    .ok_or_else(|| anyhow!("Service name '{}' is not valid, it should be of the form <SERVICE>/<METHOD>", service_name))?;
  let service_descriptor = descriptor.service
    .iter().find(|p| p.name.clone().unwrap_or_default() == service)
    .ok_or_else(|| anyhow!("Did not find a descriptor for service '{}'", service_name))?;
  construct_protobuf_interaction_for_service(service_descriptor, config, service,
    proc_name, all_descriptors, descriptor)
    .map(|(request, response)| {
      let plugin_configuration = Some(PluginConfiguration {
        interaction_configuration: Some(to_proto_struct(&hashmap! {
            "service".to_string() => Value::String(
              service_name.split_once(':').map(|(s, _)| s).unwrap_or(service_name).to_string()
            ),
            "descriptorKey".to_string() => Value::String(descriptor_hash.to_string())
          })),
        pact_configuration: None
      });
      trace!("request = {request:?}");
      trace!("response = {response:?}");
      (
        request.map(|r| InteractionResponse { plugin_configuration: plugin_configuration.clone(), .. r }),
        response.iter().map(|r| InteractionResponse { plugin_configuration: plugin_configuration.clone(), .. r.clone() }).collect()
      )
    })
}

/// Constructs an interaction for the given Protobuf service descriptor
fn construct_protobuf_interaction_for_service(
  descriptor: &ServiceDescriptorProto,
  config: &BTreeMap<String, prost_types::Value>,
  service_name: &str,
  method_name: &str,
  all_descriptors: &HashMap<String, &FileDescriptorProto>,
  file_descriptor: &FileDescriptorProto
) -> anyhow::Result<(Option<InteractionResponse>, Vec<InteractionResponse>)> {
  trace!(">> construct_protobuf_interaction_for_service({config:?}, {service_name}, {method_name})");

  let (method_name, service_part) = if method_name.contains(':') {
    method_name.split_once(':').unwrap_or((method_name, ""))
  } else {
    (method_name, "")
  };
  trace!(method_name, service_part, "looking up method descriptor");
  let method_descriptor = descriptor.method.iter()
    .find(|m| m.name.clone().unwrap_or_default() == method_name)
    .ok_or_else(|| anyhow!("Did not find a method descriptor for method '{}' in service '{}'", method_name, service_name))?;

  let input_name = method_descriptor.input_type.as_ref().ok_or_else(|| anyhow!("Input message name is empty for service {}/{}", service_name, method_name))?;
  let output_name = method_descriptor.output_type.as_ref().ok_or_else(|| anyhow!("Input message name is empty for service {}/{}", service_name, method_name))?;
  let input_message_name = last_name(input_name.as_str());
  let output_message_name = last_name(output_name.as_str());

  trace!(input_name = input_name.as_str(), input_message_name, "Input message");
  trace!(output_name = output_name.as_str(), output_message_name, "Output message");

  let request_descriptor = find_message_descriptor(input_message_name, all_descriptors)?;
  let response_descriptor = find_message_descriptor(output_message_name, all_descriptors)?;

  let request_part_config = request_part(config, service_part)?;
  trace!(config = ?request_part_config, service_part, "Processing request part config");
  let request_metadata = process_metadata(config.get("requestMetadata"))?;
  let interaction = construct_protobuf_interaction_for_message(&request_descriptor,
    &request_part_config, input_message_name, "", file_descriptor, all_descriptors,
    request_metadata.as_ref()
  )?;
  let request_part = Some(InteractionResponse {
    part_name: "request".into(),
    .. interaction
  });

  let response_part_config = response_part(config, service_part)?;
  trace!(config = ?response_part_config, service_part, "Processing response part config");
  let mut response_part = vec![];
  for (config, md_config) in response_part_config {
    let response_metadata = process_metadata(md_config)?;
    let interaction = construct_protobuf_interaction_for_message(
      &response_descriptor, &config, output_message_name, "",
      file_descriptor, all_descriptors, response_metadata.as_ref()
    )?;
    response_part.push(InteractionResponse { part_name: "response".into(), .. interaction });
  }

  Ok((request_part, response_part))
}

fn response_part<'a>(
  config: &'a BTreeMap<String, prost_types::Value>,
  service_part: &str
) -> anyhow::Result<Vec<(BTreeMap<String, prost_types::Value>, Option<&'a prost_types::Value>)>> {
  trace!(?config, ?service_part, "response_part");
  if service_part == "response" {
    Ok(vec![(config.clone(), None)])
  } else if let Some(response_config) = config.get("response") {
    Ok(response_config.kind.as_ref()
      .map(|kind| {
        match kind {
          Kind::StructValue(s) => {
            let metadata = config.get("responseMetadata");
            vec![(s.fields.clone(), metadata)]
          },
          Kind::ListValue(l) => l.values.iter().filter_map(|v| {
            v.kind.as_ref().and_then(|k| match k {
              Kind::StructValue(s) => Some((s.fields.clone(), None)),
              Kind::StringValue(_) => Some((btreemap! { "value".to_string() => v.clone() }, None)),
              _ => None
            })
          })
            .collect(),
          Kind::StringValue(_) => vec![(btreemap! { "value".to_string() => response_config.clone() }, None)],
          _ => vec![]
        }
      }).unwrap_or_default())
  } else if let Some(response_md_config) = config.get("responseMetadata") {
    Ok(vec![(btreemap!{}, Some(response_md_config))])
  } else {
    Ok(vec![])
  }
}

fn request_part(
  config: &BTreeMap<String, prost_types::Value>,
  service_part: &str
) -> anyhow::Result<BTreeMap<String, prost_types::Value>> {
  if service_part == "request" {
    Ok(config.clone())
  } else {
    let config = config.get("request").and_then(|request_config| {
      request_config.kind.as_ref().map(|kind| {
        match kind {
          Kind::StructValue(s) => Ok(s.fields.clone()),
          Kind::StringValue(_) => Ok(btreemap!{ "value".to_string() => request_config.clone() }),
          _ => {
            warn!("Request contents is of an un-processable type: {:?}", kind);
            Err(anyhow!("Request contents is of an un-processable type: {:?}, it should be either a Struct or a StringValue", kind))
          }
        }
      })
    });
    match config {
      None => Ok(btreemap!{}),
      Some(result) => result
    }
  }
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
  config: &BTreeMap<String, prost_types::Value>,
  descriptor: &FileDescriptorProto,
  descriptor_hash: &str,
  all_descriptors: &HashMap<String, &FileDescriptorProto>
) -> anyhow::Result<InteractionResponse> {
  trace!(">> configure_protobuf_message({}, {:?})", message_name, descriptor_hash);
  debug!("Looking for message of type '{}'", message_name);
  let message_descriptor = descriptor.message_type
    .iter().find(|p| p.name.clone().unwrap_or_default() == message_name)
    .ok_or_else(|| anyhow!("Did not find a descriptor for message '{}'", message_name))?;
  construct_protobuf_interaction_for_message(message_descriptor, config, message_name, "", descriptor, all_descriptors, None)
    .map(|interaction| {
      InteractionResponse {
        plugin_configuration: Some(PluginConfiguration {
          interaction_configuration: Some(to_proto_struct(&hashmap!{
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
#[instrument(ret, skip(message_descriptor, file_descriptor, all_descriptors))]
fn construct_protobuf_interaction_for_message(
  message_descriptor: &DescriptorProto,
  config: &BTreeMap<String, prost_types::Value>,
  message_name: &str,
  message_part: &str,
  file_descriptor: &FileDescriptorProto,
  all_descriptors: &HashMap<String, &FileDescriptorProto>,
  metadata: Option<&MessageMetadata>
) -> anyhow::Result<InteractionResponse> {
  trace!(">> construct_protobuf_interaction_for_message({}, {}, {:?}, {:?}, {:?})", message_name,
    message_part, file_descriptor.name, config.keys(), metadata);

  let mut message_builder = MessageBuilder::new(message_descriptor, message_name, file_descriptor);
  let mut matching_rules = MatchingRuleCategory::empty("body");
  let mut generators = hashmap!{};

  debug!("Building message {} from Protobuf descriptor", message_name);
  let mut path = DocPath::root();
  if !message_part.is_empty() {
    path.push_field(message_part);
  }

  for (key, value) in config {
    if !key.starts_with("pact:") {
      let field_path = path.join(key);
      debug!(?field_path, "Building field for key '{}'", key);
      construct_message_field(&mut message_builder, &mut matching_rules, &mut generators,
        key, &proto_value_to_json(value), &field_path, all_descriptors)?;
    }
  }

  debug!("Constructing response to return");
  trace!("Final message builder: {:?}", message_builder);
  trace!("matching rules: {:?}", matching_rules);
  trace!("generators: {:?}", generators);

  let rules = extract_rules(&matching_rules);
  let generators = extract_generators(&generators);

  let content_type = format!("application/protobuf;message={}", message_name);
  let mut metadata_fields = btreemap! {
    "contentType".to_string() => prost_string(&content_type)
  };
  if let Some(metadata) = metadata {
    for (k, v) in &metadata.values {
      metadata_fields.insert(k.clone(), prost_string(v));
    }
  }

  Ok(InteractionResponse {
    contents: Some(Body {
      content_type: content_type.clone(),
      content: Some(message_builder.encode_message()?.to_vec()),
      content_type_hint: ContentTypeHint::Binary as i32,
    }),
    message_metadata: Some(Struct {
      fields: metadata_fields
    }),
    rules,
    generators,
    interaction_markup: message_builder.generate_markup("")?,
    interaction_markup_type: MarkupType::CommonMark as i32,
    part_name: message_part.to_string(),
    metadata_rules: metadata.map(|md| extract_rules(&md.matching_rules)).unwrap_or_default(),
    metadata_generators: metadata.map(|md| extract_generators(&md.generators)).unwrap_or_default(),
    .. InteractionResponse::default()
  })
}

fn extract_generators(generators: &HashMap<String, Generator>) -> HashMap<String, pact_plugin_driver::proto::Generator> {
  generators.iter().map(|(path, generator)| {
    let gen_values = generator.values();
    let values = if gen_values.is_empty() {
      None
    } else {
      Some(to_proto_struct(&gen_values.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()))
    };
    (path.to_string(), pact_plugin_driver::proto::Generator {
      r#type: generator.name(),
      values
    })
  }).collect()
}

fn extract_rules(matching_rules: &MatchingRuleCategory) -> HashMap<String, MatchingRules> {
  matching_rules.rules.iter().map(|(path, rule_list)| {
    (path.to_string(), MatchingRules {
      rule: rule_list.rules.iter().map(|rule| {
        let rule_values = rule.values();
        let values = if rule_values.is_empty() {
          None
        } else {
          Some(to_proto_struct(&rule_values.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()))
        };
        MatchingRule {
          r#type: rule.name(),
          values
        }
      }).collect()
    })
  }).collect()
}

/// Construct a single field for a message from the provided config
#[tracing::instrument(ret,
  skip_all,
  fields(%path, %field_name, %value)
)]
fn construct_message_field(
  message_builder: &mut MessageBuilder,
  matching_rules: &mut MatchingRuleCategory,
  generators: &mut HashMap<String, Generator>,
  field_name: &str,
  value: &Value,
  path: &DocPath,
  all_descriptors: &HashMap<String, &FileDescriptorProto>
) -> anyhow::Result<()> {
  if !field_name.starts_with("pact:") {
    if let Some(field) = message_builder.field_by_name(field_name)  {
      trace!(?field_name, descriptor = ?field, "Found a descriptor for field");
      match field.r#type {
        Some(r#type) => if r#type == Type::Message as i32 {
          // Embedded message
          trace!(?field_name, "Field is for an embedded message");
          build_embedded_message_field_value(message_builder, path, &field, field_name,
            value, matching_rules, generators, all_descriptors)?;
        } else {
          // Non-embedded message field (singular value)
          trace!(?field_name, "Field is not an embedded message");
          let field_type = if is_repeated_field(&field) {
            MessageFieldValueType::Repeated
          } else {
            MessageFieldValueType::Normal
          };
          build_field_value(path, message_builder, field_type, &field, field_name, value,
                            matching_rules, generators, all_descriptors)?;
        }
        None => {
          return Err(anyhow!("Message {} field '{}' is of an unknown type", message_builder.message_name, field_name))
        }
      }
    } else {
      error!("Field '{}' was not found in message '{}'", field_name, message_builder.message_name);
      let fields: HashSet<String> = message_builder.descriptor.field.iter()
        .map(|field| field.name.clone().unwrap_or_default())
        .collect();
      return Err(anyhow!("Message {} has no field '{}'. Fields are {:?}", message_builder.message_name, field_name, fields))
    }
  }
  Ok(())
}

/// Constructs the field value for a field in a message.
#[tracing::instrument(ret,
  skip_all,
  fields(%path, ?field, %value)
)]
fn build_embedded_message_field_value(
  message_builder: &mut MessageBuilder,
  path: &DocPath,
  field_descriptor: &FieldDescriptorProto,
  field: &str,
  value: &Value,
  matching_rules: &mut MatchingRuleCategory,
  generators: &mut HashMap<String, Generator>,
  all_descriptors: &HashMap<String, &FileDescriptorProto>
) -> anyhow::Result<()> {
  if is_repeated_field(field_descriptor) && !is_map_field(&message_builder.descriptor, field_descriptor) {
    debug!("{} is a repeated field", field);

    match value {
      Value::Array(list) => {
        // We have been provided an array of values, so we use the first one to build the type
        // information, and then just process the remaining values as additional array items
        if let Some((first, rest)) = list.split_first() {
          let index_path = path.join("0");
          build_single_embedded_field_value(
            &index_path, message_builder, MessageFieldValueType::Repeated, field_descriptor,
            field, first, matching_rules, generators, all_descriptors)?;
          let mut builder = message_builder.clone();
          for (index, item) in rest.iter().enumerate() {
            let index_path = path.join((index + 1).to_string());
            let constructed = build_single_embedded_field_value(
              &index_path, &mut builder, MessageFieldValueType::Repeated,
              field_descriptor, field, item, matching_rules, generators, all_descriptors
            )?;
            if let Some(constructed) = constructed {
              message_builder.add_repeated_field_value(field_descriptor, field, constructed);
            }
          }
        }
        Ok(())
      }
      Value::Object(map) => {
        if let Some(definition) = map.get("pact:match") {
          // We have received a map to configure the repeated field with a match value, so we
          // process the rest of the map as a single example value applied against the pact:match
          // expression. Normally it should be a matchValues or matchKeys (or both)
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
            debug!("Found each value matcher");
            if mrd.rules.len() > 1 {
              warn!("{}: each value matcher can not be combined with other matchers, ignoring the other matching rules", path);
            }

            match each_value_def.rules.first() {
              Some(either) => match either {
                Either::Left(_) => {
                  matching_rules.add_rule(path.clone(), matchingrules::MatchingRule::EachValue(each_value_def.clone()), RuleLogic::And);
                  if let Some(generator) = &each_value_def.generator {
                    generators.insert(path.to_string(), generator.clone());
                  }
                  let constructed_value = value_for_type(field, each_value_def.value.as_str(),
                    field_descriptor, &message_builder.descriptor, all_descriptors)?;
                  message_builder.set_field_value(field_descriptor, field, constructed_value);
                  Ok(())
                }
                Either::Right(reference) => if let Some(field_value) = map.get(reference.name.as_str()) {
                  matching_rules.add_rule(path.clone(), matchingrules::MatchingRule::Values, RuleLogic::And);
                  let array_path = path.join("*");
                  matching_rules.add_rule(array_path.clone(), matchingrules::MatchingRule::Type, RuleLogic::And);
                  build_single_embedded_field_value(&array_path, message_builder, MessageFieldValueType::Repeated,
                                                    field_descriptor, field, field_value, matching_rules, generators, all_descriptors)
                    .map(|_| ())
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

            let constructed = value_for_type(field, mrd.value.as_str(),
              field_descriptor, &message_builder.descriptor, all_descriptors)?;
            message_builder.add_repeated_field_value(field_descriptor, field, constructed);

            Ok(())
          }
        } else {
          // No matching definition, so we have to assume the map contains the attributes of a
          // single example.
          trace!("No matching definition, assuming config contains the attributes of a single example");
          build_single_embedded_field_value(&path.join("*"), message_builder, MessageFieldValueType::Repeated, field_descriptor, field, value,
                                            matching_rules, generators, all_descriptors)
            .map(|_| ())
        }
      }
      _ => {
        // Not a map or list structure, so could be a primitive repeated field
        trace!("Not a map or list structure, assuming a single field");
        build_single_embedded_field_value(path, message_builder, MessageFieldValueType::Repeated, field_descriptor, field, value,
                                          matching_rules, generators, all_descriptors)
          .map(|_| ())
      }
    }
  } else {
    trace!("processing a standard field");
    build_single_embedded_field_value(path, message_builder, MessageFieldValueType::Normal, field_descriptor, field, value,
                                      matching_rules, generators, all_descriptors)
      .map(|_| ())
  }
}

/// Construct a non-repeated embedded message field
#[tracing::instrument(ret,
  skip_all,
  fields(%path, ?field_type, %field, %value)
)]
fn build_single_embedded_field_value(
  path: &DocPath,
  message_builder: &mut MessageBuilder,
  field_type: MessageFieldValueType,
  field_descriptor: &FieldDescriptorProto,
  field: &str,
  value: &Value,
  matching_rules: &mut MatchingRuleCategory,
  generators: &mut HashMap<String, Generator>,
  all_descriptors: &HashMap<String, &FileDescriptorProto>
) -> anyhow::Result<Option<MessageFieldValue>> {
  debug!("Configuring message field '{}' (type {:?})", field, field_descriptor.type_name);
  let type_name = field_descriptor.type_name.clone().unwrap_or_default();
  match type_name.as_str() {
    ".google.protobuf.BytesValue" => {
      debug!("Field is a Protobuf BytesValue");
      if let Value::String(_) = value {
        build_field_value(path, message_builder, field_type, field_descriptor, field,
                          value, matching_rules, generators, all_descriptors)
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
      build_map_field(path, message_builder, field_descriptor, field, value, matching_rules, generators, all_descriptors)?;
      Ok(None)
    } else if let Value::Object(config) = value {
      debug!("Configuring the message from config {:?}", config);
      let message_name = last_name(type_name.as_str());
      let embedded_type = find_nested_type(&message_builder.descriptor, field_descriptor)
        .or_else(|| find_message_type_in_file_descriptors(message_name, &message_builder.file_descriptor, all_descriptors).ok())
        .ok_or_else(|| anyhow!("Did not find message '{}' in the current message or in the file descriptors", type_name))?;
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
            construct_message_field(&mut embedded_builder, matching_rules, generators,
              key, value, &field_path, all_descriptors)?;
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
        let proto_value = build_proto_value(&field_path, value, matching_rules, generators)?;
        fields.insert(key.clone(), proto_value);
      }

      let s = Struct { fields };
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
        let proto_value = build_proto_value(&field_path, value, matching_rules, generators)?;
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
  generators: &mut HashMap<String, Generator>,
  all_descriptors: &HashMap<String, &FileDescriptorProto>
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
        trace!("Found matching rules: {:?}", mrd.rules);
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
            key_descriptor, "key", &Value::String(inner_field.clone()),
            matching_rules, generators, all_descriptors
          )?
            .ok_or_else(|| anyhow!("Was not able to construct map key value {:?}", key_descriptor.type_name))?;
          let value_value = build_single_embedded_field_value(&entry_path, &mut embedded_builder, MessageFieldValueType::Normal,
            value_descriptor, "value", value, matching_rules, generators, all_descriptors)?
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
#[tracing::instrument(ret,
  skip_all,
  fields(%path, ?field_type, %field_name, %value)
)]
fn build_field_value(
  path: &DocPath,
  message_builder: &mut MessageBuilder,
  field_type: MessageFieldValueType,
  descriptor: &FieldDescriptorProto,
  field_name: &str,
  value: &Value,
  matching_rules: &mut MatchingRuleCategory,
  generators: &mut HashMap<String, Generator>,
  all_descriptors: &HashMap<String, &FileDescriptorProto>
) -> anyhow::Result<Option<MessageFieldValue>> {
  trace!(">> build_field_value({}, {}, {:?})", path, field_name, value);

  match value {
    Value::Null => Ok(None),
    Value::String(s) => {
      let constructed_value = match field_type {
        MessageFieldValueType::Repeated => {
          let path = path.join("*");
          let constructed_value = construct_value_from_string(&path, message_builder,
            descriptor, field_name, matching_rules, generators, s, all_descriptors)?;
          debug!("Setting field {:?}:repeated to value {:?}", field_name, constructed_value);
          message_builder.add_repeated_field_value(descriptor, field_name, constructed_value.clone());
          constructed_value
        },
        _ => {
          let constructed_value = construct_value_from_string(path, message_builder,
            descriptor, field_name, matching_rules, generators, s, all_descriptors)?;
          debug!("Setting field {:?}:{:?} to value {:?}", field_name, field_type, constructed_value);
          message_builder.set_field_value(descriptor, field_name, constructed_value.clone());
          constructed_value
        },
      };
      Ok(Some(constructed_value))
    }
    Value::Array(list) => {
      if let Some((first, rest)) = list.split_first() {
        let index_path = path.join("0");
        let constructed_value = build_field_value(&index_path, message_builder,
          MessageFieldValueType::Repeated, descriptor, field_name, first,
          matching_rules, generators, all_descriptors
        )?;
        for (index, value) in rest.iter().enumerate() {
          let index_path = path.join((index + 1).to_string());
          build_field_value(&index_path, message_builder, MessageFieldValueType::Repeated,
            descriptor, field_name, value, matching_rules, generators, all_descriptors
          )?;
        }
        trace!(?message_builder, "Constructed repeated field from array");
        Ok(constructed_value)
      } else {
        Ok(None)
      }
    }
    _ => Err(anyhow!("Field values must be configured with a string value, got {:?}", value))
  }
}

fn construct_value_from_string(
  path: &DocPath,
  message_builder: &mut MessageBuilder,
  descriptor: &FieldDescriptorProto,
  field_name: &str,
  matching_rules: &mut MatchingRuleCategory,
  generators: &mut HashMap<String, Generator>,
  s: &str,
  all_descriptors: &HashMap<String, &FileDescriptorProto>
) -> anyhow::Result<MessageFieldValue> {
  trace!(?field_name, string = ?s, "Building value from string");
  if is_matcher_def(s) {
    trace!("String value is a matcher definition");
    let mrd = parse_matcher_def(s)?;
    trace!("matcher definition = {:?}", mrd);
    if !mrd.rules.is_empty() {
      for rule in &mrd.rules {
        match rule {
          Either::Left(rule) => {
            let path = if rule.is_values_matcher() && path.is_wildcard() {
              path.parent().unwrap_or(DocPath::root())
            } else {
              path.clone()
            };
            matching_rules.add_rule(path, rule.clone(), RuleLogic::And)
          },
          Either::Right(mr) => return Err(anyhow!("Was expecting a value for '{}', but got a matching reference {:?}", path, mr))
        }
      }
    }
    if let Some(generator) = &mrd.generator {
      generators.insert(path.to_string(), generator.clone());
    }
    value_for_type(field_name, &*value_for_field(&mrd), descriptor, &message_builder.descriptor,
                   all_descriptors)
  } else {
    value_for_type(field_name, s, descriptor, &message_builder.descriptor,
      all_descriptors)
  }
}

fn value_for_field(mrd: &MatchingRuleDefinition) -> String {
  if mrd.value.is_empty() {
    if let Some(value_matcher) = mrd.rules.iter().find_map(|m| {
      match m {
        Either::Left(mr) => match mr {
          matchingrules::MatchingRule::EachValue(def) => Some(def.value.clone()),
          _ => None
        }
        Either::Right(_) => None
      }
    }) {
      value_matcher
    } else {
      String::default()
    }
  } else {
    mrd.value.clone()
  }
}

fn value_for_type(
  field_name: &str,
  field_value: &str,
  descriptor: &FieldDescriptorProto,
  message_descriptor: &DescriptorProto,
  all_descriptors: &HashMap<String, &FileDescriptorProto>
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
    Type::Enum => {
      let result = find_enum_value_by_name_in_message(&message_descriptor.enum_type, type_name.as_str(), field_value)
        .or_else(|| find_enum_value_by_name(all_descriptors, type_name.as_str(), field_value));
      if let Some((n, desc)) = result {
        Ok(MessageFieldValue {
          name: field_name.to_string(),
          raw_value: Some(field_value.to_string()),
          rtype: RType::Enum(n, desc)
        })
      } else {
        Err(anyhow!("Protobuf enum value {} has no value {}", type_name, field_value))
      }
    }
    _ => Err(anyhow!("Protobuf field {} has an unsupported type {:?}", field_name, t))
  }
}

#[cfg(test)]
pub(crate) mod tests {
  use std::collections::HashMap;

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
    build_field_value,
    build_single_embedded_field_value,
    construct_message_field,
    construct_protobuf_interaction_for_message,
    construct_protobuf_interaction_for_service,
    request_part,
    response_part,
    value_for_type
  };
  use crate::utils::find_message_type_by_name;

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
    let result = value_for_type("test", "test", &descriptor, &message_descriptor, &hashmap!{}).unwrap();
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
    let result = value_for_type("test", "100", &descriptor, &message_descriptor, &hashmap!{}).unwrap();
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

    let result = construct_protobuf_interaction_for_message(&message_descriptor, &config,
      "test_message", "", &file_descriptor, &hashmap!{}, None).unwrap();

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
    let all_descriptors = hashmap!{ "simple.proto".to_string() => fs };
    let config = btreemap! {
      "value".to_string() => prost_types::Value { kind: Some(prost_types::value::Kind::StringValue("eachValue(matching(type, '00000000000000000000000000000000'))".to_string())) }
    };
    let (message_descriptor, _) = find_message_type_by_name("ValuesMessageIn", &fds).unwrap();

    let result = construct_protobuf_interaction_for_message(
      &message_descriptor,
      &config,
      "ValuesMessageIn",
      "",
      fs,
      &all_descriptors,
      None
    ).unwrap();

    let body = result.contents.as_ref().unwrap();
    expect!(body.content_type.as_str()).to(be_equal_to("application/protobuf;message=ValuesMessageIn"));
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
    let (message_descriptor, _) = find_message_type_by_name("ValuesMessageIn", &fds).unwrap();
    let mut message_builder = MessageBuilder::new(&message_descriptor, "ValuesMessageIn", fs);
    let path = DocPath::new("$.value").unwrap();
    let mut matching_rules = MatchingRuleCategory::empty("body");
    let mut generators = hashmap!{};
    let file_descriptors: HashMap<String, &FileDescriptorProto> = fds.file
      .iter().map(|des| (des.name.clone().unwrap_or_default(), des))
      .collect();

    let result = construct_message_field(&mut message_builder, &mut matching_rules,
      &mut generators, "value", &Value::String("eachValue(matching(type, '00000000000000000000000000000000'))".to_string()),
      &path, &file_descriptors);
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
            ValueType::Unknown, pact_models::matchingrules::MatchingRule::Type, None)
        )
      ]
    }));
  }

  #[test_log::test]
  fn build_field_value_with_message_with_each_value_matcher() {
    let fds = FileDescriptorSet::decode(DESCRIPTORS_FOR_EACH_VALUE_TEST.as_slice()).unwrap();
    let fs = fds.file.first().unwrap();
    let (message_descriptor, _) = find_message_type_by_name("ValuesMessageIn", &fds).unwrap();
    let field_descriptor = message_descriptor.field.first().unwrap();
    let mut message_builder = MessageBuilder::new(&message_descriptor, "ValuesMessageIn", fs);
    let path = DocPath::new("$.value").unwrap();
    let mut matching_rules = MatchingRuleCategory::empty("body");
    let mut generators = hashmap!{};
    let file_descriptors: HashMap<String, &FileDescriptorProto> = fds.file
      .iter().map(|des| (des.name.clone().unwrap_or_default(), des))
      .collect();

    let result = build_field_value(
      &path, &mut message_builder, MessageFieldValueType::Repeated, field_descriptor,
      "value", &Value::String("eachValue(matching(type, '00000000000000000000000000000000'))".to_string()),
      &mut matching_rules, &mut generators, &file_descriptors
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
    let file_descriptor = FileDescriptorProto {
      name: None,
      package: None,
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
          input_type: Some(".google.protobuf.StringValue".to_string()),
          output_type: Some("test_message".to_string()),
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

    let result = construct_protobuf_interaction_for_service(&service_descriptor, &config,
      "test_service", "call", &hashmap!{ "file".to_string() => &file_descriptor }, &file_descriptor);
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
      name: None,
      package: None,
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
          input_type: Some(".google.protobuf.StringValue".to_string()),
          output_type: Some("test_message".to_string()),
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

    let result = construct_protobuf_interaction_for_service(&service_descriptor, &config,
      "test_service", "call", &hashmap!{ "file".to_string() => &file_descriptor }, &file_descriptor);
    expect!(result).to(be_ok());
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

    let result = build_embedded_message_field_value(&mut message_builder, &path, &field_descriptor,
      "value", &config, &mut matching_rules, &mut generators, &hashmap!{}
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

    let result = build_embedded_message_field_value(&mut message_builder, &path, &field_descriptor,
      "value", &config, &mut matching_rules, &mut generators, &hashmap!{}
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
    let file_descriptors: HashMap<String, &FileDescriptorProto> = fds.file
      .iter().map(|des| (des.name.clone().unwrap_or_default(), des))
      .collect();

    let result = build_embedded_message_field_value(&mut message_builder, &path, field_descriptor,
      "listener_context", &field_config, &mut matching_rules, &mut generators, &file_descriptors
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
    let file_descriptors: HashMap<String, &FileDescriptorProto> = fds.file
      .iter().map(|des| (des.name.clone().unwrap_or_default(), des))
      .collect();

    let result = build_single_embedded_field_value(
      &path, &mut message_builder, MessageFieldValueType::Normal, field_descriptor,
      "listener_context", &field_config, &mut matching_rules, &mut generators, &file_descriptors);
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
    let file_descriptors: HashMap<String, &FileDescriptorProto> = fds.file
      .iter().map(|des| (des.name.clone().unwrap_or_default(), des))
      .collect();

    let result = construct_message_field(&mut message_builder, &mut matching_rules,
      &mut generators, "ad_break_type", &Value::String("AUDIO_AD_BREAK".to_string()),
      &path, &file_descriptors);
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
    let file_descriptors: HashMap<String, &FileDescriptorProto> = fds.file
      .iter().map(|des| (des.name.clone().unwrap_or_default(), des))
      .collect();

    let result = build_single_embedded_field_value(
      &path, &mut message_builder, MessageFieldValueType::Normal, field_descriptor,
      "ad_break_type", &field_config, &mut matching_rules, &mut generators, &file_descriptors);
    expect!(result).to(be_ok());
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
    let file_descriptors: HashMap<String, &FileDescriptorProto> = fds.file
      .iter().map(|des| (des.name.clone().unwrap_or_default(), des))
      .collect();


    let result = build_field_value(&path, &mut message_builder,
      MessageFieldValueType::Normal, field_descriptor, "result", &field_config,
      &mut matching_rules, &mut generators, &file_descriptors
    );
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
}
