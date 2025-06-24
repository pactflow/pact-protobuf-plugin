use fake::Fake;
use fake::faker::address::en::CityName;
use fake::faker::lorem::en::Sentence;
use tonic::transport::Server;
use tracing::info;
use tracing_subscriber::{EnvFilter, FmtSubscriber};

use crate::pb::feature_service_server::FeatureServiceServer;
use crate::pb::{Feature, Point, SomeEnum};

pub mod pb {
  tonic::include_proto!("repeated_enum");
}

#[derive(Default)]
pub struct FeatureService {}

#[tonic::async_trait]
impl pb::feature_service_server::FeatureService for FeatureService {
  async fn get_feature(
    &self,
    request: tonic::Request<Point>
  ) -> Result<tonic::Response<Feature>, tonic::Status> {
    let request = request.get_ref();
    info!("Request for feature with location {}:{}", request.x, request.y);

    Ok(tonic::Response::new(Feature {
      name: CityName().fake(),
      description: Sentence(5..11).fake(),
      location: Some(request.clone()),
      some_enum: vec![SomeEnum::Value1 as i32, SomeEnum::Value2 as i32],
      .. Feature::default()
    }))
  }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
  let subscriber = FmtSubscriber::builder()
    .with_env_filter(EnvFilter::from_default_env())
    .pretty()
    .finish();
  if let Err(err) = tracing::subscriber::set_global_default(subscriber) {
    eprintln!("WARN: Failed to initialise global tracing subscriber - {err}");
  };

  let addr = "[::1]:11334".parse().unwrap();
  let service = FeatureService::default();

  info!("FeatureService listening on {}", addr);

  Server::builder()
    .add_service(FeatureServiceServer::new(service))
    .serve(addr)
    .await?;

  Ok(())
}
