use tonic::transport::Server;
use tracing::info;
use tracing_subscriber::{EnvFilter, FmtSubscriber};

use crate::pb::catalog_service_server::CatalogServiceServer;
use crate::pb::{CatalogRequest, CatalogResponse};

pub mod pb {
  tonic::include_proto!("catalog");
}

#[derive(Default)]
pub struct CatalogService {}

#[tonic::async_trait]
impl pb::catalog_service_server::CatalogService for CatalogService {
  async fn get_catalog_entry(
    &self,
    request: tonic::Request<CatalogRequest>,
  ) -> Result<tonic::Response<CatalogResponse>, tonic::Status> {
    let req = request.get_ref();
    info!("Request for catalog entry: shelf={}, section={}", req.shelf, req.section);

    // KEY: Provider returns MORE formats values than consumer expects.
    // Consumer only cares about "HARDCOVER" being present.
    // But pact exact matching will fail because the arrays don't match.
    Ok(tonic::Response::new(CatalogResponse {
      title: "REFERENCE".to_string(),
      languages: vec!["EN".to_string()],
      formats: vec![
        "HARDCOVER".to_string(),
        "PAPERBACK".to_string(),
        "AUDIOBOOK".to_string(),
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
  let service = CatalogService::default();

  info!("CatalogService listening on {}", addr);

  Server::builder()
    .add_service(CatalogServiceServer::new(service))
    .serve(addr)
    .await?;

  Ok(())
}
