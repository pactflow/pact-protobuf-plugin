//! Module for processing and comparing protobuf messages

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use anyhow::anyhow;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use itertools::{Either, Itertools};
use maplit::{btreemap, hashmap};
use num::ToPrimitive;
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
  to_fully_qualified_name,
  find_enum_value_by_name_in_message,
  find_nested_type,
  is_map_field,
  is_repeated_field,
  last_name,
  prost_string,
  split_service_and_method,
  DescriptorCache
};

/// Converts user-provided configuration and .proto files into a pact interaction.
/// 
/// # Arguments
/// 
/// - `proto_file` - Path to the protobuf file
/// - `protoc` - Encapsulates protoc functionality; can parse protos into descriptors
/// - `config` - Test configuration as provided by the test author, e.g.
/// ```json
/// {
///   "pact:proto": "/path/to/protos/route/route_guide.proto",
///   "pact:proto-service": "RouteGuide/GetFeature",
///   "pact:content-type": "application/protobuf",
///   "pact:protobuf-config": {
///       "additionalIncludes": ["/path/to/protos/"]
///   },
///   "request": {
///       "latitude": "matching(number, 180)",
///       "longitude": "matching(number, 200)"
///   },
///   "response": {
///       "name": "notEmpty('Big Tree')",
///   }
/// }
/// ```
/// 
/// # Returns
/// 
/// A tuple of values to construct the pact file:
/// - Vector of interactions - single for a message interaction, or a request/response pair for a grpc interaction
/// - Plugin configuration, which can be used to store the protobuf file and descriptors
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

  let descriptor_cache = DescriptorCache::new(descriptors.clone());
  let file_name = &*proto_file.file_name().unwrap_or_default().to_string_lossy();
  let descriptor = match descriptor_cache.get_file_descriptor_by_name(file_name) {
    Some(des) => des,
    None => return Err(anyhow!("Did not find a file proto descriptor for the provided proto file '{}'", file_name)),
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
      descriptor_hash.as_str(), &descriptor_cache)?;
    interactions.push(result);
  } else if let Some(service_name) = config.get("pact:proto-service") {
    let service_name = proto_value_to_string(service_name)
      .ok_or_else(|| anyhow!("Did not get a valid value for 'pact:proto-service'. It should be a string"))?;
    debug!("Configuring a Protobuf service {}", service_name);
    let (request_part, response_part) = configure_protobuf_service(service_name.as_str(), config, descriptor,
      &descriptor_cache, descriptor_hash.as_str())?;
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

/// Configure the interaction for a gRPC service method, which has an input and output message.
/// Main work is done in `construct_protobuf_interaction_for_service`;
/// this function does two things:
/// - locates the correct service descriptor in the provided file descriptor
/// - adds interaction_configuration to the output of `construct_protobuf_interaction_for_service`, which contains:
///   - service: the fully qualified service name; allows to locate this service when verifying this interaction
///   - descriptorKey: a hash of the protobuf file descriptor, which allows to locate the file descriptor 
/// in the plugin configuration when verifying this interaction
fn configure_protobuf_service(
  service_with_method: &str,
  config: &BTreeMap<String, prost_types::Value>,
  descriptor: &FileDescriptorProto,
  descriptor_cache: &DescriptorCache,
  descriptor_hash: &str
) -> anyhow::Result<(Option<InteractionResponse>, Vec<InteractionResponse>)> {
  trace!(">> configure_protobuf_service({service_with_method}, {config:?}, {descriptor_hash})");

  debug!("Looking for service and method with name '{}'", service_with_method);
  let (service, method_name) = split_service_and_method(service_with_method)?;
  // Lookup service inside the descriptor, but don't search all file descriptors to avoid similarly named services
  let service_descriptor = descriptor.service
    .iter().find(|p| p.name() == service)
    .ok_or_else(|| anyhow!("Did not find a descriptor for service '{}'", service_with_method))?;
  trace!("service_descriptor = {:?}", service_descriptor);
  
  let service_with_method = service_with_method.split_once(':').map(|(s, _)| s).unwrap_or(service_with_method);
  let service_full_name = to_fully_qualified_name(service_with_method, descriptor.package())?;
  construct_protobuf_interaction_for_service(service_descriptor, config, method_name, descriptor_cache)
    .map(|(request, response)| {
      let plugin_configuration = Some(PluginConfiguration {
        interaction_configuration: Some(Struct {
          fields: btreemap!{
            "service".to_string() => prost_string(service_full_name),
            "descriptorKey".to_string() => prost_string(descriptor_hash.to_string()),
            "expectations".to_string() => prost_types::Value {
              kind: Some(Kind::StructValue(Struct {
                fields: config.iter()
                  .filter(|(key, _)| !key.starts_with("pact:"))
                  .map(|(k, v)| (k.clone(), v.clone()))
                  .collect()
              }))
            }
          },
        }),
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

/// Constructs an interaction for the given gRPC service descriptor
/// Interaction consists of request intraction and possibly multiple response interactions,
/// each is constructed by calling `construct_protobuf_interaction_for_message`.
/// Request and response types are looked up in all of the provided file descriptors using their
/// fully qualified names.
fn construct_protobuf_interaction_for_service(
  service_descriptor: &ServiceDescriptorProto,
  config: &BTreeMap<String, prost_types::Value>,
  method_name: &str,
  descriptor_cache: &DescriptorCache
) -> anyhow::Result<(Option<InteractionResponse>, Vec<InteractionResponse>)> {
  let service_name = service_descriptor.name.as_deref().expect("Service descriptor name cannot be empty");
  trace!(">> construct_protobuf_interaction_for_service({config:?}, {service_name}, {method_name})");

  let (method_name, service_part) = if method_name.contains(':') {
    method_name.split_once(':').unwrap_or((method_name, ""))
  } else {
    (method_name, "")
  };
  trace!(method_name, service_part, "looking up method descriptor");
  let method_descriptor = service_descriptor.method.iter()
    .find(|m| m.name.clone().unwrap_or_default() == method_name)
    .ok_or_else(|| anyhow!("Did not find a method descriptor for method '{}' in service '{}'", method_name, service_name))?;

  let input_name = method_descriptor.input_type.as_ref()
    .ok_or_else(|| anyhow!("Input message name is empty for service {}/{}", service_name, method_name))?;
  let output_name = method_descriptor.output_type.as_ref()
    .ok_or_else(|| anyhow!("Output message name is empty for service {}/{}", service_name, method_name))?;
  
  let (request_descriptor, request_file_descriptor) = 
    descriptor_cache.find_message_descriptor_for_type(input_name)?;
  let (response_descriptor, response_file_descriptor) = 
    descriptor_cache.find_message_descriptor_for_type(output_name)?;
  
  trace!(%input_name, ?request_descriptor, ?request_file_descriptor, "Input message descriptor");
  trace!(%output_name, ?response_descriptor, ?response_file_descriptor, "Output message descriptor");
  
  let request_part_config = request_part(config, service_part)?;
  trace!(config = ?request_part_config, service_part, "Processing request part config");
  let request_metadata = process_metadata(config.get("requestMetadata"))?;

  let interaction = construct_protobuf_interaction_for_message(&request_descriptor,
    &request_part_config, "", &request_file_descriptor, descriptor_cache, request_metadata.as_ref())?;
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
      &response_descriptor, &config, "", &response_file_descriptor, descriptor_cache, response_metadata.as_ref())?;
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
    config.unwrap_or_else(|| Ok(btreemap! {}))
  }
}

/// Configure the interaction for a single Protobuf message
fn configure_protobuf_message(
  message_name: &str,
  config: &BTreeMap<String, prost_types::Value>,
  descriptor: &FileDescriptorProto,
  descriptor_hash: &str,
  descriptor_cache: &DescriptorCache
) -> anyhow::Result<InteractionResponse> {
  trace!(">> configure_protobuf_message({}, {:?})", message_name, descriptor_hash);
  debug!("Looking for message '{}' in '{}'", message_name, descriptor.name());
  let message_descriptor = descriptor.message_type
    .iter()
    .find(|p| p.name() == message_name)
    .ok_or_else(|| anyhow!("Did not find a descriptor for message '{}' in '{}'", message_name, descriptor.name()))?;
  let message_full_name = to_fully_qualified_name(message_name, descriptor.package())?;
  construct_protobuf_interaction_for_message(message_descriptor, config, "", descriptor, descriptor_cache, None)
    .map(|interaction| {
      InteractionResponse {
        plugin_configuration: Some(PluginConfiguration {
          interaction_configuration: Some(to_proto_struct(&hashmap!{
            "message".to_string() => Value::String(message_full_name),
            "descriptorKey".to_string() => Value::String(descriptor_hash.to_string())
          })),
          pact_configuration: None
        }),
        .. interaction
      }
    })
}

/// Constructs an interaction for the given Protobuf message descriptor.
/// Used in both message pacts and gRPC service pacts.
/// 
/// # Arguments
/// 
/// - `message_descriptor` - Descriptor of the message to construct the interaction for
/// - `config` - Test configuration as provided by the test author. For request and response messages in gRPC
/// interaction, this will only contain the value of `request` or `response` fields in the original configuration.
/// For message pacts, this is a full configuration object (like in `process_proto` example)
/// - `message_part` - always empty string for now
/// - `file_descriptor` - Descriptor of the file containing the message
/// - `descriptor_cache` - Cached descriptor lookup structure
/// - `metadata` - Optional metadata for the message; for request and response messages in gRPC interaction
/// it's the values of `requestMetadata` and `responseMetadata` fields; not currently supported for message pacts.
/// 
/// # Returns
/// - InteractionResponse - the constructed interaction
#[instrument(ret, skip(message_descriptor, file_descriptor, descriptor_cache))]
fn construct_protobuf_interaction_for_message(
  message_descriptor: &DescriptorProto,
  config: &BTreeMap<String, prost_types::Value>,
  message_part: &str,
  file_descriptor: &FileDescriptorProto,
  descriptor_cache: &DescriptorCache,
  metadata: Option<&MessageMetadata>
) -> anyhow::Result<InteractionResponse> {
  trace!(">> construct_protobuf_interaction_for_message({}, {:?}, {:?}, {:?})",
    message_part, file_descriptor.name, config.keys(), metadata);
  trace!("message_descriptor = {:?}", message_descriptor);
  
  let message_name = message_descriptor.name.as_ref().expect("Message descriptor name cannot be empty");
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
        key, &proto_value_to_json(value), &field_path, descriptor_cache)?;
    }
  }

  debug!("Constructing response to return");
  trace!("Final message builder: {:?}", message_builder);
  trace!("matching rules: {:?}", matching_rules);
  trace!("generators: {:?}", generators);

  let rules = extract_rules(&matching_rules);
  let generators = extract_generators(&generators);

  // Add a package to the message name. This value is read in:
  // - server::generate_contents_impl()
  // - matching::match_service() - as part of compare_contents flow
  // it is not passed on to the provider under test
  let message_with_package = to_fully_qualified_name(message_name, file_descriptor.package())?;
  let content_type = format!("application/protobuf;message={}", message_with_package);
  let mut metadata_fields = btreemap! {
    "contentType".to_string() => prost_string(&content_type)
  };
  if let Some(metadata) = metadata {
    for (k, v) in &metadata.values {
      metadata_fields.insert(k.clone(), prost_string(&v.value));
    }
  }

  let expectations = Struct {
    fields: btreemap!{
      "expectations".to_string() => prost_types::Value {
        kind: Some(Kind::StructValue(Struct {
          fields: config.iter()
            .filter(|(key, _)| !key.starts_with("pact:"))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
        }))
      }
    }
  };
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
    plugin_configuration: Some(PluginConfiguration {
      interaction_configuration: Some(expectations),
      pact_configuration: None
    }),
    .. InteractionResponse::default()
  })
}

fn extract_generators(generators: &HashMap<String, Generator>) -> HashMap<String, pact_plugin_driver::proto::Generator> {
  generators.iter().filter_map(|(path, generator)| {
    let gen_values = generator.values();
    let values = if gen_values.is_empty() {
      None
    } else {
      Some(to_proto_struct(&gen_values.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()))
    };
    Some((
      path.to_string(),
      pact_plugin_driver::proto::Generator {
        r#type: generator.name(),
        values
      }
    ))
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
  descriptor_cache: &DescriptorCache
) -> anyhow::Result<()> {
  if !field_name.starts_with("pact:") {
    if let Some(field) = message_builder.field_by_name(field_name)  {
      trace!(?field_name, descriptor = ?field, "Found a descriptor for field");
      match field.r#type {
        Some(r#type) => if r#type == Type::Message as i32 {
          // Embedded message
          trace!(?field_name, "Field is for an embedded message");
          build_embedded_message_field_value(message_builder, path, &field, field_name,
            value, matching_rules, generators, descriptor_cache)?;
        } else {
          // Non-embedded message field (singular value)
          trace!(?field_name, "Field is not an embedded message");
          let field_type = if is_repeated_field(&field) {
            MessageFieldValueType::Repeated
          } else {
            MessageFieldValueType::Normal
          };
          build_field_value(path, message_builder, field_type, &field, field_name, value,
                            matching_rules, generators, descriptor_cache)?;
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
  descriptor_cache: &DescriptorCache
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
            field, first, matching_rules, generators, descriptor_cache)?;
          let mut builder = message_builder.clone();
          for (index, item) in rest.iter().enumerate() {
            let index_path = path.join((index + 1).to_string());
            let constructed = build_single_embedded_field_value(
              &index_path, &mut builder, MessageFieldValueType::Repeated,
              field_descriptor, field, item, matching_rules, generators, descriptor_cache
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
                    field_descriptor, &message_builder.descriptor, descriptor_cache)?;
                  message_builder.set_field_value(field_descriptor, field, constructed_value);
                  Ok(())
                }
                Either::Right(reference) => if let Some(field_value) = map.get(reference.name.as_str()) {
                  matching_rules.add_rule(path.clone(), matchingrules::MatchingRule::Values, RuleLogic::And);
                  let array_path = path.join("*");
                  matching_rules.add_rule(array_path.clone(), matchingrules::MatchingRule::Type, RuleLogic::And);
                  build_single_embedded_field_value(&array_path, message_builder, MessageFieldValueType::Repeated,
                                                    field_descriptor, field, field_value, matching_rules, generators, descriptor_cache)
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
              field_descriptor, &message_builder.descriptor, descriptor_cache)?;
            message_builder.add_repeated_field_value(field_descriptor, field, constructed);

            Ok(())
          }
        } else {
          // No matching definition, so we have to assume the map contains the attributes of a
          // single example.
          trace!("No matching definition, assuming config contains the attributes of a single example");
          build_single_embedded_field_value(&path.join("*"), message_builder, MessageFieldValueType::Repeated, field_descriptor, field, value,
                                            matching_rules, generators, descriptor_cache)
            .map(|_| ())
        }
      }
      _ => {
        // Not a map or list structure, so could be a primitive repeated field
        trace!("Not a map or list structure, assuming a single field");
        build_single_embedded_field_value(path, message_builder, MessageFieldValueType::Repeated, field_descriptor, field, value,
                                          matching_rules, generators, descriptor_cache)
          .map(|_| ())
      }
    }
  } else {
    trace!("processing a standard field");
    build_single_embedded_field_value(path, message_builder, MessageFieldValueType::Normal, field_descriptor, field, value,
                                      matching_rules, generators, descriptor_cache)
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
  descriptor_cache: &DescriptorCache
) -> anyhow::Result<Option<MessageFieldValue>> {
  debug!("Configuring message field '{}' (type {:?})", field, field_descriptor.type_name);
  let type_name = field_descriptor.type_name.clone().unwrap_or_default();
  match type_name.as_str() {
    ".google.protobuf.BytesValue" => {
      debug!("Field is a Protobuf BytesValue");
      if let Value::String(_) = value {
        build_field_value(path, message_builder, field_type, field_descriptor, field,
                          value, matching_rules, generators, descriptor_cache)
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
      build_map_field(path, message_builder, field_descriptor, field, value, matching_rules, generators, descriptor_cache)?;
      Ok(None)
    } else if let Value::Object(config) = value {
      debug!("Configuring the message from config {:?}", config);
      let embedded_type = find_nested_type(&message_builder.descriptor, field_descriptor)
        .or_else(|| descriptor_cache.find_message_descriptor_for_type(type_name.as_str()).ok().map(|(m, _)| m))
        .ok_or_else(|| anyhow!("Did not find message '{}' in the current message or in the file descriptors", type_name))?;
      let mut embedded_builder = MessageBuilder::new(
        &embedded_type, last_name(type_name.as_str()), &message_builder.file_descriptor);

      let field_value = if let Some(definition) = config.get("pact:match") {
        let mut field_value = None;
        let mrd = parse_matcher_def(json_to_string(definition).as_str())?;
        for rule in &mrd.rules {
          match rule {
            Either::Left(rule) => {
              matching_rules.add_rule(path.clone(), rule.clone(), RuleLogic::And);
            },
            Either::Right(reference) => if let Some(field_def) = config.get(reference.name.as_str()) {
              matching_rules.add_rule(path.clone(), matchingrules::MatchingRule::Values, RuleLogic::And);
              let array_path = path.join("*");
              matching_rules.add_rule(array_path.clone(), matchingrules::MatchingRule::Type, RuleLogic::And);
              field_value = build_single_embedded_field_value(&array_path, message_builder, MessageFieldValueType::Normal,
                                                field_descriptor, field, field_def, matching_rules, generators, descriptor_cache)?;
            } else {
              return Err(anyhow!("Expression '{}' refers to non-existent item '{}'", definition, reference.name));
            }
          }
        }
        if let Some(field_value) = field_value {
          field_value
        } else {
          for (key, value) in config {
            if !key.starts_with("pact:") {
              let field_path = path.join(key);
              construct_message_field(&mut embedded_builder, matching_rules, generators,
                                      key, value, &field_path, descriptor_cache)?;
            }
          }
          MessageFieldValue {
            name: field.to_string(),
            raw_value: None,
            rtype: RType::Message(Box::new(embedded_builder))
          }
        }
      } else {
        for (key, value) in config {
          let field_path = path.join(key);
          construct_message_field(&mut embedded_builder, matching_rules, generators, key, value, &field_path, descriptor_cache)?;
        }
        MessageFieldValue {
          name: field.to_string(),
          raw_value: None,
          rtype: RType::Message(Box::new(embedded_builder))
        }
      };

      message_builder.set_field_value(field_descriptor, field, field_value.clone());
      Ok(Some(field_value))
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
    Value::Object(map) => {
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
      let values = a.iter().enumerate().map(|(index, v)| {
        let index_path = path.join(index.to_string());
        build_proto_value(&index_path, v, matching_rules, generators)
      }).collect_vec();
      if let Some(err) = values.iter().find_map(|v| v.as_ref().err()) {
        return Err(anyhow!("Could not construct a Protobuf list value - {}", err))
      }
      // Unwrap here is safe as the previous statement would catch an error
      let list = prost_types::ListValue { values: values.iter().map(|v| v.as_ref().unwrap().clone()).collect() };
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
  descriptor_cache: &DescriptorCache
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
            Either::Right(mr) => {
              return Err(anyhow!("Was expecting a matching rule definition, but got a reference: {}", mr.name));
            }
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
      trace!("Map field key descriptor = {:?}", key_descriptor);
      trace!("Map field value descriptor = {:?}", value_descriptor);

      let mut embedded_builder = MessageBuilder::new(&map_type, message_name.as_str(), &message_builder.file_descriptor);
      for (inner_field, value) in config {
        if inner_field != "pact:match" {
          let entry_path = path.join(inner_field);

          let key_value = build_field_value(&entry_path, &mut embedded_builder, MessageFieldValueType::Normal,
            key_descriptor, "key", &Value::String(inner_field.clone()),
            matching_rules, generators, descriptor_cache
          )?
            .ok_or_else(|| anyhow!("Was not able to construct map key value {:?}", key_descriptor.type_name))?;

          let value_value = if value_descriptor.r#type() == Type::Message {
            // Embedded message
            trace!("Value is an embedded message type");
            build_single_embedded_field_value(&entry_path, &mut embedded_builder, MessageFieldValueType::Normal,
              value_descriptor, "value", value, matching_rules, generators, descriptor_cache)?
          } else {
            // Non-embedded message field (singular value)
            trace!("Value is not an embedded message");
            build_field_value(&entry_path, &mut embedded_builder, MessageFieldValueType::Normal,
              value_descriptor, "value", value, matching_rules, generators, descriptor_cache)?
          }
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
  descriptor_cache: &DescriptorCache
) -> anyhow::Result<Option<MessageFieldValue>> {
  trace!(">> build_field_value({}, {}, {:?})", path, field_name, value);

  match value {
    Value::Null => Ok(None),

    Value::String(s) => {
      let constructed_value = match field_type {
        MessageFieldValueType::Repeated => {
          let path = path.join("*");
          let constructed_value = construct_value_from_string(&path, message_builder,
            descriptor, field_name, matching_rules, generators, s, descriptor_cache)?;
          debug!("Setting field {:?}:repeated to value {:?}", field_name, constructed_value);
          message_builder.add_repeated_field_value(descriptor, field_name, constructed_value.clone());
          constructed_value
        },
        _ => {
          let constructed_value = construct_value_from_string(path, message_builder,
            descriptor, field_name, matching_rules, generators, s, descriptor_cache)?;
          debug!("Setting field {:?}:{:?} to value {:?}", field_name, field_type, constructed_value);
          message_builder.set_field_value(descriptor, field_name, constructed_value.clone());
          constructed_value
        },
      };
      Ok(Some(constructed_value))
    }

    Value::Array(list) => {
      if descriptor.r#type() == Type::Bytes {
        let constructed_value = construct_bytes_value_from_array(
          field_name, list, value.to_string())?;
        debug!("Setting field {:?}:{:?} to value {:?}", field_name, field_type, constructed_value);
        message_builder.set_field_value(descriptor, field_name, constructed_value.clone());
        Ok(Some(constructed_value))
      } else if field_type == MessageFieldValueType::Repeated {
        if let Some((first, rest)) = list.split_first() {
          let index_path = path.join("0");
          let constructed_value = build_field_value(&index_path, message_builder,
            MessageFieldValueType::Repeated, descriptor, field_name, first,
            matching_rules, generators, descriptor_cache
          )?;
          for (index, value) in rest.iter().enumerate() {
            let index_path = path.join((index + 1).to_string());
            build_field_value(&index_path, message_builder, MessageFieldValueType::Repeated,
              descriptor, field_name, value, matching_rules, generators, descriptor_cache
            )?;
          }
          trace!(?message_builder, "Constructed repeated field from array");
          Ok(constructed_value)
        } else {
          Ok(None)
        }
      } else {
        Err(anyhow!("Only repeated or byte field values can be configured with an array, field {} type is {:?}",
          field_name, descriptor.r#type()))
      }
    }

    Value::Bool(b) => if descriptor.r#type() == Type::Bool {
      let constructed_value = MessageFieldValue {
        name: field_name.to_string(),
        raw_value: Some(b.to_string()),
        rtype: RType::Boolean(*b)
      };
      update_message_builder(message_builder, field_type, descriptor, field_name, &constructed_value);
      Ok(Some(constructed_value))
    } else {
      Err(anyhow!("Only boolean field values can be configured with a boolean value, field {} type is {:?}",
        field_name,
        descriptor.r#type()))
    }

    Value::Number(n) => if n.is_u64() {
      let f = n.as_u64().unwrap_or_default();
      construct_numeric_value(message_builder, field_type, descriptor, field_name, value, f)
    } else if n.is_i64() {
      let f = n.as_i64().unwrap_or_default();
      construct_numeric_value(message_builder, field_type, descriptor, field_name, value, f)
    } else {
      let f = n.as_f64().unwrap_or_default();
      construct_numeric_value(message_builder, field_type, descriptor, field_name, value, f)
    }
    _ => Err(anyhow!("Field values must be configured with a string value, got {:?}", value))
  }
}

fn construct_numeric_value<N: ToPrimitive>(
  message_builder: &mut MessageBuilder,
  field_type: MessageFieldValueType,
  descriptor: &FieldDescriptorProto,
  field_name: &str,
  value: &Value,
  f: N
) -> anyhow::Result<Option<MessageFieldValue>> {
  match descriptor.r#type() {
    Type::Double => if let Some(f) = f.to_f64() {
      let constructed_value = MessageFieldValue {
        name: field_name.to_string(),
        raw_value: Some(f.to_string()),
        rtype: RType::Double(f)
      };
      update_message_builder(message_builder, field_type, descriptor, field_name, &constructed_value);
      Ok(Some(constructed_value))
    } else {
      Err(anyhow!("Can not construct a double value from the given value, field {} type is {:?} but value is {:?}",
        field_name, descriptor.r#type(), value))
    }
    Type::Float => if let Some(f) = f.to_f32() {
      let constructed_value = MessageFieldValue {
        name: field_name.to_string(),
        raw_value: Some(f.to_string()),
        rtype: RType::Float(f)
      };
      update_message_builder(message_builder, field_type, descriptor, field_name, &constructed_value);
      Ok(Some(constructed_value))
    } else {
      Err(anyhow!("Can not construct a float value from the given value, field {} type is {:?} but value is {:?}",
        field_name, descriptor.r#type(), value))
    }
    Type::Int32 | Type::Sint32 | Type::Sfixed32 => if let Some(i) = f.to_i32() {
      let constructed_value = MessageFieldValue {
        name: field_name.to_string(),
        raw_value: Some(i.to_string()),
        rtype: RType::Integer32(i)
      };
      update_message_builder(message_builder, field_type, descriptor, field_name, &constructed_value);
      Ok(Some(constructed_value))
    } else {
      Err(anyhow!("Can not construct an integer value from the given value, field {} type is {:?} but value is {:?}",
        field_name, descriptor.r#type(), value))
    }
    Type::Uint32 | Type::Fixed32 => if let Some(i) = f.to_u32() {
      let constructed_value = MessageFieldValue {
        name: field_name.to_string(),
        raw_value: Some(i.to_string()),
        rtype: RType::UInteger32(i)
      };
      update_message_builder(message_builder, field_type, descriptor, field_name, &constructed_value);
      Ok(Some(constructed_value))
    } else {
      Err(anyhow!("Can not construct an unsigned integer value from the given value, field {} type is {:?} but value is {:?}",
        field_name, descriptor.r#type(), value))
    }
    Type::Int64 | Type::Sint64 | Type::Sfixed64 => if let Some(i) = f.to_i64() {
      let constructed_value = MessageFieldValue {
        name: field_name.to_string(),
        raw_value: Some(i.to_string()),
        rtype: RType::Integer64(i)
      };
      update_message_builder(message_builder, field_type, descriptor, field_name, &constructed_value);
      Ok(Some(constructed_value))
    } else {
      Err(anyhow!("Can not construct an integer value from the given value, field {} type is {:?} but value is {:?}",
        field_name, descriptor.r#type(), value))
    }
    Type::Uint64 | Type::Fixed64 => if let Some(i) = f.to_u64() {
      let constructed_value = MessageFieldValue {
        name: field_name.to_string(),
        raw_value: Some(i.to_string()),
        rtype: RType::UInteger64(i)
      };
      update_message_builder(message_builder, field_type, descriptor, field_name, &constructed_value);
      Ok(Some(constructed_value))
    } else {
      Err(anyhow!("Can not construct an unsigned integer value from the given value, field {} type is {:?} but value is {:?}",
        field_name, descriptor.r#type(), value))
    },
    _ => {
      Err(anyhow!("Only numeric field values can be configured with a numeric value, field {} type is {:?} but value is {:?}",
        field_name, descriptor.r#type(), value))
    }
  }
}

fn update_message_builder(
  message_builder: &mut MessageBuilder,
  field_type: MessageFieldValueType,
  descriptor: &FieldDescriptorProto,
  field_name: &str,
  constructed_value: &MessageFieldValue
) {
  match field_type {
    MessageFieldValueType::Repeated => {
      debug!("Setting field {:?}:repeated to value {:?}", field_name, constructed_value);
      message_builder.add_repeated_field_value(descriptor, field_name, constructed_value.clone());
    },
    _ => {
      debug!("Setting field {:?}:{:?} to value {:?}", field_name, field_type, constructed_value);
      message_builder.set_field_value(descriptor, field_name, constructed_value.clone());
    }
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
  descriptor_cache: &DescriptorCache
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
              // TODO: replace this with "path.parent().unwrap_or(DocPath::root())" when pact_models
              // 1.1.6 is released
              parent(path).unwrap_or(DocPath::root())
            } else {
              path.clone()
            };
            trace!(?path, ?rule, "adding matching rule to path");
            matching_rules.add_rule(path, rule.clone(), RuleLogic::And)
          },
          Either::Right(mr) => return Err(anyhow!("Was expecting a value for '{}', but got a matching reference {:?}", path, mr))
        }
      }
    }
    if let Some(generator) = &mrd.generator {
      generators.insert(path.to_string(), generator.clone());
    }
    value_for_type(field_name, &value_for_field(&mrd), descriptor, &message_builder.descriptor,
                   descriptor_cache)
  } else {
    value_for_type(field_name, s, descriptor, &message_builder.descriptor,
      descriptor_cache)
  }
}

fn construct_bytes_value_from_array(
  field_name: &str,
  array: &Vec<Value>,
  raw_value: String
) -> anyhow::Result<MessageFieldValue> {
  trace!(?field_name, ?array, "Building bytes value from array");
  if array.iter().all(|v| v.is_number()) {
    Ok(MessageFieldValue {
      name: field_name.to_string(),
      raw_value: Some(raw_value),
      rtype: RType::Bytes(array.iter().map(|v| {
        if let Some(b) = v.as_u64() {
          b as u8
        } else if let Some(b) = v.as_i64() {
          b as u8
        } else {
          v.as_f64().unwrap_or_default() as u8
        }
      }).collect())
    })
  } else {
    Err(anyhow!("Byte arrays can only be constructed from arrays of numbers, got '{}'", raw_value))
  }
}

fn parent(path: &DocPath) -> Option<DocPath> {
  let tokens = path.tokens().clone();
  if path.is_root() || tokens.len() <= 1 {
    None
  } else {
    let mut path = DocPath::root();
    let tokens = tokens.split_last().unwrap().1;
    for part in tokens.iter().skip(1) {
      path = path.join(part.to_string());
    }
    Some(path)
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
  descriptor_cache: &DescriptorCache
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
        if let Ok(bytes) = BASE64.decode(field_value) {
          Ok(MessageFieldValue::bytes(field_name, bytes.as_slice(), field_value))
        } else {
          Ok(MessageFieldValue::str_bytes(field_name, field_value))
        }
      } else {
        Err(anyhow!("value_for_type: Protobuf field {} has an unsupported type {:?} {}", field_name, t, type_name))
      }
    }

    Type::Bytes => {
      if let Ok(bytes) = BASE64.decode(field_value) {
        Ok(MessageFieldValue::bytes(field_name, bytes.as_slice(), field_value))
      } else {
        Ok(MessageFieldValue::str_bytes(field_name, field_value))
      }
    },

    Type::Enum => {
      // First try to find enum in the message's nested enums, then try globally
      let result = find_enum_value_by_name_in_message(&message_descriptor.enum_type, type_name.as_str(), field_value)
        .or_else(|| {
          // If not found in message, find the enum globally and then look up the value
          descriptor_cache.find_enum_value_by_name(type_name.as_str(), field_value)
        });
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
pub(crate) mod tests;
