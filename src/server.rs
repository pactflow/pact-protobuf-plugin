//! Module provides the main gRPC server for the plugin process

use std::fs::File;
use std::io::BufReader;
use anyhow::anyhow;

use bytes::Bytes;
use log::{debug, error, info, trace};
use maplit::{btreemap, hashmap};
use pact_matching::{BodyMatchResult, Mismatch};
use pact_models::pact::load_pact_from_json;
use pact_plugin_driver::plugin_models::PactPluginManifest;
use pact_plugin_driver::proto;
use pact_plugin_driver::proto::catalogue_entry::EntryType;
use pact_plugin_driver::proto::pact_plugin_server::PactPlugin;
use pact_plugin_driver::utils::proto_value_to_string;
use prost::Message;
use prost_types::FileDescriptorSet;
use prost_types::value::Kind;
use serde_json::Value;
use uuid::Uuid;
use crate::matching::{match_message, match_service};
use crate::mock_server::GrpcMockServer;

use crate::protobuf::process_proto;
use crate::protoc::setup_protoc;

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
            "content-types".to_string() => "application/protobuf".to_string()
          }
        },
        proto::CatalogueEntry {
          r#type: EntryType::ContentGenerator as i32,
          key: "protobuf".to_string(),
          values: hashmap! {
            "content-types".to_string() => "application/protobuf".to_string()
          }
        },
        proto::CatalogueEntry {
          r#type: EntryType::MockServer as i32,
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
        return Ok(tonic::Response::new(proto::CompareContentsResponse {
          error: "Plugin configuration for the interaction is required".to_string(),
          .. proto::CompareContentsResponse::default()
        }))
      }
    };

    // From the plugin configuration for the interaction, get the descriptor key. This key is used
    // to lookup the encoded Protobuf descriptors in the Pact level plugin configuration
    let message_key = match interaction_config.get("descriptorKey").map(proto_value_to_string).flatten() {
      Some(key) => key,
      None => {
        error!("Plugin configuration item with key 'descriptorKey' is required");
        return Ok(tonic::Response::new(proto::CompareContentsResponse {
          error: "Plugin configuration item with key 'descriptorKey' is required".to_string(),
          .. proto::CompareContentsResponse::default()
        }))
      }
    };
    debug!("compare_contents: message_key = {}", message_key);

    let pact_configuration = plugin_configuration.pact_configuration.unwrap_or_default();
    debug!("Pact level configuration keys: {:?}", pact_configuration.fields.keys());

    let config_for_interaction = match pact_configuration.fields.get(&message_key)
      .map(|config| match &config.kind {
        Some(Kind::StructValue(s)) => s.fields.clone(),
        _ => btreemap!{}
      }) {
      Some(config) => config,
      None => {
        error!("Did not find the Protobuf config for key {}", message_key);
        return Ok(tonic::Response::new(proto::CompareContentsResponse {
          error: format!("Did not find the Protobuf config for key {}", message_key),
          .. proto::CompareContentsResponse::default()
        }))
      }
    };

    // From the plugin configuration for the interaction, there should be either a message type name
    // or a service name. Check for either.
    let message = interaction_config.get("message").map(proto_value_to_string).flatten();
    let service = interaction_config.get("service").map(proto_value_to_string).flatten();
    if message.is_none() && service.is_none() {
      error!("Plugin configuration item with key 'message' or 'service' is required");
      return Ok(tonic::Response::new(proto::CompareContentsResponse {
        error: "Plugin configuration item with key 'message' or 'service' is required".to_string(),
        .. proto::CompareContentsResponse::default()
      }))
    }

    // Get the encoded Protobuf descriptors from the Pact level configuration
    let descriptor_bytes_encoded = config_for_interaction.get("protoDescriptors")
      .map(proto_value_to_string)
      .flatten()
      .unwrap_or_default();
    if descriptor_bytes_encoded.is_empty() {
      error!("Plugin configuration item with key '{}' is required", message_key);
      return Ok(tonic::Response::new(proto::CompareContentsResponse {
        error: format!("Plugin configuration item with key '{}' is required", message_key),
        .. proto::CompareContentsResponse::default()
      }))
    }

    // The descriptor bytes will be base 64 encoded.
    let descriptor_bytes = match base64::decode(descriptor_bytes_encoded) {
      Ok(bytes) => Bytes::from(bytes),
      Err(err) => {
        error!("Failed to decode the Protobuf descriptor - {}", err);
        return Ok(tonic::Response::new(proto::CompareContentsResponse {
          error: format!("Failed to decode the Protobuf descriptor - {}", err),
          .. proto::CompareContentsResponse::default()
        }))
      }
    };
    debug!("Protobuf file descriptor set is {} bytes", descriptor_bytes.len());

    // Get an MD5 hash of the bytes to check that it matches the descriptor key
    let digest = md5::compute(&descriptor_bytes);
    let descriptor_hash = format!("{:x}", digest);
    if descriptor_hash != message_key {
      error!("Protobuf descriptors checksum failed. Expected {} but got {}", message_key, descriptor_hash);
      return Ok(tonic::Response::new(proto::CompareContentsResponse {
        error: format!("Protobuf descriptors checksum failed. Expected {} but got {}", message_key, descriptor_hash),
        .. proto::CompareContentsResponse::default()
      }))
    }

    // Decode the Protobuf descriptors
    let descriptors = match FileDescriptorSet::decode(descriptor_bytes) {
      Ok(descriptors) => descriptors,
      Err(err) => {
        error!("Failed to decode the Protobuf descriptors - {}", err);
        return Ok(tonic::Response::new(proto::CompareContentsResponse {
          error: format!("Failed to decode the Protobuf descriptors - {}", err),
          .. proto::CompareContentsResponse::default()
        }))
      }
    };

    let result = if let Some(message_name) = message {
      debug!("Received compareContents request for message {}", message_name);
      match_message(message_name.as_str(), &descriptors, request)
    } else if let Some(service_name) = service {
      debug!("Received compareContents request for service {}", service_name);
      match_service(service_name.as_str(), &descriptors, request)
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
      Err(err) => {
        Ok(tonic::Response::new(proto::CompareContentsResponse {
          error: format!("Failed to compare the Protobuf messages - {}", err),
          .. proto::CompareContentsResponse::default()
        }))
      }
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
    Ok(tonic::Response::new(proto::GenerateContentResponse {
      contents: message.contents.clone()
    }))
  }

  async fn start_mock_server(
    &self,
    request: tonic::Request<proto::StartMockServerRequest>,
  ) -> Result<tonic::Response<proto::StartMockServerResponse>, tonic::Status> {
    let request = request.get_ref();

    // Parse the Pact JSON string
    let json: Value = match serde_json::from_str(request.pact.as_str()) {
      Ok(json) => json,
      Err(err) => {
        error!("Failed to parse Pact JSON: {}", err);
        return Ok(tonic::Response::new(proto::StartMockServerResponse {
          response: Some(proto::start_mock_server_response::Response::Error(format!("Failed to parse Pact JSON: {}", err))),
          .. proto::StartMockServerResponse::default()
        }));
      }
    };
    // Load the Pact model from the JSON
    let pact = match load_pact_from_json("grpc:start_mock_server", &json) {
      Ok(pact) => match pact.as_v4_pact() {
        Ok(pact) => pact,
        Err(err) => {
          error!("Failed to parse Pact JSON, not a V4 Pact: {}", err);
          return Ok(tonic::Response::new(proto::StartMockServerResponse {
            response: Some(proto::start_mock_server_response::Response::Error(format!("Failed to parse Pact JSON, not a V4 Pact: {}", err))),
            .. proto::StartMockServerResponse::default()
          }));
        }
      },
      Err(err) => {
        error!("Failed to parse Pact JSON to a V4 Pact: {}", err);
        return Ok(tonic::Response::new(proto::StartMockServerResponse {
          response: Some(proto::start_mock_server_response::Response::Error(format!("Failed to parse Pact JSON: {}", err))),
          .. proto::StartMockServerResponse::default()
        }));
      }
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
    unimplemented!()
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
    expect!(response_message.catalogue.iter()).to(have_count(2));

    let first = &response_message.catalogue.get(0).unwrap();
    expect!(first.key.as_str()).to(be_equal_to("protobuf"));
    expect!(first.r#type).to(be_equal_to(EntryType::ContentMatcher as i32));
    expect!(first.values.get("content-types")).to(be_some().value(&"application/protobuf".to_string()));

    let second = &response_message.catalogue.get(1).unwrap();
    expect!(second.key.as_str()).to(be_equal_to("protobuf"));
    expect!(second.r#type).to(be_equal_to(EntryType::ContentGenerator as i32));
    expect!(second.values.get("content-types")).to(be_some().value(&"application/protobuf".to_string()));
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
