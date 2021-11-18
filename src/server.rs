//! Module provides the main gRPC server for the plugin process

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use anyhow::anyhow;
use itertools::Itertools;
use log::{debug, error};
use maplit::hashmap;
use pact_plugin_driver::plugin_models::PactPluginManifest;
use pact_plugin_driver::proto;
use pact_plugin_driver::proto::catalogue_entry::EntryType;
use pact_plugin_driver::proto::pact_plugin_server::PactPlugin;
use pact_plugin_driver::utils::proto_value_to_string;
use tonic::Response;

use crate::protoc::{Protoc, setup_protoc};

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

  async fn process_proto(&self, proto_file: String, protoc: &Protoc, fields: BTreeMap<String, prost_types::Value>) -> anyhow::Result<()> {
    debug!("Parsing proto file '{}'", proto_file);
    let descriptors = protoc.parse_proto_file(Path::new(proto_file.as_str())).await?;
    debug!("Parsed proto file OK, file descriptors = {:?}", descriptors.file.iter().map(|file| file.name.as_ref()).collect_vec());

    /*
    val descriptorBytes = protoResult.toByteArray()
        logger.debug { "Protobuf file descriptor set is ${descriptorBytes.size} bytes" }
        val digest = MessageDigest.getInstance("MD5")
        digest.update(descriptorBytes)
        val descriptorHash = BaseEncoding.base16().lowerCase().encode(digest.digest());
     */

    /*


        logger.debug { "Parsed proto file OK, file descriptors = ${protoResult.fileList.map { it.name }}" }

        val fileDescriptors = protoResult.fileList.associateBy { it.name }
        val fileProtoDesc = fileDescriptors[protoFile.fileName.toString()]
        if (fileProtoDesc == null) {
          logger.error { "Did not find a file proto descriptor for $protoFile" }
          return Plugin.ConfigureInteractionResponse.newBuilder()
            .setError("Did not find a file proto descriptor for $protoFile")
            .build()
        }

        if (logger.isTraceEnabled) {
          logger.trace { "All message types in proto descriptor" }
          for (messageType in fileProtoDesc.messageTypeList) {
            logger.trace { messageType.toString() }
          }
        }

        val interactions: MutableList<Plugin.InteractionResponse.Builder> = mutableListOf()

        if (config.containsKey("pact:message-type")) {
          val message = config["pact:message-type"]!!.stringValue
          when (val result = configureProtobufMessage(message, config, fileProtoDesc, fileDescriptors, protoFile)) {
            is Ok -> {
              val builder = result.value
              val pluginConfigurationBuilder = builder.pluginConfigurationBuilder
              pluginConfigurationBuilder.interactionConfigurationBuilder
                .putFields("message", Value.newBuilder().setStringValue(message).build())
                .putFields("descriptorKey", Value.newBuilder().setStringValue(descriptorHash.toString()).build())
              interactions.add(builder)
            }
            is Err -> {
              return Plugin.ConfigureInteractionResponse.newBuilder()
                .setError(result.error)
                .build()
            }
          }
        } else {
          val serviceName = config["pact:proto-service"]!!.stringValue
          when (val result = configureProtobufService(serviceName, config, fileProtoDesc, fileDescriptors, protoFile)) {
            is Ok -> {
              val (requestPart, responsePart) = result.value
              val pluginConfigurationBuilder = requestPart.pluginConfigurationBuilder
              pluginConfigurationBuilder.interactionConfigurationBuilder
                .putFields("service", Value.newBuilder().setStringValue(serviceName).build())
                .putFields("descriptorKey", Value.newBuilder().setStringValue(descriptorHash.toString()).build())
              interactions.add(requestPart)
              interactions.add(responsePart)
            }
            is Err -> {
              return Plugin.ConfigureInteractionResponse.newBuilder()
                .setError(result.error)
                .build()
            }
          }
        }

        val builder = Plugin.ConfigureInteractionResponse.newBuilder()
        val fileContents = protoFile.toFile().readText()
        val valueBuilder = Value.newBuilder()
        val structValueBuilder = valueBuilder.structValueBuilder
        structValueBuilder
          .putAllFields(
            mapOf(
              "protoFile" to Value.newBuilder().setStringValue(fileContents).build(),
              "protoDescriptors" to Value.newBuilder()
                .setStringValue(Base64.getEncoder().encodeToString(descriptorBytes))
                .build()
            )
          )
          .build()
        val pluginConfigurationBuilder = builder.pluginConfigurationBuilder
        pluginConfigurationBuilder.pactConfigurationBuilder.putAllFields(
          mapOf(descriptorHash.toString() to valueBuilder.build())
        )

        for (result in interactions) {
          logger.debug { "Adding interaction $result" }
          builder.addInteraction(result)
        }

        return builder.build()
     */

    Err(anyhow!("todo"))
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
    Ok(Response::new(proto::InitPluginResponse {
      catalogue: vec![
        proto::CatalogueEntry {
          r#type: EntryType::ContentMatcher as i32,
          key: "prototype".to_string(),
          values: hashmap! {
            "content-types".to_string() => "application/protobuf".to_string()
          }
        },
        proto::CatalogueEntry {
          r#type: EntryType::ContentGenerator as i32,
          key: "prototype".to_string(),
          values: hashmap! {
            "content-types".to_string() => "application/protobuf".to_string()
          }
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
    Ok(Response::new(()))
  }

  // Request to compare the contents and return the results of the comparison.
  async fn compare_contents(
    &self,
    request: tonic::Request<proto::CompareContentsRequest>,
  ) -> Result<tonic::Response<proto::CompareContentsResponse>, tonic::Status> {
    unimplemented!()
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
    let proto_file = match fields.get("pact:proto").and_then(|file| proto_value_to_string(file)) {
      Some(pf) => pf,
      None => {
        error!("Config item with key 'pact:proto' and path to the proto file is required");
        return Ok(Response::new(proto::ConfigureInteractionResponse {
          error: "Config item with key 'pact:proto' and path to the proto file is required".to_string(),
          .. proto::ConfigureInteractionResponse::default()
        }))
      }
    };

    // Check for either the message type or proto service
    if !fields.contains_key("pact:message-type") && !fields.contains_key("pact:proto-service") {
      let message = "Config item with key 'pact:message-type' and the protobuf message name or 'pact:proto-service' and the service name is required".to_string();
      error!("{}", message);
      return Ok(Response::new(proto::ConfigureInteractionResponse {
        error: message,
        .. proto::ConfigureInteractionResponse::default()
      }))
    }

    // Make sure we can execute the protobuf compiler
    let protoc = match setup_protoc(&self.manifest.plugin_config).await {
      Ok(protoc) => protoc,
      Err(err) => {
        error!("Failed to invoke protoc: {}", err);
        return Ok(Response::new(proto::ConfigureInteractionResponse {
          error: format!("Failed to invoke protoc: {}", err),
          .. proto::ConfigureInteractionResponse::default()
        }))
      }
    };

    match self.process_proto(proto_file, &protoc, fields).await {
      Ok(_) => {
        todo!()
      }
      Err(err) => {
        error!("Failed to process protobuf: {}", err);
        return Ok(Response::new(proto::ConfigureInteractionResponse {
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
    unimplemented!()
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
    expect!(first.key.as_str()).to(be_equal_to("prototype"));
    expect!(first.r#type).to(be_equal_to(EntryType::ContentMatcher as i32));
    expect!(first.values.get("content-types")).to(be_some().value(&"application/protobuf".to_string()));

    let second = &response_message.catalogue.get(1).unwrap();
    expect!(second.key.as_str()).to(be_equal_to("prototype"));
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
