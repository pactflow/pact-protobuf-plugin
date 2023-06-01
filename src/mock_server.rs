//! gRPC mock server implementation

use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Mutex;
use std::task::{Context, Poll};
use std::thread;

use anyhow::anyhow;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use bytes::Bytes;
use http::Method;
use hyper::{http, Request, Response};
use hyper::server::accept;
use lazy_static::lazy_static;
use maplit::hashmap;
use pact_matching::BodyMatchResult;
use pact_models::content_types::ContentType;
use pact_models::json_utils::json_to_string;
use pact_models::plugins::PluginData;
use pact_models::prelude::v4::V4Pact;
use pact_models::v4::sync_message::SynchronousMessage;
use prost::Message;
use prost_types::{FileDescriptorSet, MethodDescriptorProto};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::runtime::Handle;
use tokio::sync::oneshot::{channel, Sender};
use tonic::body::{BoxBody, empty_body};
use tonic::metadata::MetadataMap;
use tower::make::Shared;
use tower::ServiceBuilder;
use tower_http::ServiceBuilderExt;
use tower_service::Service;
use tracing::{debug, error, Instrument, instrument, trace, trace_span};
use uuid::Uuid;

use crate::dynamic_message::PactCodec;
use crate::metadata::MetadataMatchResult;
use crate::mock_service::MockService;
use crate::tcp::TcpIncoming;
use crate::utils::{find_message_type_by_name, last_name};

lazy_static! {
  pub static ref MOCK_SERVER_STATE: Mutex<HashMap<String, (Sender<()>, HashMap<String, (usize, Vec<(BodyMatchResult, MetadataMatchResult)>)>)>> = Mutex::new(hashmap!{});
}

/// Main mock server that will use the provided Pact to provide behaviour
#[derive(Debug, Clone)]
pub struct GrpcMockServer {
  pact: V4Pact,
  plugin_config: PluginData,
  descriptors: HashMap<String, FileDescriptorSet>,
  routes: HashMap<String, (FileDescriptorSet, MethodDescriptorProto, SynchronousMessage)>,
  /// Server key for this mock server
  pub server_key: String,
  /// test context pass in from the test framework
  pub test_context: HashMap<String, Value>,
}

impl GrpcMockServer
{
  /// Create a new mock server
  pub fn new(pact: V4Pact, plugin_config: &PluginData, test_context: HashMap<String, Value>) -> Self {
    GrpcMockServer {
      pact,
      plugin_config: plugin_config.clone(),
      descriptors: Default::default(),
      routes: Default::default(),
      server_key: Uuid::new_v4().to_string(),
      test_context
    }
  }

  /// Start the mock server, consuming this instance and returning the connection details
  #[instrument(skip(self))]
  pub async fn start_server(mut self, host_interface: &str, port: u32, tls: bool) -> anyhow::Result<SocketAddr> {
    // Get all the descriptors from the Pact file and parse them
    for (key, value) in &self.plugin_config.configuration {
      if let Value::Object(map) = value {
        if let Some(descriptor) = map.get("protoDescriptors") {
          let bytes = BASE64.decode(json_to_string(descriptor))?;
          let buffer = Bytes::from(bytes);
          let fds = FileDescriptorSet::decode(buffer)?;
          self.descriptors.insert(key.clone(), fds);
        }
      }
    }

    if self.descriptors.is_empty() {
      return Err(anyhow!("Pact file does not contain any Protobuf descriptors"));
    }

    // Build a map of routes using the interactions in the Pact file
    self.routes = self.pact.interactions.iter()
      .filter_map(|i| i.as_v4_sync_message())
      .filter_map(|i| i.plugin_config.get("protobuf").map(|p| (p.clone(), i.clone())))
      .filter_map(|(c, i)| {
        if let Some(key) = c.get("descriptorKey") {
          if let Some(descriptors) = self.descriptors.get(json_to_string(key).as_str()) {
            if let Some(service) = c.get("service") {
              if let Some((service_name, method_name)) = json_to_string(service).split_once('/') {
                descriptors.file.iter().filter_map(|d| {
                  d.service.iter().find(|s| s.name.clone().unwrap_or_default() == service_name)
                }).next()
                  .and_then(|d| {
                    d.method.iter()
                      .find(|m| m.name.clone().unwrap_or_default() == method_name)
                      .map(|m| (format!("{service_name}/{method_name}"), (descriptors.clone(), m.clone(), i.clone())))
                  })
              } else {
                // protobuf service was not properly formed <SERViCE>/<METHOD>
                None
              }
            } else {
              // protobuf plugin configuration section did not have a service defined
              None
            }
          } else {
            // protobuf plugin configuration section did not have a matching key to the descriptors
            None
          }
        } else {
          // Interaction did not have a protobuf plugin configuration section
          None
        }
      }).collect();

    // Bind to a OS provided port and create a TCP listener
    let interface = if host_interface.is_empty() {
      "[::1]"
    } else {
      host_interface
    };
    let addr: SocketAddr = format!("{interface}:{port}").parse()?;
    trace!("setting up mock server {addr}");

    let (snd, rcr) = channel::<()>();
    {
      let mut guard = MOCK_SERVER_STATE.lock().unwrap();
      // Initialise all the routes with an initial state of not received
      let initial_state = self.routes.keys()
        .map(|k| (k.clone(), (0, vec![])))
        .collect();
      guard.insert(self.server_key.clone(), (snd, initial_state));
    }

    let listener = TcpListener::bind(addr).await?;
    let address = listener.local_addr()?;

    self.update_mock_server_address(&address);

    let handle = Handle::current();
    // because Rust
    let key = self.server_key.clone();
    let key2 = self.server_key.clone();
    let result = thread::spawn(move || {
      let incoming_stream = TcpIncoming { inner: listener };
      let incoming = accept::from_stream(incoming_stream);

      trace!("setting up middleware");
      let service = ServiceBuilder::new()
        // High level logging of requests and responses
        .trace_for_grpc()
        // Wrap a `Service` in our middleware stack
        .service(self);

      trace!("setting up HTTP server");
      let server = hyper::Server::builder(incoming)
        .http2_only(true)
        //   //   // .http2_initial_connection_window_size(init_connection_window_size)
        //   //   // .http2_initial_stream_window_size(init_stream_window_size)
        //   //   // .http2_max_concurrent_streams(max_concurrent_streams)
        //   //   // .http2_keep_alive_interval(http2_keepalive_interval)
        //   //   // .http2_keep_alive_timeout(http2_keepalive_timeout)
        //   //   // .http2_max_frame_size(max_frame_size)
        .serve(Shared::new(service))
        .with_graceful_shutdown(async move {
          let _ = rcr.await;
          trace!("Received shutdown signal for server {}", key);
        })
        .instrument(tracing::trace_span!("mock server", key = key2.as_str(), port = address.port()));

      trace!("spawning server onto runtime");
      handle.spawn(server);
      trace!("spawning server onto runtime - done");
    }).join();

    if result.is_err() {
      Err(anyhow!("Failed to start mock server thread"))
    } else {
      trace!("Mock server setup OK");
      Ok(address)
    }
  }

  fn update_mock_server_address(&mut self, address: &SocketAddr) {
    self.test_context.insert("mockServer".to_string(), json!({
      "href": format!("http://{}:{}", address.ip(), address.port()),
      "port": address.port()
    }));
  }
}

impl Service<Request<hyper::Body>> for GrpcMockServer  {
  type Response = Response<BoxBody>;
  type Error = hyper::Error;
  type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

  fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
    Poll::Ready(Ok(()))
  }

  #[instrument(skip(self), level = "trace")]
  fn call(&mut self, req: Request<hyper::Body>) -> Self::Future {
    let routes = self.routes.clone();
    let server_key = self.server_key.clone();
    let pact = self.pact.clone();

    Box::pin(async move {
      trace!("Got request {req:?}");

      let headers = req.headers();
      let metadata = MetadataMap::from_headers(headers.clone());

      // If Content-Type does not begin with "application/grpc", gRPC servers SHOULD respond with HTTP status of 415 (Unsupported Media Type).
      // This will prevent other HTTP/2 clients from interpreting a gRPC error response, which uses status 200 (OK), as successful.
      let content_type = if let Some(content_type) = metadata.get("content-type") {
        ContentType::parse(content_type.to_str().unwrap_or_default())
          .map_err(|err| anyhow!(err))
      } else {
        Err(anyhow!("no content type was provided"))
      };

      match content_type {
        Ok(content_type) => if content_type.base_type().to_string().starts_with("application/grpc") {
          let method = req.method();
          if method == Method::POST {
            let request_path = req.uri().path();
            debug!(?request_path, "gRPC request received");
            if let Some((service, method)) = request_path[1..].split_once('/') {
              let service_name = last_name(service);
              let lookup = format!("{service_name}/{method}");
              if let Some((file, method_descriptor, message)) = routes.get(lookup.as_str()) {
                trace!(message = message.description.as_str(), "Found route for service call");

                let input_message_name = method_descriptor.input_type.clone().unwrap_or_default();
                let input_message = find_message_type_by_name(last_name(input_message_name.as_str()), file);
                let output_message_name = method_descriptor.output_type.clone().unwrap_or_default();
                let output_message = find_message_type_by_name(last_name(output_message_name.as_str()), file);

                if let Ok((input_message, _)) = input_message {
                  if let Ok((output_message, _)) = output_message {
                    let codec = PactCodec::new(file, &input_message, &output_message, message);
                    let mock_service = MockService::new(file, service_name,
                      method_descriptor, &input_message, &output_message, message, server_key.as_str(),
                      pact
                    );
                    let mut grpc = tonic::server::Grpc::new(codec);
                    let response = grpc.unary(mock_service, req).await;
                    trace!(?response, ">> sending response");
                    Ok(response)
                  } else {
                    error!("Did not find the descriptor for the output message {}", output_message_name);
                    Ok(failed_precondition())
                  }
                } else {
                  error!("Did not find the descriptor for the input message {}", input_message_name);
                  Ok(failed_precondition())
                }
              } else {
                Ok(invalid_path())
              }
            } else {
              Ok(invalid_path())
            }
          } else {
            Ok(invalid_method())
          }
        } else {
          Ok(invalid_media())
        }
        Err(err) => {
          error!("Failed to parse the content type - {err}");
          Ok(invalid_media())
        }
      }
    }.instrument(trace_span!("mock_server_handler", key = self.server_key.as_str())))
  }
}

fn invalid_media() -> Response<BoxBody> {
  http::Response::builder()
    .status(415)
    .body(empty_body())
    .unwrap()
}

fn invalid_method() -> Response<BoxBody> {
  http::Response::builder()
    .status(405)
    .body(empty_body())
    .unwrap()
}

fn invalid_path() -> Response<BoxBody> {
  http::Response::builder()
    .status(200)
    .header("grpc-status", "12")
    .header("content-type", "application/grpc")
    .body(empty_body())
    .unwrap()
}

fn failed_precondition() -> Response<BoxBody> {
  http::Response::builder()
    .status(200)
    .header("grpc-status", "9")
    .header("content-type", "application/grpc")
    .body(empty_body())
    .unwrap()
}
