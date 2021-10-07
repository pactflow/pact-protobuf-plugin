//! Module provides the main gRPC server for the plugin process

use crate::proto;
use crate::proto::pact_plugin_server::PactPlugin;

/// Plugin gRPC server implementation
#[derive(Debug, Default)]
pub struct ProtobufPactPlugin {}

#[tonic::async_trait]
impl PactPlugin for ProtobufPactPlugin {
  // Init plugin request. This will be called shortly after the plugin is started.
  async fn init_plugin(
    &self,
    request: tonic::Request<proto::InitPluginRequest>,
  ) -> Result<tonic::Response<proto::InitPluginResponse>, tonic::Status> {
    unimplemented!()
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

  // Request to configure the expected interaction for a consumer test.
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
