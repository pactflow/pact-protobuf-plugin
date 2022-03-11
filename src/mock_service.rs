//! Module provides the service implementation based on a Pact interaction

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use maplit::hashmap;
use pact_matching::{BodyMatchResult, CoreMatchingContext, DiffConfig};

use pact_models::v4::sync_message::SynchronousMessage;
use prost_types::{DescriptorProto, FileDescriptorSet, MethodDescriptorProto, ServiceDescriptorProto};
use tonic::{Request, Response, Status};
use tower_service::Service;
use tracing::{error, instrument, trace};

use crate::dynamic_message::DynamicMessage;
use crate::matching::compare;
use crate::message_decoder::decode_message;
use crate::mock_server::MOCK_SERVER_STATE;

#[derive(Debug, Clone)]
pub(crate) struct MockService {
  file_descriptor_set: FileDescriptorSet,
  message: SynchronousMessage,
  method_descriptor: MethodDescriptorProto,
  input_message: DescriptorProto,
  output_message: DescriptorProto,
  server_key: String,
}

impl MockService {
  #[instrument]
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
    match mismatches {
      Ok(result) => {
        // record the result in the static store
        let mut guard = MOCK_SERVER_STATE.lock().unwrap();
        if let Some((_, results)) = guard.get_mut(self.server_key.as_str()) {
          results.push((self.method_descriptor.name.clone().unwrap_or("unknown method".into()), result.clone()));
        }

        if result.all_matched() {
          // TODO: need to invoke any generators
          let mut response_bytes = self.message.response.first()
            .map(|d| d.contents.value())
            .flatten()
            .unwrap_or_default();
          trace!("Response message has {} bytes", response_bytes.len());
          let response_message = decode_message(&mut response_bytes, &response_descriptor, &self.file_descriptor_set)
            .map_err(|err| Status::invalid_argument(err.to_string()))?;
          let message = DynamicMessage::new(&response_descriptor, &response_message);
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
    method_descriptor: &MethodDescriptorProto,
    input_message: &DescriptorProto,
    output_message: &DescriptorProto,
    message: &SynchronousMessage,
    server_key: &str
  ) -> Self {
    MockService {
      file_descriptor_set: file_descriptor_set.clone(),
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
