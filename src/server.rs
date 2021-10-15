//! Module provides the main gRPC server for the plugin process

use log::debug;
use maplit::hashmap;
use pact_plugin_driver::proto;
use pact_plugin_driver::proto::catalogue_entry::EntryType;
use pact_plugin_driver::proto::pact_plugin_server::PactPlugin;
use tonic::Response;

/// Plugin gRPC server implementation
#[derive(Debug, Default)]
pub struct ProtobufPactPlugin {}

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
    unimplemented!()
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
    unimplemented!()
  }

  // Request to generate the contents of the interaction.
  async fn generate_content(
    &self,
    request: tonic::Request<proto::GenerateContentRequest>,
  ) -> Result<tonic::Response<proto::GenerateContentResponse>, tonic::Status> {
    unimplemented!()
  }
}
