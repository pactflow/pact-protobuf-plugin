use fake::Fake;
use fake::faker::name::en::{FirstName, LastName};
use tonic::transport::Server;
use tracing::info;
use tracing_subscriber::{EnvFilter, FmtSubscriber};

use crate::pb::user_service_server::UserServiceServer;
use crate::pb::{GetUserRequest, GetUserResponse};

pub mod pb {
    tonic::include_proto!("pactissue");
}

#[derive(Default)]
pub struct UserService {}

#[tonic::async_trait]
impl pb::user_service_server::UserService for UserService {
    async fn get_user(
        &self,
        request: tonic::Request<GetUserRequest>
    ) -> Result<tonic::Response<GetUserResponse>, tonic::Status> {
        let request = request.get_ref();
        info!("Request for user with ID {}", request.id);

        let first_name: String = FirstName().fake();
        let last_name: String = LastName().fake();
        Ok(tonic::Response::new(GetUserResponse {
            id: request.id.clone(),
            display_name: format!("{} {}", first_name, last_name),
            first_name,
            last_name,
            .. GetUserResponse::default()
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
    let service = UserService::default();

    info!("UserService listening on {}", addr);

    Server::builder()
      .add_service(UserServiceServer::new(service))
      .serve(addr)
      .await?;

    Ok(())
}
