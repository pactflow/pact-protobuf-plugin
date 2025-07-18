//! Module provides the main gRPC server for the plugin process

use std::collections::{BTreeMap, HashMap};
use std::collections::hash_map::Entry;
use std::fs::File;
use std::io::BufReader;

use anyhow::{anyhow, bail};
use bytes::{Bytes, BytesMut};
use maplit::hashmap;
use pact_matching::{BodyMatchResult, Mismatch};
use pact_matching::generators::DefaultVariantMatcher;
use pact_models::generators::{
  GenerateValue,
  Generator,
  GeneratorCategory,
  GeneratorTestMode,
  VariantMatcher
};
use pact_models::json_utils::json_to_string;
use pact_models::matchingrules::MatchingRule;
use pact_models::path_exp::DocPath;
use pact_models::prelude::{ContentType, MatchingRuleCategory, OptionalBody, RuleLogic};
use pact_models::v4::sync_message::SynchronousMessage;
use pact_plugin_driver::plugin_models::PactPluginManifest;
use pact_plugin_driver::proto;
use pact_plugin_driver::proto::{Body, body, CompareContentsRequest, CompareContentsResponse, ConfigureInteractionResponse, GenerateContentRequest, GenerateContentResponse, MetadataValue, MockServerResult, PluginConfiguration, StartMockServerResponse, VerificationPreparationResponse, VerifyInteractionResponse};
use pact_plugin_driver::proto::body::ContentTypeHint;
use pact_plugin_driver::proto::catalogue_entry::EntryType;
use pact_plugin_driver::proto::generate_content_request::TestMode;
use pact_plugin_driver::proto::pact_plugin_server::PactPlugin;
use pact_plugin_driver::utils::{
  proto_struct_to_json,
  proto_struct_to_map,
  proto_value_to_json,
  proto_value_to_string,
  to_proto_value
};
use pact_verifier::verification_result::VerificationMismatchResult;
use prost_types::{DescriptorProto, FileDescriptorProto, FileDescriptorSet, MethodDescriptorProto, ServiceDescriptorProto};
use prost_types::value::Kind;
use serde_json::Value;
use tonic::{Request, Response, Status};
use tonic::metadata::KeyAndValueRef;
use tracing::{debug, error, info, instrument, trace, warn};

use crate::dynamic_message::DynamicMessage;
use crate::matching::{match_message, match_service};
use crate::message_decoder::{decode_message, ProtobufField};
use crate::metadata::{MessageMetadataValue, MetadataMatchResult};
use crate::mock_server::{GrpcMockServer, MOCK_SERVER_STATE};
use crate::protobuf::process_proto;
use crate::protoc::setup_protoc;
use crate::utils::{
  build_grpc_route,
  find_message_descriptor_for_type,
  get_descriptors_for_interaction,
  lookup_interaction_by_id,
  lookup_service_descriptors_for_interaction,
  parse_pact_from_request_json,
  to_fully_qualified_name
};
use crate::verification::verify_interaction;

/// Plugin gRPC server implementation
#[derive(Debug, Default)]
pub struct ProtobufPactPlugin {
  manifest: PactPluginManifest
}

impl ProtobufPactPlugin {
  /// Create a new plugin instance
  pub fn new() -> Self {
    let manifest = File::open("./pact-plugin.json")
      .and_then(|file| {
        let reader = BufReader::new(file);
        match serde_json::from_reader::<BufReader<File>, PactPluginManifest>(reader) {
          Ok(manifest) => Ok(manifest),
          Err(err) => Err(err.into())
        }
      })
      .unwrap_or_default();
    ProtobufPactPlugin { manifest }
  }

  /// Return a Tonic error response for the given error
  fn error_response<E>(err: E) -> Result<Response<CompareContentsResponse>, Status>
    where E: Into<String> {
    Ok(Response::new(proto::CompareContentsResponse {
      error: err.into(),
      ..proto::CompareContentsResponse::default()
    }))
  }

  /// Returns the configured hostname to bind to from the configuration in the manifest.
  pub fn host_to_bind_to(&self) -> Option<String> {
    self.manifest.plugin_config
      .get("hostToBindTo")
      .map(json_to_string)
  }

  /// Returns any additional include paths from the configuration in the manifest to add to the
  /// Protocol Buffers compiler call.
  pub fn additional_includes(&self, config: &HashMap<String, Value>) -> Vec<String> {
    config
      .get("additionalIncludes")
      .map(|includes| {
        match includes {
          Value::Array(list) => list.iter().map(json_to_string).collect(),
          _ => vec![json_to_string(includes)]
        }
      })
      .unwrap_or_default()
  }

  fn get_mock_server_results(
    results: &HashMap<String, (usize, Vec<(BodyMatchResult, MetadataMatchResult)>)>
  ) -> (bool, Vec<MockServerResult>) {
    // All OK if there are no mismatches and all routes got at least one request
    let ok = results.iter().all(|(_, (req, r))| {
      *req > 0 && r.iter().all(|(body_result, metadata_result)| {
        *body_result == BodyMatchResult::Ok && metadata_result.all_matched()
      })
    });

    let results = results.iter()
      .flat_map(|(path, (req, r))| {
      let mut route_results = vec![];

      if *req == 0 {
        route_results.push(MockServerResult {
          path: path.clone(),
          error: format!("Did not receive any requests for path '{}'", path),
          ..MockServerResult::default()
        });
      } else {
        route_results.push(MockServerResult {
          path: path.clone(),
          mismatches: r.iter().flat_map(|(body_result, metadata_result)| {
            let mut proto_result = vec![];

            let mismatches = body_result.mismatches();
            for m in mismatches {
              match m {
                Mismatch::BodyMismatch { path, mismatch, expected, actual } => {
                  proto_result.push(proto::ContentMismatch {
                    expected: expected.as_ref().map(|d| d.to_vec()),
                    actual: actual.as_ref().map(|d| d.to_vec()),
                    mismatch: mismatch.clone(),
                    path: path.clone(),
                    mismatch_type: "body".to_string(),
                    ..proto::ContentMismatch::default()
                  });
                }
                _ => {
                  proto_result.push(proto::ContentMismatch {
                    mismatch: m.description(),
                    mismatch_type: "body".to_string(),
                    ..proto::ContentMismatch::default()
                  });
                }
              }
            }

            for m in &metadata_result.mismatches {
              match m {
                Mismatch::MetadataMismatch { key, mismatch, expected, actual } => {
                  proto_result.push(proto::ContentMismatch {
                    expected: Some(expected.as_bytes().to_vec()),
                    actual: Some(actual.as_bytes().to_vec()),
                    mismatch: mismatch.clone(),
                    path: key.clone(),
                    mismatch_type: "metadata".to_string(),
                    ..proto::ContentMismatch::default()
                  });
                }
                _ => {
                  proto_result.push(proto::ContentMismatch {
                    mismatch: m.description(),
                    mismatch_type: "metadata".to_string(),
                    ..proto::ContentMismatch::default()
                  });
                }
              }
            }

            proto_result
          }).collect(),
          ..MockServerResult::default()
        });
      }

      route_results
    }).collect();
    (ok, results)
  }

  /// Compare expected and actual contents and return results of the comparison.
  ///
  /// # Arguments:
  /// 
  /// * `request` - The request to compare the contents. Contains the following fields:
  ///  * `expected`, `actual` - Both are of type `Body` and contain the following fields:
  ///   * `content` - Actual request bytes
  ///   * `content_type` - We populate it, e.g. `"application/protobuf;message=.routeguide.Feature"`. 
  ///     Older versions of the plugin would not use fully-qualified name, so we support both.
  ///   * `content_type_hint` - always Default
  ///  * `rules` - Matching rules for this interaction, as defined in the pact file
  ///  * `allow_unexpected_keys` - true (possibly configurable)
  ///  * `plugin_configuration` - contains both pact-level plugin config and interaction-specific config:
  ///   * `interaction_configuration` - interaction-level plugin config. In pact json file 
  ///     it is under `pluginConfiguration` inside each interaction. Contains:
  ///     * `descriptorKey` - key to look up a file descriptor set in the pact-level plugin config 
  ///       (it can contain multiple sets of file descriptors)
  ///     * `message` or `service` field (but not both) - specifying either the message for this interaction or 
  ///       a gRPC service, a fully-qualified name if pact was generated by the current plugin version, or 
  ///       without the ".package" part if it's an older version.
  ///   * `pact_configuration` - pact-level plugin config, map keyed by `descriptorKey`s containing:
  ///    * `protoDescriptors` field which is a serialized `FileDescriptorSet` containing all available 
  ///     protobuf file descriptors.
  ///    * `protoFile`: raw text of a .proto file specified in `"pact:proto"` in the original test config
  /// 
  /// # Returns
  /// 
  /// Comparison results, containing either a list of mismatches or a success message.
  fn compare_contents_impl(&self, request: &CompareContentsRequest) -> anyhow::Result<CompareContentsResponse> {
    // Check for the plugin specific configuration for the interaction
    let plugin_configuration = request.plugin_configuration.clone().unwrap_or_default();
    let interaction_config = get_interaction_config(&plugin_configuration)?;

    // From the plugin configuration for the interaction, get the descriptor key. This key is used
    // to lookup the encoded Protobuf descriptors in the Pact level plugin configuration
    let expected_message_type = request.expected.as_ref()
      .and_then(|body| ContentType::parse(body.content_type.as_str()).ok())
      .as_ref()
      .and_then(|ct| ct.attributes.get("message").clone())
      .cloned();
    let message_key = Self::lookup_message_key(&interaction_config, &expected_message_type)?;
    debug!("compare_contents: message_key = {}", message_key);

    // From the plugin configuration for the interaction, there should be either a message type name
    // or a service name. Check for either.
    let (message, service) = Self::lookup_message_and_service(&interaction_config, &expected_message_type)?;

    let descriptors = Self::lookup_descriptors(plugin_configuration, message_key)?;

    let mut expected_body = request.expected.as_ref()
      .and_then(|body| body.content.clone().map(Bytes::from))
      .unwrap_or_default();
    let mut actual_body = request.actual.as_ref()
      .and_then(|body| body.content.clone().map(Bytes::from))
      .unwrap_or_default();
    let mut matching_rules = MatchingRuleCategory::empty("body");
    for (key, rules) in &request.rules {
      for rule in &rules.rule {
        let values = rule.values.as_ref().map(proto_struct_to_json).unwrap_or_default();
        let doc_path = match DocPath::new(key) {
          Ok(path) => path,
          Err(err) => return Err(anyhow!(err))
        };
        let matching_rule = match MatchingRule::create(&rule.r#type, &values) {
          Ok(rule) => rule,
          Err(err) => return Err(anyhow!(err))
        };
        matching_rules.add_rule(doc_path, matching_rule, RuleLogic::And);
      }
    }

    let result = if let Some(message_name) = message {
      debug!("Received compare_contents request for message {}", message_name);
      match_message(
        message_name.as_str(),
        &descriptors,
        &mut expected_body,
        &mut actual_body,
        &matching_rules,
        request.allow_unexpected_keys,
        &hashmap!{}
      )
    } else if let Some(service_name) = service {
      debug!("Received compareContents request for service {}", service_name);
      let content_type = request.expected.as_ref().map(|body| body.content_type.clone())
        .unwrap_or_default();
      let expected_content_type = match ContentType::parse(content_type.as_str()) {
        Ok(ct) => ct,
        Err(err) => return Err(anyhow!("Expected content type is not set or not valid - {}", err))
      };
      match_service(
        service_name.as_str(),
        &descriptors,
        &mut expected_body,
        &mut actual_body,
        &matching_rules,
        request.allow_unexpected_keys,
        &expected_content_type
      )
    } else {
      Err(anyhow!("Did not get a message or service to match"))
    };

    match result {
      Ok(result) => match result {
        BodyMatchResult::Ok => Ok(proto::CompareContentsResponse::default()),
        BodyMatchResult::BodyTypeMismatch { message, expected_type, actual_type, .. } => {
          error!("Got a BodyTypeMismatch - {}", message);
          Ok(CompareContentsResponse {
            type_mismatch: Some(proto::ContentTypeMismatch {
              expected: expected_type,
              actual: actual_type
            }),
            ..proto::CompareContentsResponse::default()
          })
        }
        BodyMatchResult::BodyMismatches(mismatches) => {
          Ok(CompareContentsResponse {
            results: mismatches.iter().map(|(k, v)| {
              (k.clone(), proto::ContentMismatches {
                mismatches: v.iter().map(mismatch_to_proto_mismatch).collect()
              })
            }).collect(),
            ..proto::CompareContentsResponse::default()
          })
        }
      }
      Err(err) => Err(err)
    }
  }

  fn lookup_descriptors(plugin_configuration: PluginConfiguration, message_key: String) -> anyhow::Result<FileDescriptorSet> {
    let pact_configuration = plugin_configuration.pact_configuration.unwrap_or_default();
    debug!("Pact level configuration keys: {:?}", pact_configuration.fields.keys());

    let config_for_interaction = pact_configuration.fields.iter()
      .map(|(key, config)| (key.clone(), proto_value_to_json(config)))
      .collect();
    get_descriptors_for_interaction(message_key.as_str(), &config_for_interaction)
  }

  fn lookup_message_and_service(
    interaction_config: &BTreeMap<String, prost_types::Value>,
    expected_message_type: &Option<String>
  ) -> anyhow::Result<(Option<String>, Option<String>)> {
    // Both message and service will be a fully-qualified name starting with the "." if the pact 
    // was generated by the current version of the plugin; or without the "." if it's an older version.
    // Example message: `.routeguide.Feature`
    // Service name will also include method, e.g. `.routeguide.RouteGuide/GetFeature`
    // Note that the message (but not the service) could be sent under a request or response key,
    // in which case we need to use the expected value from the content type.
    let message = Self::lookup_message_type(interaction_config, expected_message_type);
    let service = interaction_config.get("service").and_then(proto_value_to_string);
    if message.is_none() && service.is_none() {
      error!("Plugin configuration item with key 'message' or 'service' is required");
      Err(anyhow!("Plugin configuration item with key 'message' or 'service' is required"))
    } else {
      Ok((message, service))
    }
  }

  fn lookup_message_type(
    interaction_config: &BTreeMap<String, prost_types::Value>,
    expected_message_type: &Option<String>
  ) -> Option<String> {
    interaction_config.get("message")
      .and_then(proto_value_to_string)
      .or_else(|| expected_message_type.clone())
  }

  fn lookup_message_key(
    interaction_config: &BTreeMap<String, prost_types::Value>,
    expected_message_type: &Option<String>
  ) -> anyhow::Result<String> {
    if let Some(key) = interaction_config.get("descriptorKey").and_then(proto_value_to_string) {
      return Ok(key);
    }

    // The descriptor key may be stored under a request or response key. We use the message type
    // from the content type to match it.
    if let Some(expected_message_type) = expected_message_type {
      if let Some(request_config) = interaction_config.get("request") {
        if let Some(Kind::StructValue(s)) = &request_config.kind {
          if let Some(message) = s.fields.get("message").and_then(proto_value_to_string) {
            if message == expected_message_type.as_str() {
              if let Some(key) = s.fields.get("descriptorKey").and_then(proto_value_to_string) {
                return Ok(key);
              }
            }
          }
        }
      }

      if let Some(response_config) = interaction_config.get("response") {
        if let Some(Kind::StructValue(s)) = &response_config.kind {
          if let Some(message) = s.fields.get("message").and_then(proto_value_to_string) {
            if message == expected_message_type.as_str() {
              if let Some(key) = s.fields.get("descriptorKey").and_then(proto_value_to_string) {
                return Ok(key);
              }
            }
          }
        }
      }
    }

    error!("Plugin configuration item with key 'descriptorKey' is required");
    Err(anyhow!("Plugin configuration item with key 'descriptorKey' is required"))
  }

  /// Generate contents for the interaction.
  /// 
  /// Calls `generate_protobuf_contents` to do the actual generation. 
  /// 
  /// # Arguments:
  ///  
  ///   * `request` - The request to generate the contents. Contains the following fields:
  ///     * `contents`: the request body with `content_type`, `content` and `content_type_hint` fields,
  ///     * `generators`: map of generators for each field
  ///     * `plugin_configuration`: similar to `compare_contents` request:
  ///       * `interaction_configuration` with message/service, descriptorKey and package
  ///       * `pact_configuration` with hash of all descriptors
  ///     * `test_context`: test context
  ///     * `test_mode`: consumer or provider (or unknown)
  ///     * `content_for`: can be request or response
  /// 
  /// # Returns
  /// 
  /// Generated contents for the interaction.
  #[instrument(ret, skip(self))]
  fn generate_contents_impl(&self, request: &GenerateContentRequest) -> anyhow::Result<GenerateContentResponse> {
    // Check for the plugin specific configuration for the interaction
    let plugin_configuration = request.plugin_configuration.clone().unwrap_or_default();
    let interaction_config = get_interaction_config(&plugin_configuration)?;

    // From the plugin configuration for the interaction, get the descriptor key. This key is used
    // to lookup the encoded Protobuf descriptors in the Pact level plugin configuration
    let expected_message_type = request.contents.as_ref()
      .and_then(|body| ContentType::parse(body.content_type.as_str()).ok())
      .as_ref()
      .and_then(|ct| ct.attributes.get("message").clone())
      .cloned();
    let message_key = Self::lookup_message_key(&interaction_config, &expected_message_type)?;
    debug!("generate_contents: message_key = {}", message_key);

    let descriptors = Self::lookup_descriptors(plugin_configuration, message_key)?;

    if let Some(contents) = &request.contents {
      let content_type = ContentType::parse(contents.content_type.as_str())?;
      match content_type.attributes.get("message") {
        Some(message_type) => {
          let (message_descriptor, _) = find_message_descriptor_for_type(message_type, &descriptors)?;
          let mut body = contents.content.clone().map(Bytes::from).unwrap_or_default();
          if body.is_empty() {
            Ok(GenerateContentResponse::default())
          } else {
            let field_data = decode_message(&mut body, &message_descriptor, &descriptors)?;
            debug!("message to generate = {:?}", field_data);
            let generated_message = generate_protobuf_contents(&message_descriptor, &field_data, &content_type, &request.generators, &descriptors, request.test_mode())?;
            Ok(GenerateContentResponse {
              contents: Some(generated_message),
            })
          }
        }
        None => Err(anyhow!("Content type does not contain a message attribute"))
      }
    } else {
      Ok(GenerateContentResponse::default())
    }
  }

  fn setup_plugin_config(&self, fields: &BTreeMap<String, prost_types::Value>) -> anyhow::Result<HashMap<String, Value>> {
    match fields.get("pact:protobuf-config") {
      Some(config) => if let Some(kind) = &config.kind {
        let mut plugin_config = self.manifest.plugin_config.clone();
        match kind {
          Kind::NullValue(_) => Ok(plugin_config),
          Kind::StructValue(s) => {
            for (k, v) in &s.fields {
              let val = proto_value_to_json(v);
              match plugin_config.entry(k.clone()) {
                Entry::Occupied(mut e) => {
                  e.insert(merge_value(e.get(), &val)?);
                },
                Entry::Vacant(e) => {
                  e.insert(val);
                }
              }
            }
            Ok(plugin_config)
          }
          _ => bail!("pact:protobuf-config must be ab object, got {:?}", kind)
        }
      } else {
        Ok(self.manifest.plugin_config.clone())
      }
      None => Ok(self.manifest.plugin_config.clone())
    }
  }

  fn verification_preparation_error_response<E: Into<String>>(err: E) -> Response<VerificationPreparationResponse> {
    Response::new(proto::VerificationPreparationResponse {
      response: Some(proto::verification_preparation_response::Response::Error(err.into())),
      ..proto::VerificationPreparationResponse::default()
    })
  }

  // Applies the metadata to the request, sourced from merging the interaction request metadata and
  // the Tonic request metadata.
  fn setup_metadata(
    interaction: &SynchronousMessage,
    service_desc: &ServiceDescriptorProto,
    method_desc: &MethodDescriptorProto,
    file_desc: &FileDescriptorProto,
    request: Request<DynamicMessage>
  ) -> HashMap<String, MetadataValue> {
    let mut request_metadata = hashmap! {};
    for (k, v) in &interaction.request.metadata {
      request_metadata.insert(k.clone(), proto::MetadataValue {
        value: Some(proto::metadata_value::Value::NonBinaryValue(to_proto_value(v)))
      });
    }

    let service_full_name = to_fully_qualified_name(service_desc.name(), file_desc.package()).unwrap_or_default();
    let path = build_grpc_route(service_full_name.as_str(), method_desc.name()).unwrap_or_default();
    request_metadata.insert("request-path".to_string(), proto::MetadataValue {
      value: Some(proto::metadata_value::Value::NonBinaryValue(prost_types::Value {
        kind: Some(Kind::StringValue(path))
      }))
    });

    for entry in request.metadata().iter() {
      match entry {
        KeyAndValueRef::Ascii(k, v) => {
          request_metadata.insert(k.to_string(), proto::MetadataValue {
            value: Some(proto::metadata_value::Value::NonBinaryValue(prost_types::Value {
              kind: Some(Kind::StringValue(v.to_str().unwrap_or_default().to_string()))
            }))
          });
        }
        KeyAndValueRef::Binary(k, v) => {
          request_metadata.insert(k.to_string(), proto::MetadataValue {
            value: Some(proto::metadata_value::Value::BinaryValue(v.to_bytes().unwrap_or_default().to_vec()))
          });
        }
      }
    }
    request_metadata
  }

  fn apply_generators_to_metadata(
    interaction: SynchronousMessage,
    test_context: &HashMap<&str, Value>,
    request_metadata: &mut HashMap<String, MetadataValue>
  ) {
    let metadata_generators = interaction.request.generators.categories
      .get(&GeneratorCategory::METADATA)
      .cloned()
      .unwrap_or_default();
    if !metadata_generators.is_empty() {
      debug!(?metadata_generators, ?test_context, "Applying metadata generators...");
      let vm = DefaultVariantMatcher.boxed();
      for (key, generator) in &metadata_generators {
        if generator.corresponds_to_mode(&GeneratorTestMode::Provider) {
          if let Some(header) = key.first_field() {
            match generator.generate_value(&MessageMetadataValue::default(), &test_context, &vm) {
              Ok(v) => {
                request_metadata.insert(header.to_string(), proto::MetadataValue {
                  value: Some(proto::metadata_value::Value::NonBinaryValue(prost_types::Value {
                    kind: Some(Kind::StringValue(v.value))
                  }))
                });
              }
              Err(err) => {
                warn!("Failed to generate value for metadata key '{}': {}", key, err);
              }
            }
          }
        }
      }
    }
  }

  fn configure_interaction_error_response<S: Into<String>>(message: S) -> Response<ConfigureInteractionResponse> {
    Response::new(ConfigureInteractionResponse {
      error: message.into(),
      .. ConfigureInteractionResponse::default()
    })
  }

  fn start_mock_server_error<S: Into<String>>(err: S) -> Response<StartMockServerResponse> {
    tonic::Response::new(proto::StartMockServerResponse {
      response: Some(proto::start_mock_server_response::Response::Error(err.into())),
      .. proto::StartMockServerResponse::default()
    })
  }

  fn verify_interaction_error<S: Into<String>>(err: S) -> Response<VerifyInteractionResponse> {
    Response::new(proto::VerifyInteractionResponse {
      response: Some(proto::verify_interaction_response::Response::Error(err.into())),
      .. proto::VerifyInteractionResponse::default()
    })
  }
}

/// Generate contents for the interaction
/// 
/// # Arguments:
///  * `message_descriptor` - Descriptor for the message
///  * `fields` - all fields in the message to generate contents for
///  * `content_type` - content type of the message, comes from generation request
///  * `generators` - map of generators, comes from generation request
///  * `all_descriptors` - all descriptors for the interaction 
///     (comes from plugin_configuration in the generation request)
/// 
/// # Returns 
/// Generated data for the interaction in form of `Body` struct which contains:
///  * `content_type` - content type of the generated message
///  * `content` - generated message bytes
///  * `content_type_hint` - always `ContentTypeHint::Binary`
#[instrument(level = "trace")]
fn generate_protobuf_contents(
  message_descriptor: &DescriptorProto,
  fields: &Vec<ProtobufField>,
  content_type: &ContentType,
  generators: &HashMap<String, proto::Generator>,
  all_descriptors: &FileDescriptorSet,
  mode: TestMode
) -> anyhow::Result<Body> {
  let mut message: DynamicMessage = DynamicMessage::new(fields, all_descriptors);
  let context = hashmap!{};

  let mut generator_map = hashmap!{};
  for (key, generator) in generators {
    let path = DocPath::new(key)?;
    let generator_values = generator.values.as_ref()
      .map(proto_struct_to_json)
      .unwrap_or_default();
    let generator = Generator::create(generator.r#type.as_str(), &generator_values)?;
    generator_map.insert(path, generator);
  }
  message.apply_generators(Some(&generator_map), &to_generator_mode(mode), &context)?;

  trace!(?message, "Writing generated message");
  let mut buffer = BytesMut::new();
  message.write_to(&mut buffer)?;
  Ok(Body {
    content_type: content_type.to_string(),
    content: Some(buffer.to_vec()),
    content_type_hint: i32::from(body::ContentTypeHint::Binary),
  })
}

fn to_generator_mode(mode: TestMode) -> GeneratorTestMode {
  match mode {
    TestMode::Unknown => GeneratorTestMode::Consumer,
    TestMode::Consumer => GeneratorTestMode::Consumer,
    TestMode::Provider => GeneratorTestMode::Provider
  }
}

#[tonic::async_trait]
impl PactPlugin for ProtobufPactPlugin {
  // Init plugin request. This will be called shortly after the plugin is started.
  // This will return the catalogue entries for the plugin
  async fn init_plugin(
    &self,
    request: Request<proto::InitPluginRequest>,
  ) -> Result<Response<proto::InitPluginResponse>, Status> {
    let message = request.get_ref();
    debug!("Init request from {}/{}", message.implementation, message.version);

    // Return an entry for a content matcher and content generator for Protobuf messages
    Ok(tonic::Response::new(proto::InitPluginResponse {
      catalogue: vec![
        proto::CatalogueEntry {
          r#type: EntryType::ContentMatcher as i32,
          key: "protobuf".to_string(),
          values: hashmap! {
            "content-types".to_string() => "application/protobuf;application/grpc".to_string()
          }
        },
        proto::CatalogueEntry {
          r#type: EntryType::ContentGenerator as i32,
          key: "protobuf".to_string(),
          values: hashmap! {
            "content-types".to_string() => "application/protobuf;application/grpc".to_string()
          }
        },
        proto::CatalogueEntry {
          r#type: EntryType::Transport as i32,
          key: "grpc".to_string(),
          values: hashmap! {}
        }
      ]
    }))
  }

  // Request from the plugin driver to update our copy of the plugin catalogue.
  async fn update_catalogue(
    &self,
    _request: Request<proto::Catalogue>,
  ) -> Result<Response<()>, tonic::Status> {
    debug!("Update catalogue request");

    // currently a no-op
    Ok(Response::new(()))
  }

  // Request to compare the contents and return the results of the comparison.
  // see compare_contents_impl() for details.
  async fn compare_contents(
    &self,
    request: Request<CompareContentsRequest>,
  ) -> Result<Response<CompareContentsResponse>, Status> {
    trace!("Got compare_contents request {:?}", request.get_ref());
    let request = request.get_ref();
    match self.compare_contents_impl(request) {
      Ok(result) => Ok(Response::new(result)),
      Err(err) => Self::error_response(err.to_string())
    }
  }

  // Request to configure the expected interaction for a consumer tests.
  async fn configure_interaction(
    &self,
    request: Request<proto::ConfigureInteractionRequest>,
  ) -> Result<Response<proto::ConfigureInteractionResponse>, Status> {
    let message = request.get_ref();
    debug!("Configure interaction request for content type '{}': {:?}", message.content_type, message);

    // Check for the "pact:proto" key
    let fields = message.contents_config.as_ref()
      .map(|config| config.fields.clone())
      .unwrap_or_default();
    let proto_file = match fields.get("pact:proto").and_then(proto_value_to_string) {
      Some(pf) => pf,
      None => {
        error!("Config item with key 'pact:proto' and path to the proto file is required");
        return Ok(Self::configure_interaction_error_response("Config item with key 'pact:proto' and path to the proto file is required"))
      }
    };

    // Check for either the message type or proto service
    if !fields.contains_key("pact:message-type") && !fields.contains_key("pact:proto-service") {
      let message = "Config item with key 'pact:message-type' and the protobuf message name or 'pact:proto-service' and the service name is required".to_string();
      error!("{}", message);
      return Ok(Self::configure_interaction_error_response(message))
    }

    let plugin_config = match self.setup_plugin_config(&fields) {
      Ok(config) => config,
      Err(err) => return Ok(Self::configure_interaction_error_response(err.to_string()))
    };
    // Make sure we can execute the protobuf compiler
    let protoc = match setup_protoc(&plugin_config, &self.additional_includes(&plugin_config)).await {
      Ok(protoc) => protoc,
      Err(err) => {
        error!("Failed to invoke protoc: {}", err);
        return Ok(Self::configure_interaction_error_response(format!("Failed to invoke protoc: {}", err)))
      }
    };

    // Process the proto file and configure the interaction
    match process_proto(proto_file, &protoc, &fields).await {
      Ok((interactions, plugin_config)) => {
        Ok(Response::new(proto::ConfigureInteractionResponse {
          interaction: interactions,
          plugin_configuration: Some(plugin_config),
          .. proto::ConfigureInteractionResponse::default()
        }))
      }
      Err(err) => {
        error!("Failed to process protobuf: {}", err);
        Ok(Self::configure_interaction_error_response(format!("Failed to process protobuf: {}", err)))
      }
    }
  }

  // Request to generate the contents of the interaction.
  // see generate_contents_impl() for details.
  async fn generate_content(
    &self,
    request: Request<GenerateContentRequest>,
  ) -> Result<Response<GenerateContentResponse>, Status> {
    let message = request.get_ref();
    debug!("Generate content request {:?}", message);
    match self.generate_contents_impl(message) {
      Ok(result) => Ok(Response::new(result)),
      Err(err) => Err(Status::aborted(err.to_string()))
    }
  }

  async fn start_mock_server(
    &self,
    request: Request<proto::StartMockServerRequest>,
  ) -> Result<Response<proto::StartMockServerResponse>, Status> {
    debug!("Received start mock server request");
    let request = request.get_ref();
    let pact = match parse_pact_from_request_json(request.pact.as_str(), "grpc:start_mock_server") {
      Ok(pact) => pact,
      Err(err) => return Ok(Self::start_mock_server_error(format!("Failed to parse Pact JSON: {}", err)))
    };

    trace!("Got pact {pact:?}");
    // Check for the plugin specific configuration for the Protobuf descriptors
    let plugin_config = match pact.plugin_data.iter().find(|pd| pd.name == "protobuf") {
      None => {
        error!("Provided Pact file does not have any Protobuf descriptors");
        return Ok(Self::start_mock_server_error("Provided Pact file does not have any Protobuf descriptors".to_string()))
      }
      Some(config) => config.clone()
    };

    let test_context: HashMap<String, Value> = match request.test_context.as_ref()
      .map(proto_struct_to_json)
      .unwrap_or_default() {
      Value::Object(map) => map.iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect(),
      _ => hashmap!{}
    };

    let grpc_mock_server = GrpcMockServer::new(pact, &plugin_config, test_context);
    let server_key = grpc_mock_server.server_key.clone();
    match grpc_mock_server.start_server(request.host_interface.as_str(), request.port, request.tls).await {
      Ok(address) => {
        info!("Started mock gRPC server on {}", address);
        Ok(tonic::Response::new(proto::StartMockServerResponse {
          response: Some(proto::start_mock_server_response::Response::Details(proto::MockServerDetails {
            key: server_key,
            port: address.port() as u32,
            address: format!("http://{}", address)
          }))
        }))
      }
      Err(err) => {
        error!("Failed to start gRPC mock server: {}", err);
        return Ok(Self::start_mock_server_error(format!("Failed to start gRPC mock server: {}", err)));
      }
    }
  }

  async fn shutdown_mock_server(
    &self,
    request: Request<proto::ShutdownMockServerRequest>,
  ) -> Result<Response<proto::ShutdownMockServerResponse>, Status> {
    let request = request.get_ref();
    let mut guard = MOCK_SERVER_STATE.lock().unwrap();
    if let Some((_, results)) = guard.get(&request.server_key) {
      let (ok, results) = Self::get_mock_server_results(results);
      guard.remove(&request.server_key);
      Ok(Response::new(proto::ShutdownMockServerResponse {
        ok,
        results
      }))
    } else {
      Ok(tonic::Response::new(proto::ShutdownMockServerResponse {
        ok: false,
        results: vec![
          proto::MockServerResult {
            error: format!("Did not find any mock server results for a server with ID {}", request.server_key),
            .. proto::MockServerResult::default()
          }
        ]
      }))
    }
  }

  async fn get_mock_server_results(
    &self,
    request: Request<proto::MockServerRequest>,
  ) -> Result<Response<proto::MockServerResults>, Status> {
    let request = request.get_ref();
    let guard = MOCK_SERVER_STATE.lock().unwrap();
    if let Some((_, results)) = guard.get(&request.server_key) {
      let (ok, results) = Self::get_mock_server_results(results);
      Ok(tonic::Response::new(proto::MockServerResults {
        ok,
        results
      }))
    } else {
      Ok(tonic::Response::new(proto::MockServerResults {
        ok: false,
        results: vec![
          proto::MockServerResult {
            error: format!("Did not find any mock server results for a server with ID {}", request.server_key),
            .. proto::MockServerResult::default()
          }
        ]
      }))
    }
  }

  /// Called as first step of provider verification flow.
  /// - loads the pact from request json
  /// - finds the interaction by key
  /// - looks up gRPC service, method and file descriptors for interaction
  /// - builds request body and metadata based on the loaded descriptors
  /// - adds `request-path` field to metadata in form of `/package.Service/Method`, same as what protoc compiler does.
  /// - returns the updated interaction with a built-out request
  async fn prepare_interaction_for_verification(
    &self,
    request: Request<proto::VerificationPreparationRequest>,
  ) -> Result<Response<proto::VerificationPreparationResponse>, Status> {
    debug!("Received prepare interaction for verification request");

    let request = request.get_ref();
    trace!("Got prepare_interaction_for_verification request {:?}", request);

    let pact = match parse_pact_from_request_json(request.pact.as_str(), "grpc:prepare_interaction_for_verification") {
      Ok(pact) => pact,
      Err(err) => return Ok(Self::verification_preparation_error_response(format!("Failed to parse Pact JSON: {}", err)))
    };

    let key = request.interaction_key.as_str();
    let interaction_by_id = lookup_interaction_by_id(key, &pact);
    let interaction = match interaction_by_id {
      Some(interaction) => match interaction.as_v4_sync_message() {
        Some(interaction) => interaction,
        None => return Ok(Self::verification_preparation_error_response(format!("gRPC interactions must be of type V4 synchronous message, got {}", interaction.type_of())))
      }
      None => {
        error!(?key, "Did not find an interaction that matches the given key");
        return Ok(Self::verification_preparation_error_response(format!("Did not find an interaction that matches the given key '{}'", key)));
      }
    };

    let (all_file_desc, service_desc, method_desc, file_desc) = match lookup_service_descriptors_for_interaction(&interaction, &pact) {
      Ok(values) => values,
      Err(err) => {
        return Ok(Self::verification_preparation_error_response(err.to_string()))
      }
    };

    let mut raw_request_body = interaction.request.contents.value().unwrap_or_default();
    let input_message = match find_message_descriptor_for_type(method_desc.input_type(), &all_file_desc) {
      Ok((message, _)) => message,
      Err(err) => {
        return Ok(Self::verification_preparation_error_response(err.to_string()))
      }
    };

    let config = proto_struct_to_map(&request.config.clone().unwrap_or_default());
    let test_context = config.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    let decoded_body = match decode_message(&mut raw_request_body, &input_message, &all_file_desc) {
      Ok(field_values) => {
        let mut message = DynamicMessage::new(&field_values, &all_file_desc);
        if let Err(err) = message.apply_generators(
          interaction.request.generators.categories.get(&GeneratorCategory::BODY),
          &GeneratorTestMode::Provider,
          &test_context
        ) {
          return Ok(Self::verification_preparation_error_response(err.to_string()));
        }
        message
      }
      Err(err) => {
        return Ok(Self::verification_preparation_error_response(err.to_string()));
      }
    };

    let request = Request::new(decoded_body.clone());

    let mut request_metadata = Self::setup_metadata(&interaction, &service_desc,
      &method_desc, &file_desc, request);
    Self::apply_generators_to_metadata(interaction, &test_context, &mut request_metadata);

    let mut buffer = BytesMut::new();
    if let Err(err) = decoded_body.write_to(&mut buffer) {
      return Ok(Self::verification_preparation_error_response(err.to_string()));
    }
    let integration_data = proto::InteractionData {
      body: Some(Body {
        content_type: "application/grpc".to_string(),
        content: Some(buffer.to_vec()),
        content_type_hint: ContentTypeHint::Binary as i32,
      }),
      metadata: request_metadata
    };

    trace!(integration_data = ?integration_data, "returning request data");
    Ok(Response::new(proto::VerificationPreparationResponse {
      response: Some(proto::verification_preparation_response::Response::InteractionData(integration_data)),
      .. proto::VerificationPreparationResponse::default()
    }))
  }

  /// Called as a second part in provider verification flow,
  /// after `prepare_interaction_for_verification` to verify the interaction
  /// After `prepare_interaction_for_verification` has built the request body and metadata,
  /// this function will use that data to actually make the gRPC call to the provider and verify response.
  /// Most of the work is done in `verification::verify_interaction` function.
  async fn verify_interaction(
    &self,
    request: Request<proto::VerifyInteractionRequest>
  ) -> Result<Response<proto::VerifyInteractionResponse>, Status> {
    debug!("Received verify interaction request");

    let request = request.get_ref();
    trace!("Got verify_interaction request {:?}", request);

    let pact = match parse_pact_from_request_json(request.pact.as_str(), "grpc:verify_interaction") {
      Ok(pact) => pact,
      Err(err) => return Ok(Self::verify_interaction_error(format!("Failed to parse Pact JSON: {}", err)))
    };

    let key = request.interaction_key.as_str();
    let interaction_by_id = lookup_interaction_by_id(key, &pact);
    // TODO: this lookup of interactions by id is duplicate with at least one other function
    let interaction = match interaction_by_id {
      Some(interaction) => match interaction.as_v4_sync_message() {
        Some(interaction) => interaction,
        None => return Ok(Self::verify_interaction_error(format!("gRPC interactions must be of type V4 synchronous message, got {}", interaction.type_of())))
      }
      None => {
        error!(?key, "Did not find an interaction that matches the given key");
        return Ok(Self::verify_interaction_error(format!("Did not find an interaction that matches the given key '{}'", key)))
      }
    };

    let body = match &request.interaction_data {
      Some(data) => match &data.body {
        Some(b) => match &b.content {
          Some(data) => OptionalBody::Present(Bytes::from(data.clone()), Some(ContentType::from(b.content_type.clone())), None),
          None => OptionalBody::Missing
        }
        None => OptionalBody::Missing
      }
      None => OptionalBody::Missing
    };
    let metadata = match &request.interaction_data {
      Some(data) => data.metadata.clone(),
      None => HashMap::default()
    };

    let config = request.config.as_ref()
      .map(proto_struct_to_map)
      .unwrap_or_default();
    match verify_interaction(&pact, &interaction, &body, &metadata, &config).await {
      Ok((result, output)) => {
        let results = result.iter()
          .flat_map(|result| match result {
            VerificationMismatchResult::Mismatches { mismatches, .. } => {
              mismatches.iter()
                .map(|mismatch| {
                  if let Mismatch::BodyMismatch { path, expected, actual, mismatch } = mismatch {
                    proto::VerificationResultItem {
                      result: Some(proto::verification_result_item::Result::Mismatch(proto::ContentMismatch {
                        expected: expected.as_ref().map(|b| b.to_vec()),
                        actual: actual.as_ref().map(|b| b.to_vec()),
                        mismatch: mismatch.clone(),
                        path: path.clone(),
                        .. proto::ContentMismatch::default()
                      })),
                      .. proto::VerificationResultItem::default()
                    }
                  } else {
                    proto::VerificationResultItem {
                      result: Some(proto::verification_result_item::Result::Mismatch(proto::ContentMismatch {
                        mismatch: mismatch.description(),
                        .. proto::ContentMismatch::default()
                      })),
                      .. proto::VerificationResultItem::default()
                    }
                  }
                })
                .collect()
            }
            VerificationMismatchResult::Error { error, .. } => {
              vec![proto::VerificationResultItem {
                result: Some(proto::verification_result_item::Result::Error(error.clone())),
                .. proto::VerificationResultItem::default()
              }]
            }
          })
          .collect();
        Ok(Response::new(proto::VerifyInteractionResponse {
          response: Some(proto::verify_interaction_response::Response::Result(proto::VerificationResult {
            success: result.is_empty(),
            mismatches: results,
            output,
            .. proto::VerificationResult::default()
          })),
          .. proto::VerifyInteractionResponse::default()
        }))
      }
      Err(err) => Ok(Self::verify_interaction_error(err.to_string()))
    }
  }
}

fn merge_value(initial: &Value, updated: &Value) -> anyhow::Result<Value> {
  match initial {
    Value::Array(a) => match updated {
      Value::Array(a2) => {
        let mut v = a.clone();
        v.extend_from_slice(a2.as_slice());
        Ok(Value::Array(v))
      }
      _ => {
        let mut v = a.clone();
        v.push(updated.clone());
        Ok(Value::Array(v))
      }
    }
    Value::Object(o) => match updated {
      Value::Null => Ok(initial.clone()),
      Value::Object(o2) => {
        let mut map = o.clone();
        for (k, v) in o2 {
          match map.get(k) {
            None => {
              map.insert(k.clone(), v.clone());
            },
            Some(val) => {
              map.insert(k.clone(), merge_value(val, v)?);
            },
          }
        }
        Ok(Value::Object(map))
      }
      _ => bail!("Can not merge config values: {:?} and {:?}", initial, updated)
    }
    _ => match updated {
      Value::Null => Ok(initial.clone()),
      _ => Ok(updated.clone())
    }
  }
}

fn get_interaction_config(config: &PluginConfiguration) -> anyhow::Result<BTreeMap<String, prost_types::Value>> {
  let interaction_config = config.interaction_configuration.as_ref()
    .map(|config| &config.fields);
  match interaction_config {
    Some(config) => Ok(config.clone()),
    None => {
      error!("Plugin configuration for the interaction is required");
      Err(anyhow!("Plugin configuration for the interaction is required"))
    }
  }
}

fn mismatch_to_proto_mismatch(mismatch: &Mismatch) -> proto::ContentMismatch {
  match mismatch {
    Mismatch::MethodMismatch { expected, actual, mismatch } => {
      proto::ContentMismatch {
        expected: Some(expected.as_bytes().to_vec()),
        actual: Some(actual.as_bytes().to_vec()),
        mismatch: mismatch.clone(),
        .. proto::ContentMismatch::default()
      }
    }
    Mismatch::PathMismatch { expected, actual, mismatch } => {
      proto::ContentMismatch {
        expected: Some(expected.as_bytes().to_vec()),
        actual: Some(actual.as_bytes().to_vec()),
        mismatch: mismatch.clone(),
        .. proto::ContentMismatch::default()
      }
    }
    Mismatch::StatusMismatch { expected, actual, mismatch } => {
      proto::ContentMismatch {
        expected: Some(expected.to_string().as_bytes().to_vec()),
        actual: Some(actual.to_string().as_bytes().to_vec()),
        mismatch: mismatch.clone(),
        .. proto::ContentMismatch::default()
      }
    }
    Mismatch::QueryMismatch { expected, actual, mismatch, .. } => {
      proto::ContentMismatch {
        expected: Some(expected.as_bytes().to_vec()),
        actual: Some(actual.as_bytes().to_vec()),
        mismatch: mismatch.clone(),
        .. proto::ContentMismatch::default()
      }
    }
    Mismatch::HeaderMismatch { expected, actual, mismatch, .. } => {
      proto::ContentMismatch {
        expected: Some(expected.as_bytes().to_vec()),
        actual: Some(actual.as_bytes().to_vec()),
        mismatch: mismatch.clone(),
        .. proto::ContentMismatch::default()
      }
    }
    Mismatch::BodyTypeMismatch { expected, actual, mismatch, .. } => {
      proto::ContentMismatch {
        expected: Some(expected.as_bytes().to_vec()),
        actual: Some(actual.as_bytes().to_vec()),
        mismatch: mismatch.clone(),
        .. proto::ContentMismatch::default()
      }
    }
    Mismatch::BodyMismatch { path, expected, actual, mismatch } => {
      proto::ContentMismatch {
        expected: expected.as_ref().map(|v| v.to_vec()),
        actual: actual.as_ref().map(|v| v.to_vec()),
        mismatch: mismatch.clone(),
        path: path.clone(),
        .. proto::ContentMismatch::default()
      }
    }
    Mismatch::MetadataMismatch { key, expected, actual, mismatch } => {
      proto::ContentMismatch {
        expected: Some(expected.as_bytes().to_vec()),
        actual: Some(actual.as_bytes().to_vec()),
        mismatch: mismatch.clone(),
        path: key.clone(),
        .. proto::ContentMismatch::default()
      }
    }
  }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
  use expectest::prelude::*;
  use maplit::{btreemap, hashmap};
  use pact_matching::{BodyMatchResult, Mismatch};
  use pact_plugin_driver::plugin_models::PactPluginManifest;
  use pact_plugin_driver::proto;
  use pact_plugin_driver::proto::catalogue_entry::EntryType;
  use pact_plugin_driver::proto::pact_plugin_server::PactPlugin;
  use pact_plugin_driver::proto::start_mock_server_response;
  use prost_types::value::Kind;
  use serde_json::{json, Map, Value};
  use tonic::Request;

  use crate::metadata::MetadataMatchResult;
  use crate::server::{merge_value, ProtobufPactPlugin};

  #[tokio::test]
  async fn init_plugin_test() {
    let plugin = ProtobufPactPlugin { manifest: Default::default() };
    let request = proto::InitPluginRequest {
      implementation: "test".to_string(),
      version: "0".to_string()
    };

    let response = plugin.init_plugin(Request::new(request)).await.unwrap();
    let response_message = response.get_ref();
    expect!(response_message.catalogue.iter()).to(have_count(3));

    let first = &response_message.catalogue.get(0).unwrap();
    expect!(first.key.as_str()).to(be_equal_to("protobuf"));
    expect!(first.r#type).to(be_equal_to(EntryType::ContentMatcher as i32));
    expect!(first.values.get("content-types")).to(be_some().value(&"application/protobuf;application/grpc".to_string()));

    let second = &response_message.catalogue.get(1).unwrap();
    expect!(second.key.as_str()).to(be_equal_to("protobuf"));
    expect!(second.r#type).to(be_equal_to(EntryType::ContentGenerator as i32));
    expect!(second.values.get("content-types")).to(be_some().value(&"application/protobuf;application/grpc".to_string()));

    let third = &response_message.catalogue.get(2).unwrap();
    expect!(third.key.as_str()).to(be_equal_to("grpc"));
    expect!(third.r#type).to(be_equal_to(EntryType::Transport as i32));
    expect!(third.values.iter()).to(be_empty());
  }

  #[tokio::test]
  async fn configure_interaction_test__with_no_config() {
    let plugin = ProtobufPactPlugin { manifest: Default::default() };
    let request = proto::ConfigureInteractionRequest {
      content_type: "text/test".to_string(),
      contents_config: Some(prost_types::Struct {
        fields: btreemap!{}
      })
    };

    let response = plugin.configure_interaction(Request::new(request)).await.unwrap();
    let response_message = response.get_ref();
    expect!(&response_message.error).to(
      be_equal_to("Config item with key 'pact:proto' and path to the proto file is required"));
  }

  #[tokio::test]
  async fn configure_interaction_test__with_missing_message_or_service_name() {
    let plugin = ProtobufPactPlugin { manifest: Default::default() };
    let request = proto::ConfigureInteractionRequest {
      content_type: "text/test".to_string(),
      contents_config: Some(prost_types::Struct {
        fields: btreemap!{
          "pact:proto".to_string() => prost_types::Value { kind: Some(prost_types::value::Kind::StringValue("test.proto".to_string())) }
        }
      })
    };

    let response = plugin.configure_interaction(Request::new(request)).await.unwrap();
    let response_message = response.get_ref();
    expect!(&response_message.error).to(
      be_equal_to("Config item with key 'pact:message-type' and the protobuf message name or 'pact:proto-service' and the service name is required"));
  }

  #[test]
  fn ProtobufPactPlugin__host_to_bind_to__default() {
    let plugin = ProtobufPactPlugin { manifest: Default::default() };
    expect!(plugin.host_to_bind_to()).to(be_none());
  }

  #[test]
  fn ProtobufPactPlugin__host_to_bind_to__with_string_value() {
    let manifest = PactPluginManifest {
      plugin_config: hashmap! {
        "hostToBindTo".to_string() => json!("127.0.1.1")
      },
      .. PactPluginManifest::default()
    };
    let plugin = ProtobufPactPlugin { manifest };
    expect!(plugin.host_to_bind_to()).to(be_some().value("127.0.1.1".to_string()));
  }

  #[test]
  fn ProtobufPactPlugin__host_to_bind_to__with_non_string_value() {
    let manifest = PactPluginManifest {
      plugin_config: hashmap! {
        "hostToBindTo".to_string() => json!("127")
      },
      .. PactPluginManifest::default()
    };
    let plugin = ProtobufPactPlugin { manifest };
    expect!(plugin.host_to_bind_to()).to(be_some().value("127".to_string()));
  }

  #[test]
  fn ProtobufPactPlugin__additional_includes__default() {
    let plugin = ProtobufPactPlugin { manifest: Default::default() };
    expect!(plugin.additional_includes(&hashmap!{}).iter()).to(be_empty());
  }

  #[test]
  fn ProtobufPactPlugin__additional_includes__with_string_value() {
    let manifest = PactPluginManifest::default();
    let plugin = ProtobufPactPlugin { manifest };
    let config = hashmap! {
      "additionalIncludes".to_string() => json!("/some/path")
    };
    expect!(plugin.additional_includes(&config)).to(be_equal_to(vec!["/some/path".to_string()]));
  }

  #[test]
  fn ProtobufPactPlugin__additional_includes__with_list_value() {
    let manifest = PactPluginManifest::default();
    let plugin = ProtobufPactPlugin { manifest };
    let config = hashmap! {
      "additionalIncludes".to_string() => json!(["/path1", "/path2"])
    };
    expect!(plugin.additional_includes(&config)).to(be_equal_to(vec![
      "/path1".to_string(),
      "/path2".to_string()
    ]));
  }

  #[test]
  fn ProtobufPactPlugin__additional_includes__with_non_string_values() {
    let manifest = PactPluginManifest::default();
    let plugin = ProtobufPactPlugin { manifest };
    let config = hashmap! {
      "additionalIncludes".to_string() => json!(["/path1", 200])
    };
    expect!(plugin.additional_includes(&config)).to(be_equal_to(vec![
      "/path1".to_string(),
      "200".to_string()
    ]));
  }

  #[test]
  fn ProtobufPactPlugin__setup_plugin_config__overwrites_manifest_config_from_test_config() {
    let manifest = PactPluginManifest {
      plugin_config: hashmap!{
        "protocVersion".to_string() => json!("1")
      },
      ..PactPluginManifest::default()
    };
    let plugin = ProtobufPactPlugin { manifest };
    let config = btreemap!{
      "pact:protobuf-config".to_string() => prost_types::Value { kind: Some(Kind::StructValue(
        prost_types::Struct {
          fields: btreemap!{
            "protocVersion".to_string() => prost_types::Value { kind: Some(Kind::StringValue("2".to_string())) }
          }
        }))
      }
    };
    expect!(plugin.setup_plugin_config(&config).unwrap()).to(be_equal_to(hashmap!{
      "protocVersion".to_string() => json!("2")
    }));
  }

  #[test_log::test]
  fn get_mock_server_results_test() {
    let mock_results = hashmap!{};
    let (ok, results) = ProtobufPactPlugin::get_mock_server_results(&mock_results);
    expect!(ok).to(be_true());
    expect!(results.len()).to(be_equal_to(0));
  }

  #[test_log::test]
  fn get_mock_server_results_test_with_no_mismatches() {
    let mock_results = hashmap!{
      "Req/Path1".to_string() => (1, vec![]),
      "Req/Path2".to_string() => (1, vec![ (BodyMatchResult::Ok, MetadataMatchResult::ok()) ]),
      "Req/Path3".to_string() => (1, vec![ (BodyMatchResult::Ok, MetadataMatchResult::ok()), (BodyMatchResult::Ok, MetadataMatchResult::ok()) ])
    };
    let (ok, results) = ProtobufPactPlugin::get_mock_server_results(&mock_results);
    expect!(ok).to(be_true());
    expect!(results.len()).to(be_equal_to(3));
  }

  #[test_log::test]
  fn get_mock_server_results_test_with_mismatches() {
    let mismatches = hashmap! {
      "$".to_string() => vec![]
    };
    let mismatches2 = hashmap! {
      "$".to_string() => vec![
        Mismatch::BodyMismatch {
          path: "$".to_string(),
          expected: None,
          actual: None,
          mismatch: "boom".to_string()
        }
      ]
    };
    let mock_results = hashmap!{
      "Req/Path1".to_string() => (1, vec![ (BodyMatchResult::BodyTypeMismatch {
        expected_type: "blob".to_string(),
        actual_type: "blob".to_string(),
        message: "it was a blob".to_string(),
        expected: None,
        actual: None
      }, MetadataMatchResult::ok()) ]),
      "Req/Path2".to_string() => (1, vec![ (BodyMatchResult::BodyMismatches(mismatches), MetadataMatchResult::ok()) ]),
      "Req/Path3".to_string() => (1, vec![ (BodyMatchResult::BodyMismatches(mismatches2), MetadataMatchResult::ok()) ])
    };
    let (ok, results) = ProtobufPactPlugin::get_mock_server_results(&mock_results);
    expect!(ok).to(be_false());
    expect!(results.len()).to(be_equal_to(3));
  }

  #[test_log::test]
  fn get_mock_server_results_test_with_a_mix_of_mismatches_and_no_mismatches() {
    let mismatches = hashmap! {
      "$".to_string() => vec![
        Mismatch::BodyMismatch {
          path: "$".to_string(),
          expected: None,
          actual: None,
          mismatch: "boom".to_string()
        }
      ]
    };
    let md_mismatch = vec![
      Mismatch::MetadataMismatch {
        key: "x-test".to_string(),
        expected: "A".to_string(),
        actual: "B".to_string(),
        mismatch: "Should never be B".to_string(),
      }
    ];
    let mock_results = hashmap!{
      "Req/Path1".to_string() => (1, vec![ (BodyMatchResult::BodyTypeMismatch {
        expected_type: "blob".to_string(),
        actual_type: "blob".to_string(),
        message: "it was a blob".to_string(),
        expected: None,
        actual: None
      }, MetadataMatchResult::ok()) ]),
      "Req/Path2".to_string() => (1, vec![ (BodyMatchResult::Ok, MetadataMatchResult::ok()) ]),
      "Req/Path3".to_string() => (1, vec![ (BodyMatchResult::BodyMismatches(mismatches), MetadataMatchResult::ok()) ]),
      "Req/Path4".to_string() => (1, vec![ (BodyMatchResult::Ok, MetadataMatchResult::mismatches(md_mismatch)) ])
    };
    let (ok, results) = ProtobufPactPlugin::get_mock_server_results(&mock_results);
    expect!(ok).to(be_false());
    expect!(results.len()).to(be_equal_to(4));
  }

  #[test_log::test]
  fn get_mock_server_results_test_with_a_path_with_no_requests() {
    let mock_results = hashmap!{
      "Req/Path1".to_string() => (0, vec![]),
      "Req/Path2".to_string() => (1, vec![ (BodyMatchResult::Ok, MetadataMatchResult::ok()) ]),
      "Req/Path3".to_string() => (1, vec![ (BodyMatchResult::Ok, MetadataMatchResult::ok()), (BodyMatchResult::Ok, MetadataMatchResult::ok()) ])
    };
    let (ok, results) = ProtobufPactPlugin::get_mock_server_results(&mock_results);
    expect!(ok).to(be_false());
    expect!(results.len()).to(be_equal_to(3));
    let path_1_result = results.iter().find(|it| it.path == "Req/Path1").unwrap().clone();
    expect!(path_1_result.error).to(be_equal_to("Did not receive any requests for path 'Req/Path1'"));
  }

  #[test_log::test(tokio::test)]
  async fn start_mock_server_returns_an_error_if_the_pact_json_is_invalid() {
    let plugin = ProtobufPactPlugin { manifest: Default::default() };
    let request = proto::StartMockServerRequest {
      host_interface: "".to_string(),
      port: 0,
      tls: false,
      pact: "I'm not JSON!".to_string(),
      .. proto::StartMockServerRequest::default()
    };
    let result = plugin.start_mock_server(Request::new(request)).await;
    let response = result.unwrap();
    if let Some(start_mock_server_response::Response::Error(message)) = &response.get_ref().response {
      expect!(message.starts_with("Failed to parse Pact JSON")).to(be_true());
    } else {
      panic!("Was expecting an error message");
    }
  }

  #[test_log::test(tokio::test)]
  async fn start_mock_server_returns_an_error_if_the_pact_does_not_have_any_descriptors() {
    let plugin = ProtobufPactPlugin { manifest: Default::default() };
    let request = proto::StartMockServerRequest {
      host_interface: "".to_string(),
      port: 0,
      tls: false,
      pact: "{}".to_string(),
      .. proto::StartMockServerRequest::default()
    };
    let result = plugin.start_mock_server(Request::new(request)).await;
    let response = result.unwrap();
    if let Some(start_mock_server_response::Response::Error(message)) = &response.get_ref().response {
      expect!(message).to(be_equal_to("Provided Pact file does not have any Protobuf descriptors"));
    } else {
      panic!("Was expecting an error message");
    }
  }

  #[test_log::test(tokio::test)]
  async fn shutdown_mock_server_returns_an_error_if_the_server_key_was_not_found() {
    let plugin = ProtobufPactPlugin { manifest: Default::default() };
    let request = proto::ShutdownMockServerRequest {
      server_key: "1234abcd".to_string(),
    };
    let result = plugin.shutdown_mock_server(Request::new(request)).await;
    let response = result.unwrap();
    let shutdown_response = response.get_ref();
    expect!(shutdown_response.ok).to(be_false());
    let error_response = shutdown_response.results.get(0).unwrap();
    expect!(&error_response.error).to(be_equal_to("Did not find any mock server results for a server with ID 1234abcd"));
  }

  #[test_log::test(tokio::test)]
  async fn get_mock_server_results_returns_an_error_if_the_server_key_was_not_found() {
    let plugin = ProtobufPactPlugin { manifest: Default::default() };
    let request = proto::MockServerRequest {
      server_key: "1234abcd".to_string(),
    };
    let result = plugin.get_mock_server_results(Request::new(request)).await;
    let response = result.unwrap();
    let get_mock_server_results_response = response.get_ref();
    expect!(get_mock_server_results_response.ok).to(be_false());
    let error_response = get_mock_server_results_response.results.get(0).unwrap();
    expect!(&error_response.error).to(be_equal_to("Did not find any mock server results for a server with ID 1234abcd"));
  }

  #[test_log::test]
  fn merge_value_test() {
    expect!(merge_value(&Value::Null, &Value::Null).unwrap()).to(be_equal_to(Value::Null));
    expect!(merge_value(&Value::Null, &Value::String("s".to_string())).unwrap()).to(be_equal_to(Value::String("s".to_string())));
    expect!(merge_value(&Value::Null, &Value::Bool(true)).unwrap()).to(be_equal_to(Value::Bool(true)));
    expect!(merge_value(&Value::Null, &json!(1)).unwrap()).to(be_equal_to(json!(1)));
    expect!(merge_value(&Value::Null, &Value::Array(vec![])).unwrap()).to(be_equal_to(Value::Array(vec![])));
    expect!(merge_value(&Value::Null, &Value::Object(Map::default())).unwrap()).to(be_equal_to(Value::Object(Map::default())));

    let s = Value::String("x".to_string());
    expect!(merge_value(&s, &Value::Null).unwrap()).to(be_equal_to(s.clone()));
    expect!(merge_value(&s, &Value::String("s".to_string())).unwrap()).to(be_equal_to(Value::String("s".to_string())));
    expect!(merge_value(&s, &Value::Bool(true)).unwrap()).to(be_equal_to(Value::Bool(true)));
    expect!(merge_value(&s, &json!(1)).unwrap()).to(be_equal_to(json!(1)));
    expect!(merge_value(&s, &Value::Array(vec![])).unwrap()).to(be_equal_to(Value::Array(vec![])));
    expect!(merge_value(&s, &Value::Object(Map::default())).unwrap()).to(be_equal_to(Value::Object(Map::default())));

    let b = Value::Bool(false);
    expect!(merge_value(&b, &Value::Null).unwrap()).to(be_equal_to(b.clone()));
    expect!(merge_value(&b, &Value::String("s".to_string())).unwrap()).to(be_equal_to(Value::String("s".to_string())));
    expect!(merge_value(&b, &Value::Bool(true)).unwrap()).to(be_equal_to(Value::Bool(true)));
    expect!(merge_value(&b, &json!(1)).unwrap()).to(be_equal_to(json!(1)));
    expect!(merge_value(&b, &Value::Array(vec![])).unwrap()).to(be_equal_to(Value::Array(vec![])));
    expect!(merge_value(&b, &Value::Object(Map::default())).unwrap()).to(be_equal_to(Value::Object(Map::default())));

    let n = json!(100.02);
    expect!(merge_value(&n, &Value::Null).unwrap()).to(be_equal_to(n.clone()));
    expect!(merge_value(&n, &Value::String("s".to_string())).unwrap()).to(be_equal_to(Value::String("s".to_string())));
    expect!(merge_value(&n, &Value::Bool(true)).unwrap()).to(be_equal_to(Value::Bool(true)));
    expect!(merge_value(&n, &json!(1)).unwrap()).to(be_equal_to(json!(1)));
    expect!(merge_value(&n, &Value::Array(vec![])).unwrap()).to(be_equal_to(Value::Array(vec![])));
    expect!(merge_value(&n, &Value::Object(Map::default())).unwrap()).to(be_equal_to(Value::Object(Map::default())));

    let a = Value::Array(vec![]);
    expect!(merge_value(&a, &Value::Null).unwrap()).to(be_equal_to(Value::Array(vec![Value::Null])));
    expect!(merge_value(&a, &Value::String("s".to_string())).unwrap()).to(be_equal_to(Value::Array(vec![Value::String("s".to_string())])));
    expect!(merge_value(&a, &Value::Bool(true)).unwrap()).to(be_equal_to(Value::Array(vec![Value::Bool(true)])));
    expect!(merge_value(&a, &json!(1)).unwrap()).to(be_equal_to(Value::Array(vec![json!(1)])));
    expect!(merge_value(&a, &Value::Object(Map::default())).unwrap()).to(be_equal_to(Value::Array(vec![Value::Object(Map::default())])));
    expect!(merge_value(&a, &Value::Array(vec![])).unwrap()).to(be_equal_to(Value::Array(vec![])));
    expect!(merge_value(&a, &Value::Array(vec![Value::Null])).unwrap()).to(be_equal_to(Value::Array(vec![Value::Null])));
    expect!(merge_value(&Value::Array(vec![Value::Null]), &Value::Array(vec![])).unwrap()).to(be_equal_to(Value::Array(vec![Value::Null])));
    expect!(merge_value(&Value::Array(vec![Value::Null]), &Value::Array(vec![Value::Bool(true)])).unwrap()).to(be_equal_to(Value::Array(vec![Value::Null, Value::Bool(true)])));
    expect!(merge_value(&Value::Array(vec![Value::Array(vec![Value::Null])]), &Value::Array(vec![Value::Array(vec![Value::Bool(true)])])).unwrap())
      .to(be_equal_to(Value::Array(vec![Value::Array(vec![Value::Null]), Value::Array(vec![Value::Bool(true)])])));

    let m = Value::Object(Map::default());
    expect!(merge_value(&m, &Value::Null).unwrap()).to(be_equal_to(m.clone()));
    expect!(merge_value(&m, &Value::String("s".to_string()))).to(be_err());
    expect!(merge_value(&m, &Value::Bool(true))).to(be_err());
    expect!(merge_value(&m, &json!(1))).to(be_err());
    expect!(merge_value(&m, &Value::Array(vec![]))).to(be_err());
    expect!(merge_value(&m, &Value::Object(Map::default())).unwrap()).to(be_equal_to(m.clone()));
    expect!(merge_value(&m, &json!({"test": "ok"})).unwrap()).to(be_equal_to(json!({"test": "ok"})));
    expect!(merge_value(&json!({"test": "ok"}), &Value::Object(Map::default())).unwrap()).to(be_equal_to(json!({"test": "ok"})));
    expect!(merge_value(&json!({"test": "ok"}), &json!({"other": "value"})).unwrap())
      .to(be_equal_to(json!({"test": "ok", "other": "value"})));
    expect!(merge_value(&json!({"test": "ok"}), &json!({"test": "not ok", "other": "value"})).unwrap())
      .to(be_equal_to(json!({"test": "not ok", "other": "value"})));
    expect!(merge_value(&json!({"additional": ["ok"]}), &json!({"additional": ["not ok"], "other": "value"})).unwrap())
      .to(be_equal_to(json!({"additional": ["ok", "not ok"], "other": "value"})));
  }

  #[test_log::test]
  fn lookup_message_key_returns_the_descriptor_key() {
    let config = btreemap!{
      "descriptorKey".to_string() => prost_types::Value { kind: Some(Kind::StringValue("1234567".to_string())) }
    };
    expect!(ProtobufPactPlugin::lookup_message_key(&config, &None))
      .to(be_ok().value("1234567".to_string()));
  }

  #[test_log::test]
  fn lookup_message_key_returns_an_error_when_there_is_no_descriptor_key() {
    expect!(ProtobufPactPlugin::lookup_message_key(
      &btreemap!{},
      &None
    )).to(be_err());
  }

  #[test_log::test]
  fn lookup_message_key_returns_the_descriptor_key_from_the_request_if_the_message_type_matches() {
    let config = btreemap!{
      "request".to_string() => prost_types::Value {
        kind: Some(Kind::StructValue(prost_types::Struct {
          fields: btreemap!{
            "descriptorKey".to_string() => prost_types::Value { kind: Some(Kind::StringValue("1234567".to_string())) },
            "message".to_string() => prost_types::Value { kind: Some(Kind::StringValue(".package.Type".to_string())) }
          }
        }))
      }
    };
    expect!(ProtobufPactPlugin::lookup_message_key(&config, &Some(".package.Type".to_string())))
      .to(be_ok().value("1234567".to_string()));
  }

  #[test_log::test]
  fn lookup_message_key_returns_an_error_if_the_request_message_type_does_not_match() {
    let config = btreemap!{
      "request".to_string() => prost_types::Value {
        kind: Some(Kind::StructValue(prost_types::Struct {
          fields: btreemap!{
            "descriptorKey".to_string() => prost_types::Value { kind: Some(Kind::StringValue("1234567".to_string())) },
            "message".to_string() => prost_types::Value { kind: Some(Kind::StringValue(".package.OtherType".to_string())) }
          }
        }))
      }
    };
    expect!(ProtobufPactPlugin::lookup_message_key(&config, &Some(".package.Type".to_string())))
      .to(be_err());
  }

  #[test_log::test]
  fn lookup_message_key_returns_the_descriptor_key_from_the_response_if_the_message_type_matches() {
    let config = btreemap!{
      "response".to_string() => prost_types::Value {
        kind: Some(Kind::StructValue(prost_types::Struct {
          fields: btreemap!{
            "descriptorKey".to_string() => prost_types::Value { kind: Some(Kind::StringValue("1234567".to_string())) },
            "message".to_string() => prost_types::Value { kind: Some(Kind::StringValue(".package.Type".to_string())) }
          }
        }))
      }
    };
    expect!(ProtobufPactPlugin::lookup_message_key(&config, &Some(".package.Type".to_string())))
      .to(be_ok().value("1234567".to_string()));
  }

  #[test_log::test]
  fn lookup_message_key_returns_an_error_if_the_response_message_type_does_not_match() {
    let config = btreemap!{
      "response".to_string() => prost_types::Value {
        kind: Some(Kind::StructValue(prost_types::Struct {
          fields: btreemap!{
            "descriptorKey".to_string() => prost_types::Value { kind: Some(Kind::StringValue("1234567".to_string())) },
            "message".to_string() => prost_types::Value { kind: Some(Kind::StringValue(".package.OtherType".to_string())) }
          }
        }))
      }
    };
    expect!(ProtobufPactPlugin::lookup_message_key(&config, &Some(".package.Type".to_string())))
      .to(be_err());
  }
}
