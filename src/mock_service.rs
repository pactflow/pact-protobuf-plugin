//! Module provides the service implementation based on a Pact interaction

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use maplit::hashmap;
use pact_matching::{CoreMatchingContext, DiffConfig};

use pact_models::v4::sync_message::SynchronousMessage;
use prost_types::{DescriptorProto, FileDescriptorSet, MethodDescriptorProto};
use tonic::{Request, Response, Status};
use tower_service::Service;
use tracing::{debug, error, instrument, trace};

use crate::dynamic_message::DynamicMessage;
use crate::matching::compare;
use crate::message_decoder::decode_message;
use crate::mock_server::MOCK_SERVER_STATE;

#[derive(Debug, Clone)]
pub(crate) struct MockService {
  file_descriptor_set: FileDescriptorSet,
  service_name: String,
  message: SynchronousMessage,
  method_descriptor: MethodDescriptorProto,
  input_message: DescriptorProto,
  output_message: DescriptorProto,
  server_key: String
}

impl MockService {
  #[instrument(skip(self, message_descriptor, response_descriptor))]
  pub(crate) async fn handle_message(
    &self,
    request: DynamicMessage,
    message_descriptor: DescriptorProto,
    response_descriptor: DescriptorProto
  ) -> Result<Response<DynamicMessage>, Status> {
    // 1. Compare the incoming message to the request message from the interaction
    let mut expected_message_bytes = self.message.request.contents.value().unwrap_or_default();
    let expected_message = decode_message(&mut expected_message_bytes, &message_descriptor, &self.file_descriptor_set)
      .map_err(|err| Status::invalid_argument(err.to_string()))?;
    let context = CoreMatchingContext::new(DiffConfig::NoUnexpectedKeys,
                                           &self.message.request.matching_rules.rules_for_category("body").unwrap_or_default(),
                                           &hashmap!{});
    let mismatches = compare(&message_descriptor, &expected_message, &request.proto_fields(), &context,
                             &expected_message_bytes, &self.file_descriptor_set);
    trace!("Comparison result = {:?}", mismatches);
    match mismatches {
      Ok(result) => {
        {
          // record the result in the static store
          let mut guard = MOCK_SERVER_STATE.lock().unwrap();
          let key = format!("{}/{}", self.service_name, self.method_descriptor.name.clone().unwrap_or_else(|| "unknown method".into()));
          if let Some((_, results)) = guard.get_mut(self.server_key.as_str()) {
            let mut route_results = results.entry(key).or_insert((0, vec![]));
            trace!(store_length = route_results.1.len(), "Adding result to mock server '{}' static store", self.server_key);
            route_results.0 += 1;
            route_results.1.push(result.clone());
          } else {
            error!("INTERNAL ERROR: Did not find an entry for '{}' in mock server static store", self.server_key);
          }
        }

        if result.all_matched() {
          debug!("Request matched OK, returning expected response");
          // TODO: need to invoke any generators
          let mut response_bytes = self.message.response.first()
            .and_then(|d| d.contents.value())
            .unwrap_or_default();
          trace!("Response message has {} bytes", response_bytes.len());
          let response_message = decode_message(&mut response_bytes, &response_descriptor, &self.file_descriptor_set)
            .map_err(|err| {
              error!("Failed to encode response message - {}", err);
              Status::invalid_argument(err.to_string())
            })?;
          let message = DynamicMessage::new(&response_message);
          trace!("Sending message {message:?}");
          Ok(Response::new(message))
        } else {
          error!("Failed to match the request message - {result:?}");
          Err(Status::failed_precondition(format!("Failed to match the request message - {result:?}")))
        }
      }
      Err(err) => {
        error!("Failed to match the request message - {err}");
        Err(Status::failed_precondition(err.to_string()))
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
    server_key: &str
  ) -> Self {
    MockService {
      file_descriptor_set: file_descriptor_set.clone(),
      service_name: service_name.to_string(),
      method_descriptor: method_descriptor.clone(),
      input_message: input_message.clone(),
      output_message: output_message.clone(),
      message: message.clone(),
      server_key: server_key.to_string()
    }
  }
}

impl Service<tonic::Request<DynamicMessage>> for MockService {
  type Response = tonic::Response<DynamicMessage>;
  type Error = Status;
  type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

  fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
    Poll::Ready(Ok(()))
  }

  fn call(&mut self, req: Request<DynamicMessage>) -> Self::Future {
    let request = req.into_inner();
    let message_descriptor = self.input_message.clone();
    let response_descriptor = self.output_message.clone();
    let service = self.clone();
    Box::pin(async move {
      service.handle_message(request, message_descriptor, response_descriptor).await
    })
  }
}
