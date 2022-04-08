//! Module with all the functions to verify a gRPC interaction

use std::collections::HashMap;
use std::fmt::{Debug, Display, Formatter};
use std::str::FromStr;

use anyhow::anyhow;
use bytes::{Buf, BufMut, Bytes, BytesMut};
use maplit::hashmap;
use pact_matching::BodyMatchResult;
use pact_models::json_utils::{json_to_num, json_to_string};
use pact_models::prelude::OptionalBody;
use pact_models::prelude::v4::V4Pact;
use pact_models::v4::sync_message::SynchronousMessage;
use pact_models::content_types::ContentType;
use pact_plugin_driver::proto;
use pact_plugin_driver::utils::proto_value_to_string;
use pact_verifier::verification_result::{MismatchResult, VerificationExecutionResult};
use prost_types::{DescriptorProto, FileDescriptorSet, MethodDescriptorProto, ServiceDescriptorProto};
use serde_json::Value;
use tonic::metadata::{Ascii, Binary, MetadataKey, MetadataMap, MetadataValue};
use tonic::metadata::errors::InvalidMetadataKey;
use tonic::{Request, Response, Status};
use tonic::codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use tower::ServiceExt;
use tracing::{debug, error, instrument, trace, warn};

use crate::dynamic_message::{DynamicMessage, DynamicMessageEncoder, PactCodec};
use crate::matching::match_service;
use crate::message_decoder::decode_message;
use crate::utils::{find_message_type_by_name, last_name, lookup_interaction_config, lookup_service_descriptors_for_interaction};

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
) -> anyhow::Result<Vec<MismatchResult>> {
  debug!("Verifying interaction {}", interaction);
  debug!("interaction={:?}", interaction);

  let (file_desc, service_desc, method_desc, _) = lookup_service_descriptors_for_interaction(interaction, &pact)?;
  let input_message_name = method_desc.input_type.clone().unwrap_or_default();
  let input_message = find_message_type_by_name(last_name(input_message_name.as_str()), &file_desc)?;
  let output_message_name = method_desc.output_type.clone().unwrap_or_default();
  let output_message = find_message_type_by_name(last_name(output_message_name.as_str()), &file_desc)?;

  match build_grpc_request(request_body, metadata, &file_desc, &input_message) {
    Ok(request) => match make_grpc_request(request, config, metadata, &file_desc, &input_message, &output_message, interaction).await {
      Ok(response) => {
        debug!("Received response from gRPC server - {:?}", response);
        let response_metadata = response.metadata();
        let mut body = response.get_ref();
        trace!("gRPC metadata: {:?}", response_metadata);
        trace!("gRPC body: {:?}", body);
        verify_response(body, response_metadata, pact, interaction,
                        &file_desc, &service_desc, &method_desc)
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

fn verify_response(
  response_body: &DynamicMessage,
  response_metadata: &MetadataMap,
  pact: &V4Pact,
  interaction: &SynchronousMessage,
  file_desc: &FileDescriptorSet,
  service_desc: &ServiceDescriptorProto,
  method_desc: &MethodDescriptorProto
) -> anyhow::Result<Vec<MismatchResult>> {
  let response = interaction.response.first().cloned()
    .unwrap_or_default();
  let expected_body = response.contents.value();

  let mut results = vec![];
  if let Some(mut expected_body) = expected_body {
    let ct = ContentType {
      main_type: "application".into(),
      sub_type: "grpc".into(),
      .. ContentType::default()
    };
    let mut actual_body = BytesMut::new();
    response_body.write_to(&mut actual_body);
    match match_service(
      service_desc.name.clone().unwrap_or_default().as_str(),
      method_desc.name.clone().unwrap_or_default().as_str(),
      &file_desc,
      &mut expected_body,
      &mut actual_body.freeze(),
      &response.matching_rules.rules_for_category("body").unwrap_or_default(),
      true,
      &ct
    ) {
      Ok(result) => {
        debug!("Match service result: {:?}", result);
        match result {
          BodyMatchResult::Ok => {}
          BodyMatchResult::BodyTypeMismatch { message, .. } => {
            results.push(MismatchResult::Error { error: message, interaction_id: interaction.id.clone() });
          }
          BodyMatchResult::BodyMismatches(mismatches) => {
            for (_, mismatches) in mismatches {
              results.push(MismatchResult::Mismatches { mismatches, interaction_id: interaction.id.clone() });
            }
          }
        }
      }
      Err(err) => {
        error!("Verifying the response failed with an error - {}", err);
        results.push(MismatchResult::Error { error: err.to_string(), interaction_id: interaction.id.clone() })
      }
    }
  }

  Ok(results)
}

async fn make_grpc_request(
  request: Request<DynamicMessage>,
  config: &HashMap<String, Value>,
  metadata: &HashMap<String, proto::MetadataValue>,
  file_desc: &FileDescriptorSet,
  input_desc: &DescriptorProto,
  output_desc: &DescriptorProto,
  interaction: &SynchronousMessage
) -> anyhow::Result<Response<DynamicMessage>> {
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
  let codec = PactCodec::new(file_desc, output_desc, input_desc, interaction);
  let mut grpc = tonic::client::Grpc::new(conn);
  grpc.unary(request, path, codec).await
    .map_err(|err| {
      error!("gRPC request failed with status {:?}", err);
      anyhow!(GrpcError { status: err })
    })
}

fn build_grpc_request(
  body: &OptionalBody,
  metadata: &HashMap<String, proto::MetadataValue>,
  file_desc: &FileDescriptorSet,
  input_desc: &DescriptorProto
) -> anyhow::Result<tonic::Request<DynamicMessage>> {
  let mut bytes = body.value().unwrap_or_default();
  let message_fields = decode_message(&mut bytes, input_desc, file_desc)?;
  let mut request = tonic::Request::new(DynamicMessage::new(input_desc, &message_fields));
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
