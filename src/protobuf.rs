//! Module for processing and comparing protobuf messages

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use anyhow::anyhow;
use itertools::{Either, Itertools};
use log::{debug, error, LevelFilter, max_level, trace, warn};
use maplit::hashmap;
use pact_models::generators::Generator;
use pact_models::matchingrules::MatchingRuleCategory;
use pact_models::matchingrules::expressions::{MatchingReference, parse_matcher_def};
use pact_models::path_exp::DocPath;
use pact_models::prelude::RuleLogic;
use pact_plugin_driver::proto::{InteractionResponse, PluginConfiguration, Body};
use pact_plugin_driver::proto::body::ContentTypeHint;
use pact_plugin_driver::utils::{proto_value_to_json, proto_value_to_string, to_proto_struct};
use prost_types::{DescriptorProto, field_descriptor_proto, FieldDescriptorProto, FileDescriptorProto};
use prost_types::field_descriptor_proto::Type;
use serde_json::{json, Value};
use tokio::fs::File;
use tokio::io::AsyncReadExt;

use crate::message_builder::{MessageBuilder, MessageFieldValue};
use crate::protoc::Protoc;

/// Process the provided protobuf file and configure the interaction
pub(crate) async fn process_proto(proto_file: String, protoc: &Protoc, config: BTreeMap<String, prost_types::Value>) -> anyhow::Result<(Vec<InteractionResponse>, PluginConfiguration)> {
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

  let descriptor_hash = base64::encode(descriptor_bytes);
  let mut interactions = vec![];

  if let Some(message_type) = config.get("pact:message-type") {
    let message = proto_value_to_string(message_type)
      .ok_or_else(|| anyhow!("Did not get a valid value for 'pact:message-type'. It should be a string"))?;
    let result = configure_protobuf_message(message.as_str(), config, descriptor, file_descriptors, proto_file, descriptor_hash.as_str())?;
    interactions.push(result);
  } else if let Some(service_name) = config.get("pact:proto-service") {
    let service_name = proto_value_to_string(service_name)
      .ok_or_else(|| anyhow!("Did not get a valid value for 'pact:proto-service'. It should be a string"))?;
    let (request_part, response_part) = configure_protobuf_service(service_name, config, descriptor, file_descriptors, proto_file, descriptor_hash.as_str())?;
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
        "protoDescriptors": descriptor_hash
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
  all_descriptors: HashMap<String, &FileDescriptorProto>,
  proto_file: &Path,
  descriptor_hash: &str
) -> anyhow::Result<(InteractionResponse, InteractionResponse)> {
  // is Ok -> {
  //   val (requestPart, responsePart) = result.value
  //   val pluginConfigurationBuilder = requestPart.pluginConfigurationBuilder
  //   pluginConfigurationBuilder.interactionConfigurationBuilder
  //     .putFields("service", Value.newBuilder().setStringValue(serviceName).build())
  //     .putFields("descriptorKey", Value.newBuilder().setStringValue(descriptorHash.toString()).build())
  //   interactions.add(requestPart)
  //   interactions.add(responsePart)
  // }
  todo!()
}

/// Configure the interaction for a single Protobuf message
fn configure_protobuf_message(
  message_name: &str,
  config: BTreeMap<String, prost_types::Value>,
  descriptor: &FileDescriptorProto,
  all_descriptors: HashMap<String, &FileDescriptorProto>,
  proto_file: &Path,
  descriptor_hash: &str
) -> anyhow::Result<InteractionResponse> {
  debug!("Looking for message of type '{}'", message_name);
  let message_descriptor = descriptor.message_type
    .iter().find(|p| p.name.clone().unwrap_or_default() == message_name)
    .ok_or_else(|| anyhow!("Did not find a descriptor for message '{}'", message_name))?;
  construct_protobuf_interaction_for_message(message_descriptor, config, message_name)
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
  message_name: &str
) -> anyhow::Result<InteractionResponse> {
  let mut message_builder = MessageBuilder::new(message_descriptor, message_name);
  let mut matching_rules = MatchingRuleCategory::empty("body");
  let mut generators = hashmap!{};

  debug!("Building message from Protobuf descriptor");
  for (key, value) in config {
    if !key.starts_with("pact:") {
      if let Some(field) = message_descriptor.field.iter().find(|f| f.name.clone().unwrap_or_default() == key) {
        match field.r#type {
          Some(r#type) => if r#type == field_descriptor_proto::Type::Message as i32 {
            let (message_value, additional_values) = build_message_field_value(DocPath::root().push_field(key.as_str()), field, key.as_str(), value, &mut matching_rules, &mut generators)?;
            debug!("Setting field {} to value {:?}", key, message_value);
            if field.label.unwrap_or_default() == field_descriptor_proto::Label::Repeated as i32 {
              message_builder.add_repeated_field_value(field, key.as_str(), message_value);
              for item in additional_values {
                message_builder.add_repeated_field_value(field, key.as_str(), item);
              }
            } else {
              message_builder.set_field(field, key.as_str(), message_value);
            }
          } else {
            let field_value = build_field_value(&DocPath::root(), field, key.as_str(), value, &mut matching_rules, &mut generators)?;
            if let Some(field_value) = field_value {
              debug!("Setting field {:?} to value {:?}", key, field_value);
              message_builder.set_field(field, key.as_str(), field_value);
            }
          }
          None => {
            return Err(anyhow!("Message {} field {} is of an unknown type", message_name, key))
          }
        }
      } else {
        return Err(anyhow!("Message {} has no field {}", message_name, key))
      }
    }
  }

  debug!("Returning response");

  //       .setInteractionMarkup("""
  //         |## ${descriptor.name}
  //         |```
  //         |$message
  //         |```
  //         |
  //       """.trimMargin("|"))
  //
  //     builder.contentsBuilder
  //       .setContentType("application/protobuf;message=$messageName")
  //       .setContent(BytesValue.newBuilder().setValue(message.toByteString()).build())
  //       .setContentTypeHint(Plugin.Body.ContentTypeHint.BINARY)
  //
  //     for ((key, rules) in matchingRules.matchingRules) {
  //       val rulesBuilder = Plugin.MatchingRules.newBuilder()
  //
  //       for (rule in rules.rules) {
  //         rulesBuilder.addRule(
  //           Plugin.MatchingRule.newBuilder()
  //             .setType(rule.name)
  //             .setValues(toProtoStruct(rule.attributes))
  //             .build()
  //         )
  //       }
  //
  //       builder.putRules(key, rulesBuilder.build())
  //     }
  //
  //     for ((key, generator) in generators) {
  //       builder.putGenerators(
  //         key, Plugin.Generator.newBuilder()
  //           .setType(generator.type)
  //           .setValues(toProtoStruct(toJson(generator.toMap(PactSpecVersion.V4)).asObject()!!.entries))
  //           .build()
  //       )
  //     }

  Ok(InteractionResponse {
    contents: Some(Body {
      content_type: format!("application/protobuf;message={}", message_name),
      content: Some(message_builder.encode_message()?.to_vec()),
      content_type_hint: ContentTypeHint::Binary as i32,
    }),
    //     /// All matching rules to apply
    //     #[prost(map = "string, message", tag = "2")]
    //     pub rules: ::std::collections::HashMap<::prost::alloc::string::String, MatchingRules>,
    //     /// Generators to apply
    //     #[prost(map = "string, message", tag = "3")]
    //     pub generators: ::std::collections::HashMap<::prost::alloc::string::String, Generator>,
    //     /// For message interactions, any metadata to be applied
    //     #[prost(message, optional, tag = "4")]
    //     pub message_metadata: ::core::option::Option<::prost_types::Struct>,
    //     /// Markdown/HTML formatted text representation of the interaction
    //     #[prost(string, tag = "6")]
    //     pub interaction_markup: ::prost::alloc::string::String,
    //     #[prost(enumeration = "interaction_response::MarkupType", tag = "7")]
    //     pub interaction_markup_type: i32,
    //     /// Description of what part this interaction belongs to (in the case of there being more than one, for instance,
    //     /// request/response messages)
    //     #[prost(string, tag = "8")]
    //     pub part_name: ::prost::alloc::string::String,
    .. InteractionResponse::default()
  })
}

fn build_message_field_value(
  path: &DocPath,
  descriptor: &FieldDescriptorProto,
  field: &str,
  value: prost_types::Value,
  matching_rules: &mut MatchingRuleCategory,
  generators: &mut HashMap<String, Generator>
) -> anyhow::Result<(MessageFieldValue, Vec<MessageFieldValue>)> {
  todo!()
}

/// Constructs a simple message field (non-repeated or map) from the configuration value and
/// updates the matching rules and generators for it.
fn build_field_value(
  path: &DocPath,
  descriptor: &FieldDescriptorProto,
  key: &str,
  value: prost_types::Value,
  matching_rules: &mut MatchingRuleCategory,
  generators: &mut HashMap<String, Generator>
) -> anyhow::Result<Option<MessageFieldValue>> {
  trace!("build_field_value({}, {}, {:?})", path, key, proto_value_to_json(&value));

  if let Some(val) = &value.kind {
    if let prost_types::value::Kind::NullValue(_) = val {
      Ok(None)
    } else {
      let mrd = parse_matcher_def(proto_value_to_string(&value)
        .ok_or_else(|| anyhow!("Field values must be a string, got {:?}", value))?.as_str())?;
      let mut field_path = path.clone();
      field_path.push_field(key);
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
      value_for_type(key, mrd.value.as_str(), descriptor)
        .map(|val| Some(val))
    }
  } else {
    Err(anyhow!("Field '{}' has an unknown type, can not do anything with it", key))
  }
}

fn value_for_type(field_name: &str, field_value: &str, descriptor: &FieldDescriptorProto) -> anyhow::Result<MessageFieldValue> {
  trace!("value_for_type({}, {}, descriptor)", field_name, field_value);
  debug!("Creating value for type {:?} from '{}'", descriptor.type_name, field_value);
  //         Descriptors.FieldDescriptor.JavaType.ENUM -> field.enumType.findValueByName(fieldValue)
  //         Descriptors.FieldDescriptor.JavaType.MESSAGE -> {
  //           if (field.messageType.fullName == "google.protobuf.BytesValue") {
  //             BytesValue.newBuilder().setValue(ByteString.copyFromUtf8(fieldValue ?: "")).build()
  //           } else {
  //             logger.error { "field ${field.name} is a Message type" }
  //             throw RuntimeException("field ${field.name} is a Message type")
  //           }
  //         }
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
    // Type::Message => {}
    Type::Bytes => Ok(MessageFieldValue::bytes(field_name, field_value)),
    // Type::Enum => {}
    _ => Err(anyhow!("Protobuf field {} has an unsupported type {:?}", field_name, t))
  }
}

#[cfg(test)]
mod tests {
  use expectest::prelude::*;
  use maplit::btreemap;
  use prost_types::field_descriptor_proto::Type;
  use prost_types::{DescriptorProto, field_descriptor_proto, FieldDescriptorProto, Value};

  use crate::message_builder::RType;
  use crate::protobuf::{construct_protobuf_interaction_for_message, value_for_type};

  #[test]
  fn value_for_type_test() {
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
    let result = value_for_type("test", "test", &descriptor).unwrap();
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
    let result = value_for_type("test", "100", &descriptor).unwrap();
    expect!(result.name).to(be_equal_to("test"));
    expect!(result.raw_value).to(be_some().value("100".to_string()));
    expect!(result.rtype).to(be_equal_to(RType::UInteger64(100)));
  }

  #[test]
  fn construct_protobuf_interaction_for_message_test() {
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

    let result = construct_protobuf_interaction_for_message(&message_descriptor, config, "test_message").unwrap();
    let body = result.contents.as_ref().unwrap();
    expect!(body.content_type.as_str()).to(be_equal_to("application/protobuf;message=test_message"));
    expect!(body.content_type_hint).to(be_equal_to(0));
    expect!(body.content.as_ref()).to(be_some().value(&vec![ 88 ]));

    // #[prost(message, optional, tag = "1")]
    //     pub contents: ::core::option::Option<Body>,
    //     /// All matching rules to apply
    //     #[prost(map = "string, message", tag = "2")]
    //     pub rules: ::std::collections::HashMap<::prost::alloc::string::String, MatchingRules>,
    //     /// Generators to apply
    //     #[prost(map = "string, message", tag = "3")]
    //     pub generators: ::std::collections::HashMap<::prost::alloc::string::String, Generator>,
    //     /// For message interactions, any metadata to be applied
    //     #[prost(message, optional, tag = "4")]
    //     pub message_metadata: ::core::option::Option<::prost_types::Struct>,
    //     /// Plugin specific data to be persisted in the pact file
    //     #[prost(message, optional, tag = "5")]
    //     pub plugin_configuration: ::core::option::Option<PluginConfiguration>,
    //     /// Markdown/HTML formatted text representation of the interaction
    //     #[prost(string, tag = "6")]
    //     pub interaction_markup: ::prost::alloc::string::String,
    //     #[prost(enumeration = "interaction_response::MarkupType", tag = "7")]
    //     pub interaction_markup_type: i32,
    //     /// Description of what part this interaction belongs to (in the case of there being more than one, for instance,
    //     /// request/response messages)
    //     #[prost(string, tag = "8")]
    //     pub part_name: ::prost::alloc::string::String,
  }
}
