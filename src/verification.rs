//! Module with all the functions to verify a gRPC interaction

use std::collections::HashMap;
use std::fmt::{Debug, Display, Formatter};
use std::str::FromStr;

use anyhow::anyhow;
use bytes::Bytes;
use pact_models::json_utils::{json_to_num, json_to_string};
use pact_models::prelude::OptionalBody;
use pact_models::prelude::v4::V4Pact;
use pact_models::v4::sync_message::SynchronousMessage;
use pact_plugin_driver::proto;
use pact_plugin_driver::utils::proto_value_to_string;
use pact_verifier::verification_result::{MismatchResult, VerificationExecutionResult};
use serde_json::Value;
use tonic::metadata::{Ascii, Binary, MetadataKey, MetadataValue};
use tonic::metadata::errors::InvalidMetadataKey;
use tonic::{Request, Response, Status};
use tower::ServiceExt;
use tracing::{debug, error, instrument, trace, warn};

use crate::dynamic_message::DynamicMessage;
use crate::utils::lookup_interaction_config;

#[derive(Debug)]
struct GrpcError {
  pub status: Status
}

impl Display for GrpcError {
  fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
    write!(f, "gRPC request failed with status {}", self.status)
  }
}

impl std::error::Error for GrpcError {}

/// Verify a gRPC interaction
#[instrument]
pub async fn verify_interaction(
  pact: &V4Pact,
  interaction: &SynchronousMessage,
  request_body: &OptionalBody,
  metadata: &HashMap<String, proto::MetadataValue>,
  config: &HashMap<String, Value>
) -> anyhow::Result<Vec<(String, MismatchResult)>> {
  match build_grpc_request(request_body, metadata) {
    Ok(request) => match make_grpc_request(request, config, metadata).await {
      Ok(response) => {
        debug!("Received response from gRPC server");
        trace!("gRPC metadata: {:?}", response.metadata());
        trace!("gRPC body: {} bytes", response.get_ref().len());
        Ok(vec![])
      }
      Err(err) => {
        error!("Received error response from gRPC provider - {:?}", err);

        if let Some(grpc_status) = err.downcast_ref::<GrpcError>() {
          trace!("gRPC message: {}", grpc_status.status.message());
          trace!("gRPC metadata: {:?}", grpc_status.status.metadata());
          Err(anyhow!(format!("gRPC error: status {}, message '{}'", grpc_status.status.code(),
            grpc_status.status.message())))
        } else {
          Err(anyhow!(err))
        }
      }
    }
    Err(err) => {
      error!("Failed to build gRPC request: {}", err);
      Err(anyhow!(err))
    }
  }
}

async fn make_grpc_request(
  request: Request<Bytes>,
  config: &HashMap<String, Value>,
  metadata: &HashMap<String, proto::MetadataValue>
) -> anyhow::Result<Response<Bytes>> {
  let host = config.get("host")
    .map(json_to_string)
    .unwrap_or("[::1]".to_string());
  let port = json_to_num(config.get("port").cloned())
    .unwrap_or(8080);
  let dest = format!("http://{}:{}", host, port);

  let request_path_data = metadata.get("request-path")
    .ok_or_else(|| anyhow!("INTERNAL ERROR: request-path is not set in the request metadata"))?;
  let request_path = match &request_path_data.value {
    Some(data) => match data {
      proto::metadata_value::Value::NonBinaryValue(value) => proto_value_to_string(value).unwrap_or_default(),
      _ => return Err(anyhow!("INTERNAL ERROR: request-path is not set correctly in the request metadata"))
    }
    None => return Err(anyhow!("INTERNAL ERROR: request-path is not set in the request metadata"))
  };
  let path = http::uri::PathAndQuery::try_from(request_path)?;

  debug!("Connecting to channel {}", dest);
  let mut conn = tonic::transport::Endpoint::new(dest)?.connect().await?;
  conn.ready().await?;

  debug!("Making gRPC request to {}", path);
  let codec = tonic::codec::ProstCodec::default();
  let mut grpc = tonic::client::Grpc::new(conn);
  grpc.unary(request, path, codec).await
    .map_err(|err| {
      error!("gRPC request failed with status {:?}", err);
      anyhow!(GrpcError { status: err })
    })
}

fn build_grpc_request(
  body: &OptionalBody,
  metadata: &HashMap<String, proto::MetadataValue>
) -> anyhow::Result<tonic::Request<Bytes>> {
  let mut request = tonic::Request::new(body.value().unwrap_or_default());
  let request_metadata = request.metadata_mut();
  for (key, md) in metadata {
    if key != "request-path" {
      if let Some(value) = &md.value {
        match value {
          proto::metadata_value::Value::NonBinaryValue(value) => {
            let str_value = proto_value_to_string(value).unwrap_or_default();
            match str_value.parse::<MetadataValue<Ascii>>() {
              Ok(value) => match key.parse::<MetadataKey<Ascii>>() {
                Ok(key) => {
                  request_metadata.insert(key, value.clone());
                }
                Err(err) => {
                  warn!("Protobuf metadata key '{}' is not valid - {}", key, err);
                }
              }
              Err(err) => {
                warn!("Could not parse Protobuf metadata value for key '{}' - {}", key, err);
              }
            }
          }
          proto::metadata_value::Value::BinaryValue(value) => match key.parse::<MetadataKey<Binary>>() {
            Ok(key) => {
              request_metadata.insert_bin(key, MetadataValue::from_bytes(value));
            }
            Err(err) => {
              warn!("Protobuf metadata key '{}' is not valid - {}", key, err);
            }
          }
        }
      }
    }
  }
  Ok(request)
}
