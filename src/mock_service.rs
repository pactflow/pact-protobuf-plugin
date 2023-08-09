//! Module provides the service implementation based on a Pact interaction

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use maplit::hashmap;
use pact_matching::{CoreMatchingContext, DiffConfig};
use pact_models::generators::{GenerateValue, GeneratorCategory, NoopVariantMatcher, VariantMatcher};
use pact_models::json_utils::json_to_string;
use pact_models::pact::Pact;
use pact_models::path_exp::DocPath;
use pact_models::prelude::v4::V4Pact;
use pact_models::v4::message_parts::MessageContents;
use pact_models::v4::sync_message::SynchronousMessage;
use pact_plugin_driver::plugin_models::PluginInteractionConfig;
use prost_types::{DescriptorProto, FileDescriptorSet, MethodDescriptorProto};
use tonic::{Request, Response, Status};
use tonic::metadata::{Entry, MetadataMap};
use tower_service::Service;
use tracing::{debug, error, info, instrument, trace, warn};

use crate::dynamic_message::DynamicMessage;
use crate::matching::compare;
use crate::message_decoder::decode_message;
use crate::metadata::{compare_metadata, grpc_status};
use crate::mock_server::MOCK_SERVER_STATE;

#[derive(Debug, Clone)]
pub(crate) struct MockService {
  file_descriptor_set: FileDescriptorSet,
  service_name: String,
  message: SynchronousMessage,
  method_descriptor: MethodDescriptorProto,
  input_message: DescriptorProto,
  output_message: DescriptorProto,
  server_key: String,
  pact: V4Pact
}

impl MockService {
  #[instrument(skip(self, message_descriptor, response_descriptor))]
  pub(crate) async fn handle_message(
    &self,
    request: DynamicMessage,
    message_descriptor: DescriptorProto,
    response_descriptor: DescriptorProto,
    request_metadata: MetadataMap
  ) -> Result<Response<DynamicMessage>, Status> {
    // 1. Compare the incoming message to the request message from the interaction
    let mut expected_message_bytes = self.message.request.contents.value().unwrap_or_default();
    let expected_message = decode_message(&mut expected_message_bytes, &message_descriptor, &self.file_descriptor_set)
      .map_err(|err| Status::invalid_argument(err.to_string()))?;
    let plugin_config = self.pact.plugin_data().iter()
      .map(|pd| {
        (pd.name.clone(), PluginInteractionConfig {
          pact_configuration: pd.configuration.clone(),
          interaction_configuration: self.message.plugin_config.get(pd.name.as_str()).cloned().unwrap_or_default()
        })
      }).collect();

    let context = CoreMatchingContext::new(DiffConfig::NoUnexpectedKeys,
      &self.message.request.matching_rules.rules_for_category("body").unwrap_or_default(),
      &plugin_config);
    let mismatches = compare(&message_descriptor, &expected_message, request.proto_fields(), &context,
                             &expected_message_bytes, &self.file_descriptor_set);

    // 2. Compare any metadata from the incoming message
    let md_context = CoreMatchingContext::new(DiffConfig::NoUnexpectedKeys,
      &self.message.request.matching_rules.rules_for_category("metadata").unwrap_or_default(),
      &plugin_config);
    let md_mismatches = compare_metadata(&self.message.request.metadata, &request_metadata,
      &md_context);

    trace!("Comparison result = {:?}", mismatches);
    match (mismatches, md_mismatches) {
      (Ok(result), Ok((md_result, _))) => {
        {
          // record the result in the static store
          let mut guard = MOCK_SERVER_STATE.lock().unwrap();
          let key = format!("{}/{}", self.service_name, self.method_descriptor.name.clone().unwrap_or_else(|| "unknown method".into()));
          if let Some((_, results)) = guard.get_mut(self.server_key.as_str()) {
            let route_results = results.entry(key).or_insert((0, vec![]));
            trace!(store_length = route_results.1.len(), "Adding result to mock server '{}' static store", self.server_key);
            route_results.0 += 1;
            route_results.1.push((result.clone(), md_result.clone()));
          } else {
            error!("INTERNAL ERROR: Did not find an entry for '{}' in mock server static store", self.server_key);
          }
        }

        if result.all_matched() && md_result.all_matched() {
          debug!("Request matched OK");
          let response_contents = self.message.response.first().cloned().unwrap_or_default();
          // check for a gRPC status on the response metadata
          if let Some(status) = grpc_status(&response_contents) {
            info!("a gRPC status {} is set for the response, returning that", status);
            Err(status)
          } else {
            debug!("Returning response");
            let mut response_bytes = response_contents.contents.value()
              .unwrap_or_default();
            trace!("Response message has {} bytes", response_bytes.len());
            let response_message = decode_message(&mut response_bytes, &response_descriptor, &self.file_descriptor_set)
              .map_err(|err| {
                error!("Failed to encode response message - {}", err);
                Status::invalid_argument(err.to_string())
              })?;
            let mut message = DynamicMessage::new(&response_message, &self.file_descriptor_set);
            self.apply_generators(&mut message, &response_contents).map_err(|err| {
              error!("Failed to generate response message - {}", err);
              Status::invalid_argument(err.to_string())
            })?;
            trace!("Sending message {message:?}");
            let mut response = Response::new(message);
            if !response_contents.metadata.is_empty() {
              Self::set_response_metadata(response_contents, &mut response);
            }
            Ok(response)
          }
        } else {
          error!("Failed to match the request message - {result:?}");
          Err(Status::failed_precondition(format!("Failed to match the request message - {result:?}")))
        }
      }
      (Err(err), _) => {
        error!("Failed to match the request message - {err}");
        Err(Status::failed_precondition(err.to_string()))
      }
      (_, Err(err)) => {
        error!("Failed to match the request message metadata - {err}");
        Err(Status::failed_precondition(err.to_string()))
      }
    }
  }

  fn set_response_metadata(response_contents: MessageContents, response: &mut Response<DynamicMessage>) {
    let md = response.metadata_mut();
    for (key, value) in &response_contents.metadata {
      let key = key.to_lowercase();
      // exclude the content type, because that is a special value added by the Pact framework
      // also exclude the gRPC status, because that is handled separately
      if key != "content-type" && key != "contenttype" && key != "grpc-status" {
        match json_to_string(value).parse() {
          Ok(parsed_val) => {
            match md.entry(key.as_str()) {
              Ok(entry) => match entry {
                Entry::Occupied(mut o) => {
                  warn!("Replacing existing gRPC metadata key '{}'", key);
                  o.insert(parsed_val);
                },
                Entry::Vacant(v) => {
                  v.insert(parsed_val);
                }
              }
              Err(err) => {
                error!("'{}' is not a valid gRPC metadata key, ignoring it - {}", key, err);
              }
            }
          }
          Err(err) => {
            error!("'{}' is not a valid gRPC metadata value, ignoring it - {}", value, err);
          }
        }
      }
    }
  }
}

impl MockService {
  pub(crate) fn new(
    file_descriptor_set: &FileDescriptorSet,
    service_name: &str,
    method_descriptor: &MethodDescriptorProto,
    input_message: &DescriptorProto,
    output_message: &DescriptorProto,
    message: &SynchronousMessage,
    server_key: &str,
    pact: V4Pact
  ) -> Self {
    MockService {
      file_descriptor_set: file_descriptor_set.clone(),
      service_name: service_name.to_string(),
      method_descriptor: method_descriptor.clone(),
      input_message: input_message.clone(),
      output_message: output_message.clone(),
      message: message.clone(),
      server_key: server_key.to_string(),
      pact
    }
  }

  fn apply_generators(&self, message: &mut DynamicMessage, contents: &MessageContents) -> anyhow::Result<()> {
    let variant_matcher = NoopVariantMatcher {};
    let vm_boxed = variant_matcher.boxed();
    let context = hashmap!{}; // TODO: This needs to be passed in via the start mock server call

    if let Some(generators) = contents.generators.categories.get(&GeneratorCategory::BODY) {
      for (key, generator) in generators.iter() {
        let path = DocPath::new(key)?;
        let value = message.fetch_value(&path);
        if let Some(value) = value {
          let generated_value = generator.generate_value(&value.data, &context, &vm_boxed)?;
          message.set_value(&path, generated_value)?;
        }
      }
    }

    Ok(())
  }
}

impl Service<Request<DynamicMessage>> for MockService {
  type Response = Response<DynamicMessage>;
  type Error = Status;
  type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

  fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
    Poll::Ready(Ok(()))
  }

  fn call(&mut self, req: Request<DynamicMessage>) -> Self::Future {
    let (request_metadata, _, request) = req.into_parts();
    let message_descriptor = self.input_message.clone();
    let response_descriptor = self.output_message.clone();
    let service = self.clone();
    Box::pin(async move {
      service.handle_message(request, message_descriptor, response_descriptor, request_metadata).await
    })
  }
}

#[cfg(test)]
mod tests {
  use base64::Engine;
  use base64::engine::general_purpose::STANDARD as BASE64;
  use bytes::{Bytes, BytesMut};
  use expectest::prelude::*;
  use pact_models::v4::pact::V4Pact;
  use prost::Message;
  use prost_types::FileDescriptorSet;
  use serde_json::json;
  use tonic::metadata::MetadataMap;

  use crate::dynamic_message::DynamicMessage;
  use crate::message_decoder::decode_message;
  use crate::mock_service::MockService;
  use crate::protobuf::tests::DESCRIPTOR_BYTES;

  #[test_log::test(tokio::test)]
  async fn handle_message_applies_any_generators() {
    let bytes = BASE64.decode(DESCRIPTOR_BYTES).unwrap();
    let bytes1 = Bytes::copy_from_slice(bytes.as_slice());
    let file_descriptor_set = FileDescriptorSet::decode(bytes1).unwrap();
    let fds = &file_descriptor_set;
    let ac_desc = fds.file.iter()
      .find(|ds| ds.name.clone().unwrap_or_default() == "area_calculator.proto")
      .unwrap();
    let service_desc = ac_desc.service.iter()
      .find(|sd| sd.name.clone().unwrap_or_default() == "Calculator")
      .unwrap();
    let method = service_desc.method.iter()
      .find(|md| md.name.clone().unwrap_or_default() == "calculateOne")
      .unwrap();
    let input_message = ac_desc.message_type.iter()
      .find(|md| md.name.clone().unwrap_or_default() == "ShapeMessage")
      .unwrap();
    let output_message = ac_desc.message_type.iter()
      .find(|md| md.name.clone().unwrap_or_default() == "AreaResponse")
      .unwrap();

    let pact_json = json!({
      "interactions": [
        {
          "description": "calculate rectangle area request",
          "key": "c7fbe3ee",
          "pluginConfiguration": {
            "protobuf": {
              "descriptorKey": "d4147b5793ad1996e476382bd79499a5",
              "service": "Calculator/calculateOne"
            }
          },
          "request": {
            "contents": {
              "content": "EgoNAABAQBUAAIBA",
              "contentType": "application/protobuf; message=ShapeMessage",
              "contentTypeHint": "BINARY",
              "encoded": "base64"
            },
            "matchingRules": {
              "body": {
                "$.rectangle.length": {
                  "combine": "AND",
                  "matchers": [
                    {
                      "match": "number"
                    }
                  ]
                },
                "$.rectangle.width": {
                  "combine": "AND",
                  "matchers": [
                    {
                      "match": "number"
                    }
                  ]
                }
              }
            }
          },
          "response": [
            {
              "contents": {
                "content": "CgQAAEBBEgoyMDAwLTAxLTAx",
                "contentType": "application/protobuf; message=AreaResponse",
                "contentTypeHint": "BINARY",
                "encoded": "base64"
              },
              "generators": {
                "body": {
                  "$.value": {
                    "digits": "10",
                    "type": "RandomDecimal"
                  }
                }
              },
              "matchingRules": {
                "body": {
                  "$.value.*": {
                    "combine": "AND",
                    "matchers": [
                      {
                        "match": "number"
                      }
                    ]
                  }
                }
              }
            }
          ],
          "transport": "grpc",
          "type": "Synchronous/Messages"
        }
      ],
      "metadata": {
        "pactSpecification": {
          "version": "4.0"
        }
      }
    });
    let pact = V4Pact::pact_from_json(&pact_json, "<>").unwrap();
    let message = pact.interactions.first().unwrap();

    let bytes = BASE64.decode("EgoNAABAQBUAAIBA").unwrap();
    let mut bytes2 = BytesMut::from(bytes.as_slice());
    let fields = decode_message(&mut bytes2, input_message, fds).unwrap();
    let request = DynamicMessage::new(fields.as_slice(), &file_descriptor_set);

    let mock_service = MockService {
      file_descriptor_set: file_descriptor_set.clone(),
      service_name: "Calculator".to_string(),
      message: message.as_v4_sync_message().unwrap(),
      method_descriptor: method.clone(),
      input_message: input_message.clone(),
      output_message: output_message.clone(),
      server_key: "1234".to_string(),
      pact
    };
    let response = mock_service.handle_message(request,
      input_message.clone(), output_message.clone(),
      MetadataMap::default()
    ).await.unwrap();
    let response_message = response.into_inner();
    let response_fields = response_message.proto_fields();
    let area = &response_fields[0];
    expect!(area.data.to_string()).to_not(be_equal_to("12"));
  }
}
