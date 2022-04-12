//! Module provides the main gRPC server for the plugin process

use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;

use anyhow::anyhow;
use bytes::{Bytes, BytesMut};
use log::{debug, error, info, trace};
use maplit::hashmap;
use pact_matching::{BodyMatchResult, Mismatch};
use pact_models::matchingrules::MatchingRule;
use pact_models::path_exp::DocPath;
use pact_models::prelude::{ContentType, MatchingRuleCategory, OptionalBody, RuleLogic};
use pact_plugin_driver::plugin_models::PactPluginManifest;
use pact_plugin_driver::proto;
use pact_plugin_driver::proto::body::ContentTypeHint;
use pact_plugin_driver::proto::catalogue_entry::EntryType;
use pact_plugin_driver::proto::pact_plugin_server::PactPlugin;
use pact_plugin_driver::proto::CompareContentsResponse;
use pact_plugin_driver::utils::{proto_struct_to_json, proto_struct_to_map, proto_value_to_json, proto_value_to_string, to_proto_value};
use pact_verifier::verification_result::MismatchResult;
use tonic::metadata::KeyAndValueRef;
use tonic::{Response, Status};
use crate::dynamic_message::DynamicMessage;

use crate::matching::{match_message, match_service};
use crate::message_decoder::decode_message;
use crate::mock_server::{GrpcMockServer, MOCK_SERVER_STATE};
use crate::protobuf::process_proto;
use crate::protoc::setup_protoc;
use crate::utils::{find_message_type_by_name, get_descriptors_for_interaction, last_name, lookup_interaction_by_id, lookup_service_descriptors_for_interaction, parse_pact_from_request_json};
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

  fn error_response<E>(err: E) -> Result<Response<CompareContentsResponse>, Status>
    where E: Into<String> {
    Ok(tonic::Response::new(proto::CompareContentsResponse {
      error: err.into(),
      ..proto::CompareContentsResponse::default()
    }))
  }
}

#[tonic::async_trait]
impl PactPlugin for ProtobufPactPlugin {
  // Init plugin request. This will be called shortly after the plugin is started.
  // This will return the catalogue entries for the plugin
  async fn init_plugin(
    &self,
    request: tonic::Request<proto::InitPluginRequest>,
  ) -> Result<tonic::Response<proto::InitPluginResponse>, tonic::Status> {
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
    _request: tonic::Request<proto::Catalogue>,
  ) -> Result<tonic::Response<()>, tonic::Status> {
    debug!("Update catalogue request");

    // currently a no-op
    Ok(tonic::Response::new(()))
  }

  // Request to compare the contents and return the results of the comparison.
  async fn compare_contents(
    &self,
    request: tonic::Request<proto::CompareContentsRequest>,
  ) -> Result<tonic::Response<proto::CompareContentsResponse>, tonic::Status> {
    trace!("Got compare_contents request {:?}", request.get_ref());

    let request = request.get_ref();

    // Check for the plugin specific configuration for the interaction
    let plugin_configuration = request.plugin_configuration.clone().unwrap_or_default();
    let interaction_config = plugin_configuration.interaction_configuration.as_ref()
      .map(|config| &config.fields);
    let interaction_config = match interaction_config {
      Some(config) => config,
      None => {
        error!("Plugin configuration for the interaction is required");
        return Self::error_response("Plugin configuration for the interaction is required")
      }
    };

    // From the plugin configuration for the interaction, get the descriptor key. This key is used
    // to lookup the encoded Protobuf descriptors in the Pact level plugin configuration
    let message_key = match interaction_config.get("descriptorKey").and_then(proto_value_to_string) {
      Some(key) => key,
      None => {
        error!("Plugin configuration item with key 'descriptorKey' is required");
        return Self::error_response("Plugin configuration item with key 'descriptorKey' is required");
      }
    };
    debug!("compare_contents: message_key = {}", message_key);

    // From the plugin configuration for the interaction, there should be either a message type name
    // or a service name. Check for either.
    let message = interaction_config.get("message").and_then(proto_value_to_string);
    let service = interaction_config.get("service").and_then(proto_value_to_string);
    if message.is_none() && service.is_none() {
      error!("Plugin configuration item with key 'message' or 'service' is required");
      return Self::error_response("Plugin configuration item with key 'message' or 'service' is required");
    }

    let pact_configuration = plugin_configuration.pact_configuration.unwrap_or_default();
    debug!("Pact level configuration keys: {:?}", pact_configuration.fields.keys());

    let config_for_interaction = pact_configuration.fields.iter()
      .map(|(key, config)| (key.clone(), proto_value_to_json(config)))
      .collect();
    let descriptors = match get_descriptors_for_interaction(message_key.as_str(), &config_for_interaction) {
      Ok(descriptors) => descriptors,
      Err(err) => return Self::error_response(err.to_string())
    };

    let mut expected_body = request.expected.as_ref()
      .and_then(|body| body.content.clone().map(Bytes::from))
      .unwrap_or_default();
    let mut actual_body = request.actual.as_ref()
      .map(|body| body.content.clone().map(Bytes::from))
      .flatten()
      .unwrap_or_default();
    let mut matching_rules = MatchingRuleCategory::empty("body");
    for (key, rules) in &request.rules {
      for rule in &rules.rule {
        let values = rule.values.as_ref().map(proto_struct_to_json).unwrap_or_default();
        let doc_path = match DocPath::new(key) {
          Ok(path) => path,
          Err(err) => return Self::error_response(err.to_string())
        };
        let matching_rule = match MatchingRule::create(&rule.r#type, &values) {
          Ok(rule) => rule,
          Err(err) => return Self::error_response(err.to_string())
        };
        matching_rules.add_rule(doc_path, matching_rule, RuleLogic::And);
      }
    }

    let result = if let Some(message_name) = message {
      debug!("Received compareContents request for message {}", message_name);
      match_message(
        message_name.as_str(),
        &descriptors,
        &mut expected_body,
        &mut actual_body,
        &matching_rules,
        request.allow_unexpected_keys
      )
    } else if let Some(service_name) = service {
      debug!("Received compareContents request for service {}", service_name);
      let (service, method) = match service_name.split_once('/') {
        Some(result) => result,
        None => return Self::error_response(format!("Service name '{}' is not valid, it should be of the form <SERVICE>/<METHOD>", service_name))
      };
      let content_type = request.expected.as_ref().map(|body| body.content_type.clone())
        .unwrap_or_default();
      let expected_content_type = match ContentType::parse(content_type.as_str()) {
        Ok(ct) => ct,
        Err(err) => return Self::error_response(format!("Expected content type is not set or not valid - {}", err))
      };
      match_service(
        service,
        method,
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

    return match result {
      Ok(result) => match result {
        BodyMatchResult::Ok => Ok(tonic::Response::new(proto::CompareContentsResponse::default())),
        BodyMatchResult::BodyTypeMismatch { message, expected_type, actual_type, .. } => {
          error!("Got a BodyTypeMismatch - {}", message);
          Ok(tonic::Response::new(proto::CompareContentsResponse {
            type_mismatch: Some(proto::ContentTypeMismatch {
              expected: expected_type,
              actual: actual_type
            }),
            .. proto::CompareContentsResponse::default()
          }))
        }
        BodyMatchResult::BodyMismatches(mismatches) => {
          Ok(tonic::Response::new(proto::CompareContentsResponse {
            results: mismatches.iter().map(|(k, v)| {
              (k.clone(), proto::ContentMismatches {
                mismatches: v.iter().map(mismatch_to_proto_mismatch).collect()
              })
            }).collect(),
            .. proto::CompareContentsResponse::default()
          }))
        }
      }
      Err(err) => Self::error_response(format!("Failed to compare the Protobuf messages - {}", err))
    }
  }

  // Request to configure the expected interaction for a consumer tests.
  async fn configure_interaction(
    &self,
    request: tonic::Request<proto::ConfigureInteractionRequest>,
  ) -> Result<tonic::Response<proto::ConfigureInteractionResponse>, tonic::Status> {
    let message = request.get_ref();
    debug!("Configure interaction request for content type '{}'", message.content_type);

    // Check for the "pact:proto" key
    let fields = message.contents_config.as_ref().map(|config| config.fields.clone()).unwrap_or_default();
    let proto_file = match fields.get("pact:proto").and_then(proto_value_to_string) {
      Some(pf) => pf,
      None => {
        error!("Config item with key 'pact:proto' and path to the proto file is required");
        return Ok(tonic::Response::new(proto::ConfigureInteractionResponse {
          error: "Config item with key 'pact:proto' and path to the proto file is required".to_string(),
          .. proto::ConfigureInteractionResponse::default()
        }))
      }
    };

    // Check for either the message type or proto service
    if !fields.contains_key("pact:message-type") && !fields.contains_key("pact:proto-service") {
      let message = "Config item with key 'pact:message-type' and the protobuf message name or 'pact:proto-service' and the service name is required".to_string();
      error!("{}", message);
      return Ok(tonic::Response::new(proto::ConfigureInteractionResponse {
        error: message,
        .. proto::ConfigureInteractionResponse::default()
      }))
    }

    // Make sure we can execute the protobuf compiler
    let protoc = match setup_protoc(&self.manifest.plugin_config).await {
      Ok(protoc) => protoc,
      Err(err) => {
        error!("Failed to invoke protoc: {}", err);
        return Ok(tonic::Response::new(proto::ConfigureInteractionResponse {
          error: format!("Failed to invoke protoc: {}", err),
          .. proto::ConfigureInteractionResponse::default()
        }))
      }
    };

    // Process the proto file and configure the interaction
    match process_proto(proto_file, &protoc, &fields).await {
      Ok((interactions, plugin_config)) => {
        Ok(tonic::Response::new(proto::ConfigureInteractionResponse {
          interaction: interactions,
          plugin_configuration: Some(plugin_config),
          .. proto::ConfigureInteractionResponse::default()
        }))
      }
      Err(err) => {
        error!("Failed to process protobuf: {}", err);
        Ok(tonic::Response::new(proto::ConfigureInteractionResponse {
          error: format!("Failed to process protobuf: {}", err),
          .. proto::ConfigureInteractionResponse::default()
        }))
      }
    }
  }

  // Request to generate the contents of the interaction.
  async fn generate_content(
    &self,
    request: tonic::Request<proto::GenerateContentRequest>,
  ) -> Result<tonic::Response<proto::GenerateContentResponse>, tonic::Status> {
    debug!("Generate content request");
    let message = request.get_ref();
    // TODO: apply any generators here
    Ok(tonic::Response::new(proto::GenerateContentResponse {
      contents: message.contents.clone()
    }))
  }

  async fn start_mock_server(
    &self,
    request: tonic::Request<proto::StartMockServerRequest>,
  ) -> Result<tonic::Response<proto::StartMockServerResponse>, tonic::Status> {
    debug!("Received start mock server request");
    let request = request.get_ref();
    let pact = match parse_pact_from_request_json(request.pact.as_str(), "grpc:start_mock_server") {
      Ok(pact) => pact,
      Err(err) => return Ok(tonic::Response::new(proto::StartMockServerResponse {
        response: Some(proto::start_mock_server_response::Response::Error(format!("Failed to parse Pact JSON: {}", err))),
        ..proto::StartMockServerResponse::default()
      }))
    };

    trace!("Got pact {pact:?}");
    // Check for the plugin specific configuration for the Protobuf descriptors
    let plugin_config = match pact.plugin_data.iter().find(|pd| pd.name == "protobuf") {
      None => {
        error!("Provided Pact file does not have any Protobuf descriptors");
        return Ok(tonic::Response::new(proto::StartMockServerResponse {
          response: Some(proto::start_mock_server_response::Response::Error("Provided Pact file does not have any Protobuf descriptors".to_string())),
          .. proto::StartMockServerResponse::default()
        }))
      }
      Some(config) => config.clone()
    };

    let grpc_mock_server = GrpcMockServer::new(pact, &plugin_config);
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
        return Ok(tonic::Response::new(proto::StartMockServerResponse {
          response: Some(proto::start_mock_server_response::Response::Error(format!("Failed to start gRPC mock server: {}", err))),
          .. proto::StartMockServerResponse::default()
        }));
      }
    }
  }

  async fn shutdown_mock_server(
    &self,
    request: tonic::Request<proto::ShutdownMockServerRequest>,
  ) -> Result<tonic::Response<proto::ShutdownMockServerResponse>, tonic::Status> {
    let request = request.get_ref();
    let mut guard = MOCK_SERVER_STATE.lock().unwrap();
    if let Some((_, results)) = guard.get(&request.server_key) {
      let ok = results.iter().all(|(_, r)| *r == BodyMatchResult::Ok);
      let results = results.iter().map(|(path, r)| {
        proto::MockServerResult {
          path: path.clone(),
          mismatches: r.mismatches().iter().map(|m| {
            match m {
              Mismatch::BodyMismatch { path, mismatch, expected, actual } => {
                proto::ContentMismatch {
                  expected: expected.as_ref().map(|d| d.to_vec()),
                  actual: actual.as_ref().map(|d| d.to_vec()),
                  mismatch: mismatch.clone(),
                  path: path.clone(),
                  .. proto::ContentMismatch::default()
                }
              }
              _ => proto::ContentMismatch {
                mismatch: m.description(),
                .. proto::ContentMismatch::default()
              }
            }
          }).collect(),
          .. proto::MockServerResult::default()
        }
      }).collect();
      guard.remove(&request.server_key);
      Ok(tonic::Response::new(proto::ShutdownMockServerResponse {
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

  async fn prepare_interaction_for_verification(
    &self,
    request: tonic::Request<proto::VerificationPreparationRequest>,
  ) -> Result<tonic::Response<proto::VerificationPreparationResponse>, tonic::Status> {
    debug!("Received prepare interaction for verification request");

    let request = request.get_ref();
    trace!("Got prepare_interaction_for_verification request {:?}", request);

    let pact = match parse_pact_from_request_json(request.pact.as_str(), "grpc:prepare_interaction_for_verification") {
      Ok(pact) => pact,
      Err(err) => return Ok(tonic::Response::new(proto::VerificationPreparationResponse {
        response: Some(proto::verification_preparation_response::Response::Error(format!("Failed to parse Pact JSON: {}", err))),
        .. proto::VerificationPreparationResponse::default()
      }))
    };

    let interaction = match lookup_interaction_by_id(request.interaction_key.as_str(), &pact) {
      Ok(interaction) => match interaction.as_v4_sync_message() {
        Some(interaction) => interaction,
        None => return Ok(tonic::Response::new(proto::VerificationPreparationResponse {
          response: Some(proto::verification_preparation_response::Response::Error(format!("gRPC interactions must be of type V4 synchronous message, got {}", interaction.type_of()))),
          ..proto::VerificationPreparationResponse::default()
        }))
      }
      Err(err) => {
        return Ok(tonic::Response::new(proto::VerificationPreparationResponse {
          response: Some(proto::verification_preparation_response::Response::Error(err.to_string())),
          ..proto::VerificationPreparationResponse::default()
        }))
      }
    };

    let (file_desc, service_desc, method_desc, package) = match lookup_service_descriptors_for_interaction(&interaction, &pact) {
      Ok(values) => values,
      Err(err) => {
        return Ok(tonic::Response::new(proto::VerificationPreparationResponse {
          response: Some(proto::verification_preparation_response::Response::Error(err.to_string())),
          ..proto::VerificationPreparationResponse::default()
        }))
      }
    };

    let mut raw_request_body = interaction.request.contents.value().unwrap_or_default();
    let input_message_name = method_desc.input_type.clone().unwrap_or_default();
    let input_message = match find_message_type_by_name(last_name(input_message_name.as_str()), &file_desc) {
      Ok(message) => message,
      Err(err) => {
        return Ok(tonic::Response::new(proto::VerificationPreparationResponse {
          response: Some(proto::verification_preparation_response::Response::Error(err.to_string())),
          ..proto::VerificationPreparationResponse::default()
        }))
      }
    };

    // TODO: use any generators here
    let decoded_body = match decode_message(&mut raw_request_body, &input_message, &file_desc) {
      Ok(message) => DynamicMessage::new(&input_message, &message),
      Err(err) => {
        return Ok(tonic::Response::new(proto::VerificationPreparationResponse {
          response: Some(proto::verification_preparation_response::Response::Error(err.to_string())),
          ..proto::VerificationPreparationResponse::default()
        }))
      }
    };
    let request = tonic::Request::new(decoded_body.clone());

    let mut request_metadata: HashMap<String, proto::MetadataValue> = interaction.request.metadata.iter()
      .map(|(k, v)| (k.clone(), proto::MetadataValue {
        value: Some(proto::metadata_value::Value::NonBinaryValue(to_proto_value(v)))
      }))
      .collect();

    let path = format!("/{}.{}/{}", package, service_desc.name.unwrap_or_default(), method_desc.name.unwrap_or_default());
    request_metadata.insert("request-path".to_string(), proto::MetadataValue {
      value: Some(proto::metadata_value::Value::NonBinaryValue(prost_types::Value {
        kind: Some(prost_types::value::Kind::StringValue(path))
      }))
    });

    for entry in request.metadata().iter() {
      match entry {
        KeyAndValueRef::Ascii(k, v) => {
          request_metadata.insert(k.to_string(), proto::MetadataValue {
            value: Some(proto::metadata_value::Value::NonBinaryValue(prost_types::Value {
              kind: Some(prost_types::value::Kind::StringValue(v.to_str().unwrap_or_default().to_string()))
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

    let mut buffer = BytesMut::new();
    if let Err(err) = decoded_body.write_to(&mut buffer) {
      return Ok(tonic::Response::new(proto::VerificationPreparationResponse {
        response: Some(proto::verification_preparation_response::Response::Error(err.to_string())),
        ..proto::VerificationPreparationResponse::default()
      }))
    }
    let integration_data = proto::InteractionData {
      body: Some(proto::Body {
        content_type: "application/grpc".to_string(),
        content: Some(buffer.to_vec()),
        content_type_hint: ContentTypeHint::Binary as i32,
      }),
      metadata: request_metadata
    };

    Ok(tonic::Response::new(proto::VerificationPreparationResponse {
      response: Some(proto::verification_preparation_response::Response::InteractionData(integration_data)),
      .. proto::VerificationPreparationResponse::default()
    }))
  }

  async fn verify_interaction(
    &self,
    request: tonic::Request<proto::VerifyInteractionRequest>
  ) -> Result<tonic::Response<proto::VerifyInteractionResponse>, tonic::Status> {
    debug!("Received verify interaction request");

    let request = request.get_ref();
    trace!("Got verify_interaction request {:?}", request);

    let pact = match parse_pact_from_request_json(request.pact.as_str(), "grpc:verify_interaction") {
      Ok(pact) => pact,
      Err(err) => return Ok(tonic::Response::new(proto::VerifyInteractionResponse {
        response: Some(proto::verify_interaction_response::Response::Error(format!("Failed to parse Pact JSON: {}", err))),
        .. proto::VerifyInteractionResponse::default()
      }))
    };

    let interaction = match lookup_interaction_by_id(request.interaction_key.as_str(), &pact) {
      Ok(interaction) => match interaction.as_v4_sync_message() {
        Some(interaction) => interaction,
        None => return Ok(tonic::Response::new(proto::VerifyInteractionResponse {
          response: Some(proto::verify_interaction_response::Response::Error(format!("gRPC interactions must be of type V4 synchronous message, got {}", interaction.type_of()))),
          .. proto::VerifyInteractionResponse::default()
        }))
      }
      Err(err) => {
        return Ok(tonic::Response::new(proto::VerifyInteractionResponse {
          response: Some(proto::verify_interaction_response::Response::Error(err.to_string())),
          ..proto::VerifyInteractionResponse::default()
        }))
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

    let config = request.config.as_ref().map(proto_struct_to_map).unwrap_or_default();
    match verify_interaction(&pact, &interaction, &body, &metadata, &config).await {
      Ok(result) => {
        let results = result.iter()
          .flat_map(|result| match result {
            MismatchResult::Mismatches { mismatches, .. } => {
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
            MismatchResult::Error { error, .. } => {
              vec![proto::VerificationResultItem {
                result: Some(proto::verification_result_item::Result::Error(error.clone())),
                .. proto::VerificationResultItem::default()
              }]
            }
          })
          .collect();
        Ok(tonic::Response::new(proto::VerifyInteractionResponse {
          response: Some(proto::verify_interaction_response::Response::Result(proto::VerificationResult {
            success: result.is_empty(),
            mismatches: results,
            .. proto::VerificationResult::default()
          })),
          .. proto::VerifyInteractionResponse::default()
        }))
      }
      Err(err) => {
        Ok(tonic::Response::new(proto::VerifyInteractionResponse {
          response: Some(proto::verify_interaction_response::Response::Error(err.to_string())),
          .. proto::VerifyInteractionResponse::default()
        }))
      }
    }
  }
}

fn mismatch_to_proto_mismatch(mismatch: &Mismatch) -> proto::ContentMismatch {
  match mismatch {
    Mismatch::MethodMismatch { expected, actual } => {
      proto::ContentMismatch {
        expected: Some(expected.as_bytes().to_vec()),
        actual: Some(actual.as_bytes().to_vec()),
        mismatch: "Method mismatch".to_string(),
        ..proto::ContentMismatch::default()
      }
    }
    Mismatch::PathMismatch { expected, actual, mismatch } => {
      proto::ContentMismatch {
        expected: Some(expected.as_bytes().to_vec()),
        actual: Some(actual.as_bytes().to_vec()),
        mismatch: mismatch.clone(),
        ..proto::ContentMismatch::default()
      }
    }
    Mismatch::StatusMismatch { expected, actual, mismatch } => {
      proto::ContentMismatch {
        expected: Some(expected.to_string().as_bytes().to_vec()),
        actual: Some(actual.to_string().as_bytes().to_vec()),
        mismatch: mismatch.clone(),
        ..proto::ContentMismatch::default()
      }
    }
    Mismatch::QueryMismatch { expected, actual, mismatch, .. } => {
      proto::ContentMismatch {
        expected: Some(expected.as_bytes().to_vec()),
        actual: Some(actual.as_bytes().to_vec()),
        mismatch: mismatch.clone(),
        ..proto::ContentMismatch::default()
      }
    }
    Mismatch::HeaderMismatch { expected, actual, mismatch, .. } => {
      proto::ContentMismatch {
        expected: Some(expected.as_bytes().to_vec()),
        actual: Some(actual.as_bytes().to_vec()),
        mismatch: mismatch.clone(),
        ..proto::ContentMismatch::default()
      }
    }
    Mismatch::BodyTypeMismatch { expected, actual, mismatch, .. } => {
      proto::ContentMismatch {
        expected: Some(expected.as_bytes().to_vec()),
        actual: Some(actual.as_bytes().to_vec()),
        mismatch: mismatch.clone(),
        ..proto::ContentMismatch::default()
      }
    }
    Mismatch::BodyMismatch { path, expected, actual, mismatch } => {
      proto::ContentMismatch {
        expected: expected.as_ref().map(|v| v.to_vec()),
        actual: actual.as_ref().map(|v| v.to_vec()),
        mismatch: mismatch.clone(),
        path: path.clone(),
        ..proto::ContentMismatch::default()
      }
    }
    Mismatch::MetadataMismatch { key, expected, actual, mismatch } => {
      proto::ContentMismatch {
        expected: Some(expected.as_bytes().to_vec()),
        actual: Some(actual.as_bytes().to_vec()),
        mismatch: mismatch.clone(),
        path: key.clone(),
        ..proto::ContentMismatch::default()
      }
    }
  }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
  use expectest::prelude::*;
  use maplit::btreemap;
  use pact_plugin_driver::proto;
  use pact_plugin_driver::proto::catalogue_entry::EntryType;
  use pact_plugin_driver::proto::pact_plugin_server::PactPlugin;
  use tonic::Request;

  use crate::server::ProtobufPactPlugin;

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
}
