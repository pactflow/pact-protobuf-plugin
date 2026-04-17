use tonic::transport::Server;
use tracing::info;
use tracing_subscriber::{EnvFilter, FmtSubscriber};

use crate::pb::env_metadata_service_server::EnvMetadataServiceServer;
use crate::pb::{EnvMetadataRequest, EnvMetadataResponse};

pub mod pb {
  tonic::include_proto!("envmetadata");
}

#[derive(Default)]
pub struct EnvMetadataService {}

#[tonic::async_trait]
impl pb::env_metadata_service_server::EnvMetadataService for EnvMetadataService {
  async fn get_env_metadata(
    &self,
    request: tonic::Request<EnvMetadataRequest>,
  ) -> Result<tonic::Response<EnvMetadataResponse>, tonic::Status> {
    let req = request.get_ref();
    info!("Request for env metadata: cloud={}, region={}", req.cloud, req.region);

    // KEY: Provider returns MORE networking values than consumer expects.
    // Consumer only cares about "PUBLIC" being present.
    // But pact exact matching will fail because the arrays don't match.
    Ok(tonic::Response::new(EnvMetadataResponse {
      type_name: "DEDICATED".to_string(),
      durability: vec!["LOW".to_string()],
      networking: vec![
        "PUBLIC".to_string(),
        "PRIVATE_LINK".to_string(),
        "TRANSIT_GATEWAY".to_string(),
      ],
    }))
  }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
  let subscriber = FmtSubscriber::builder()
    .with_env_filter(EnvFilter::from_default_env())
    .finish();
  if let Err(err) = tracing::subscriber::set_global_default(subscriber) {
    eprintln!("WARN: Failed to initialise global tracing subscriber - {err}");
  };

  let addr = "[::1]:11335".parse().unwrap();
  let service = EnvMetadataService::default();

  info!("EnvMetadataService listening on {}", addr);

  Server::builder()
    .add_service(EnvMetadataServiceServer::new(service))
    .serve(addr)
    .await?;

  Ok(())
}
