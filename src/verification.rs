//! Module with all the functions to verify a gRPC interaction

use std::collections::{BTreeMap, HashMap};
use std::fmt::{Debug, Display, Formatter};

use ansi_term::Colour::{Green, Red};
use ansi_term::Style;
use anyhow::anyhow;
use bytes::BytesMut;
use maplit::hashmap;
use pact_matching::{BodyMatchResult, CoreMatchingContext, DiffConfig, Mismatch};
use pact_models::json_utils::{json_to_num, json_to_string};
use pact_models::path_exp::DocPath;
use pact_models::prelude::OptionalBody;
use pact_models::prelude::v4::V4Pact;
use pact_models::v4::message_parts::MessageContents;
use pact_models::v4::sync_message::SynchronousMessage;
use pact_plugin_driver::proto;
use pact_plugin_driver::utils::proto_value_to_string;
use pact_verifier::verification_result::VerificationMismatchResult;
use prost_types::{DescriptorProto, FileDescriptorSet, MethodDescriptorProto};
use serde_json::Value;
use tonic::{Request, Response, Status};
use tonic::metadata::{Ascii, Binary, MetadataKey, MetadataMap, MetadataValue};
use tower::ServiceExt;
use tracing::{debug, error, instrument, trace, warn};

use crate::dynamic_message::{DynamicMessage, PactCodec};
use crate::matching::match_message;
use crate::message_decoder::decode_message;
use crate::metadata::{compare_metadata, grpc_status, MetadataMatchResult};
use crate::utils::{
  build_expectations,
  find_message_descriptor_for_type,
  lookup_service_descriptors_for_interaction
};

#[derive(Debug)]
struct GrpcError {
  pub status: Status
}

impl Display for GrpcError {
  fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
    write!(f, "gRPC request failed {}", self.status)
  }
}

impl std::error::Error for GrpcError {}

/// Verify a gRPC interaction.
/// 
/// This function will make a call to the gRPC server with the request body and metadata
/// and call `verify_response` to compare the response with the expected response in the interaction.
/// 
/// # Arguments
/// * `pact` - Pact to verify against, full pact config containing all interactions and plugin configuration
/// * `interaction` - Interaction to verify
/// * `request_body` - Request body to send to the gRPC server; created in `server::prepare_interaction_for_verification`
/// * `metadata` - Metadata to send to the gRPC server; created in `server::prepare_interaction_for_verification`
/// * `config` - host and port to connect to the gRPC server
/// 
/// # Returns
/// A tuple with a vector of verification results and a vector of strings 
/// with the human-readable output of the verification
pub async fn verify_interaction(
  pact: &V4Pact,
  interaction: &SynchronousMessage,
  request_body: &OptionalBody,
  metadata: &HashMap<String, proto::MetadataValue>,
  config: &BTreeMap<String, Value>
) -> anyhow::Result<(Vec<VerificationMismatchResult>, Vec<String>)> {
  debug!("Verifying interaction {}", interaction);
  trace!(?interaction, ?metadata, ?config, ?request_body, ?pact);

  let (all_file_descriptors, service_desc, method_desc, _) = 
    lookup_service_descriptors_for_interaction(interaction, pact)?;
  
  let input_message_name = method_desc.input_type.clone().unwrap_or_default();
  let (input_message_desc, _) = find_message_descriptor_for_type(
    input_message_name.as_str(), &all_file_descriptors)?;
  
  let output_message_name = method_desc.output_type.clone().unwrap_or_default();
  // uses type name from method_descriptor, which always contains the doc; 3-way logic is safe here
  let (output_message_desc, _) = find_message_descriptor_for_type(
    output_message_name.as_str(), &all_file_descriptors)?;
  let bold = Style::new().bold();

  match build_grpc_request(request_body, metadata, &all_file_descriptors, &input_message_desc) {
    Ok(request) => match make_grpc_request(
      request, config, metadata, &all_file_descriptors, &input_message_desc, &output_message_desc, interaction).await {
      Ok(response) => {
        debug!("Received response from gRPC server - {:?}", response);
        let response_metadata = response.metadata();
        let body = response.get_ref();
        trace!("gRPC metadata: {:?}", response_metadata);
        trace!("gRPC body: {:?}", body);
        let expectations = build_expectations(interaction, "response");
        trace!("consumer expectations: {:?}", expectations);
        let (result, verification_output) = verify_response(body, response_metadata, interaction,
          &all_file_descriptors, &method_desc, &expectations.unwrap_or_default())?;

        let status_result = if !result.is_empty() {
          Red.paint("FAILED")
        } else {
          Green.paint("OK")
        };
        let mut output = vec![
          format!("Given a {}/{} request",
                  bold.paint(service_desc.name.unwrap_or_default()),
                  bold.paint(method_desc.name.unwrap_or_default())),
          format!("    with an input {} message", bold.paint(input_message_name)),
          format!("    will return an output {} message [{}]", bold.paint(output_message_name), status_result)
        ];
        output.extend(verification_output);

        Ok((result, output))
      }
      Err(err) => {
        error!("Received error response from gRPC provider - {:?}", err);
        if let Some(received_status) = err.downcast_ref::<GrpcError>() {
          trace!("gRPC message: {}", received_status.status.message());
          trace!("gRPC metadata: {:?}", received_status.status.metadata());
          let default_contents = MessageContents::default();
          let expected_response = interaction.response.first()
            .unwrap_or(&default_contents);
          if let Some(expected_status) = grpc_status(expected_response) {
            let (result, verification_output) = verify_error_response(expected_response,
                                                                      &received_status.status, &interaction.id);
            let status_result = if !result.is_empty() {
              Red.paint("FAILED")
            } else {
              Green.paint("OK")
            };
            let mut output = vec![
              format!("Given a {}/{} request",
                      bold.paint(service_desc.name.unwrap_or_default()),
                      bold.paint(method_desc.name.unwrap_or_default())),
              format!("    with an input {} message", bold.paint(input_message_name)),
              format!("    will return an error response {} [{}]", bold.paint(expected_status.code().to_string()), status_result)
            ];
            output.extend(verification_output);
            Ok((result, output))
          } else {
            Err(anyhow!(format!("gRPC error: status {}, message '{}'", received_status.status.code(),
              received_status.status.message())))
          }
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

#[instrument]
fn verify_error_response(
  response: &MessageContents,
  actual_status: &Status,
  interaction_id: &Option<String>
) -> (Vec<VerificationMismatchResult>, Vec<String>) {
  let mut output = vec![];
  let mut results = vec![];
  if !response.metadata.is_empty() {
    output.push("      with metadata".to_string());
    let mut metadata = actual_status.metadata().clone();
    if let Ok(code) = i32::from(actual_status.code()).to_string().parse() {
      metadata.insert("grpc-status", code);
    }
    if !actual_status.message().is_empty() {
      if let Ok(message) = actual_status.message().parse() {
        metadata.insert("grpc-message", message);
      }
    }
    match verify_metadata(&metadata, response) {
      Ok((result, md_output)) => {
        if !result.result {
          results.push(VerificationMismatchResult::Mismatches {
            mismatches: result.mismatches,
            interaction_id: interaction_id.clone()
          });
        }
        output.extend(md_output);
      }
      Err(err) => {
        results.push(VerificationMismatchResult::Mismatches {
          mismatches: vec![ Mismatch::MetadataMismatch {
            key: "".to_string(),
            expected: "".to_string(),
            actual: "".to_string(),
            mismatch: format!("Failed to verify the message metadata: {}", err)
          } ],
          interaction_id: interaction_id.clone()
        });
      }
    }
  }
  (results, output)
}

/// Verify response from the gRPC server against expected response in the interaction
fn verify_response(
  response_body: &DynamicMessage,
  response_metadata: &MetadataMap,
  interaction: &SynchronousMessage,
  all_file_descriptors: &FileDescriptorSet,
  method_descriptor: &MethodDescriptorProto,
  expectations: &HashMap<DocPath, String>
) -> anyhow::Result<(Vec<VerificationMismatchResult>, Vec<String>)> {
  let response = interaction.response.first().cloned()
    .unwrap_or_default();
  if interaction.response.len() > 1 {
    warn!("Interaction has more than one response, only comparing the first one");
  }
  let expected_body = response.contents.value();

  let mut results = vec![];
  let mut output = vec![];

  if let Some(mut expected_body) = expected_body {
    let mut actual_body = BytesMut::new();
    response_body.write_to(&mut actual_body)?;

    match match_message(
      method_descriptor.output_type(), 
      all_file_descriptors,
      &mut expected_body,
      &mut actual_body.freeze(),
      &response.matching_rules.rules_for_category("body").unwrap_or_default(),
      true,
      expectations
    ) {
      Ok(result) => {
        debug!("Match service result: {:?}", result);
        match result {
          BodyMatchResult::Ok => {}
          BodyMatchResult::BodyTypeMismatch { message, .. } => {
            results.push(VerificationMismatchResult::Error { error: message, interaction_id: interaction.id.clone() });
          }
          BodyMatchResult::BodyMismatches(mismatches) => {
            for (_, mismatches) in mismatches {
              results.push(VerificationMismatchResult::Mismatches { mismatches, interaction_id: interaction.id.clone() });
            }
          }
        }
      }
      Err(err) => {
        error!("Verifying the response failed with an error - {}", err);
        results.push(VerificationMismatchResult::Error { error: err.to_string(), interaction_id: interaction.id.clone() })
      }
    }
  }

  if !response.metadata.is_empty() {
    output.push("      with metadata".to_string());
    match verify_metadata(response_metadata, &response) {
      Ok((result, md_output)) => {
        if !result.result {
          results.push(VerificationMismatchResult::Mismatches {
            mismatches: result.mismatches,
            interaction_id: interaction.id.clone(),
          });
        }
        output.extend(md_output);
      }
      Err(err) => {
        results.push(VerificationMismatchResult::Mismatches {
          mismatches: vec![ Mismatch::MetadataMismatch {
            key: "".to_string(),
            expected: "".to_string(),
            actual: "".to_string(),
            mismatch: format!("Failed to verify the message metadata: {}", err)
          } ],
          interaction_id: interaction.id.clone()
        });
      }
    }
  }

  Ok((results, output))
}

#[instrument(level = "trace")]
fn verify_metadata(
  metadata: &MetadataMap,
  response: &MessageContents
) -> anyhow::Result<(MetadataMatchResult, Vec<String>)> {
  let rules = response.matching_rules.rules_for_category("metadata").unwrap_or_default();
  let plugin_config = hashmap!{};
  let context = CoreMatchingContext::new(DiffConfig::AllowUnexpectedKeys,
    &rules, &plugin_config);
  compare_metadata(&response.metadata, metadata, &context)
}

async fn make_grpc_request(
  request: Request<DynamicMessage>,
  config: &BTreeMap<String, Value>,
  metadata: &HashMap<String, proto::MetadataValue>,
  file_desc: &FileDescriptorSet,
  input_desc: &DescriptorProto,
  output_desc: &DescriptorProto,
  interaction: &SynchronousMessage
) -> anyhow::Result<Response<DynamicMessage>> {
  let host = config.get("host")
    .map(json_to_string)
    .unwrap_or_else(|| "[::1]".to_string());
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
      error!("gRPC request failed {:?}", err);
      anyhow!(GrpcError { status: err })
    })
}

fn build_grpc_request(
  body: &OptionalBody,
  metadata: &HashMap<String, proto::MetadataValue>,
  file_desc: &FileDescriptorSet,
  input_desc: &DescriptorProto
) -> anyhow::Result<Request<DynamicMessage>> {
  trace!(?body, ?metadata, ?file_desc, ?input_desc, ">> build_grpc_request");
  let mut bytes = body.value().unwrap_or_default();
  let message_fields = decode_message(&mut bytes, input_desc, file_desc)?;
  let mut request = Request::new(DynamicMessage::new(&message_fields, file_desc));
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

#[cfg(test)]
mod tests {
  use expectest::prelude::*;
  use maplit::hashmap;
  use pact_models::path_exp::DocPath;
  use serde_json::json;

  use crate::utils::expectations_from_json;

  #[test]
  fn expectations_from_json_test() {
    let json = json!({});
    expect!(expectations_from_json(json.as_object().unwrap())).to(be_equal_to(hashmap!{}));

    let json = json!({
      "request": {
        "x": "matching(number, 100)",
        "y": "matching(number, 200)"
      },
      "response": {
        "name": "matching(type, 'TestLocation')",
        "location": {
          "x": "matching(number, 100)",
          "y": "matching(number, 200)"
        },
        "description": "matching(type, 'Test Location')"
      }
    });
    expect!(expectations_from_json(json.as_object().unwrap())).to(be_equal_to(hashmap!{
      DocPath::new_unwrap("$.request") => "".to_string(),
      DocPath::new_unwrap("$.request.x") => "\"matching(number, 100)\"".to_string(),
      DocPath::new_unwrap("$.request.y") => "\"matching(number, 200)\"".to_string(),
      DocPath::new_unwrap("$.response") => "".to_string(),
      DocPath::new_unwrap("$.response.location") => "".to_string(),
      DocPath::new_unwrap("$.response.location.x") => "\"matching(number, 100)\"".to_string(),
      DocPath::new_unwrap("$.response.location.y") => "\"matching(number, 200)\"".to_string(),
      DocPath::new_unwrap("$.response.name") => "\"matching(type, 'TestLocation')\"".to_string(),
      DocPath::new_unwrap("$.response.description") => "\"matching(type, 'Test Location')\"".to_string()
    }));
  }
}
