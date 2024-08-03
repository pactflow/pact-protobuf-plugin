//! gRPC mock server implementation

use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Mutex;
use std::task::{Context, Poll};

use anyhow::anyhow;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use bytes::Bytes;
use http::{Method, Request, Response};
use hyper::body::Incoming;
use hyper::server::conn::http2::Builder;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::service::TowerToHyperService;
use lazy_static::lazy_static;
use maplit::hashmap;
use pact_matching::BodyMatchResult;
use pact_models::content_types::ContentType;
use pact_models::generators::generate_hexadecimal;
use pact_models::json_utils::json_to_string;
use pact_models::plugins::PluginData;
use pact_models::prelude::v4::V4Pact;
use pact_models::v4::sync_message::SynchronousMessage;
use prost::Message;
use prost_types::{FileDescriptorProto, FileDescriptorSet, MethodDescriptorProto};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::select;
use tokio::sync::oneshot::{channel, Sender};
use tonic::body::{BoxBody, empty_body};
use tonic::metadata::MetadataMap;
use tower::ServiceBuilder;
use tower_http::ServiceBuilderExt;
use tower_service::Service;
use tracing::{debug, error, Instrument, instrument, trace, trace_span};

use crate::dynamic_message::PactCodec;
use crate::metadata::MetadataMatchResult;
use crate::mock_service::MockService;
use crate::utils::{build_grpc_route, find_message_descriptor_for_type, lookup_service_descriptors_for_interaction, parse_grpc_route, to_fully_qualified_name};

lazy_static! {
  pub static ref MOCK_SERVER_STATE: Mutex<HashMap<String, (Sender<()>, HashMap<String, (usize, Vec<(BodyMatchResult, MetadataMatchResult)>)>)>> = Mutex::new(hashmap!{});
}

/// Main mock server that will use the provided Pact to provide behaviour
#[derive(Debug, Clone)]
pub struct GrpcMockServer {
  pact: V4Pact,
  plugin_config: PluginData,
  descriptors: HashMap<String, FileDescriptorSet>,
  routes: HashMap<String, (FileDescriptorSet, FileDescriptorProto, MethodDescriptorProto, SynchronousMessage)>,
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
      server_key: generate_hexadecimal(8),
      test_context
    }
  }

  /// Start the mock server, consuming this instance and returning the connection details
  /// For each interaction, loads the corresponding service and file descriptors from the Pact file
  /// into a map keyed by a gRPC route in a standard form of `/package.Service/Method`. 
  /// When serving, it allows to easily find the correct descriptors based on the route being called.
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
    .filter_map(|i| match lookup_service_descriptors_for_interaction(&i, &self.pact) {
      Ok((file_set, service, method, file)) => Some((file_set, service, method, file, i.clone())),
      Err(_) => None
    }).filter_map(|(file_set, service, method, file, i)| {
      match to_fully_qualified_name(service.name(), file.package()) {
        Ok(service_full_name) => {
          match build_grpc_route(service_full_name.as_str(), method.name()) {
            Ok(route) => Some((route, (file_set.clone(), file.clone(), method.clone(), i.clone()))),
            Err(_) => None
          }
        },
        Err(_) => None
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

    let (shutdown_snd, mut shutdown_recv) = channel::<()>();
    {
      let mut guard = MOCK_SERVER_STATE.lock().unwrap();
      // Initialise all the routes with an initial state of not received
      let initial_state = self.routes.keys()
        .map(|k| (k.clone(), (0, vec![])))
        .collect();
      guard.insert(self.server_key.clone(), (shutdown_snd, initial_state));
    }

    let listener = TcpListener::bind(addr).await?;
    let address = listener.local_addr()?;

    self.update_mock_server_address(&address);

    let server_key = self.server_key.clone();
    let graceful = hyper_util::server::graceful::GracefulShutdown::new();
    tokio::spawn(async move {
      trace!("Mock server main loop starting");

      trace!("setting up middleware");
      let service = ServiceBuilder::new()
        // High level logging of requests and responses
        .trace_for_grpc()
        // Wrap a `Service` in our middleware stack
        .service(self);
      let http_service = TowerToHyperService::new(service);

      loop {
        let http_service = http_service.clone();

        select! {
          connection = listener.accept() => {
            match connection {
              Ok((stream, remote_address)) => {
                debug!("Received connection from remote {}", remote_address);
                let io = TokioIo::new(stream);
                let conn = Builder::new(TokioExecutor::new())
                  .serve_connection(io, http_service);

                let conn = graceful.watch(conn);
                tokio::spawn(async move {
                  if let Err(err) = conn.await {
                      error!("Failed to serve connection: {err}");
                  }
                  trace!("Connection dropped: {}", remote_address);
                });
              },
              Err(e) => {
                error!("Failed to accept connection: {e}");
              }
            }
          }

          _ = &mut shutdown_recv => {
            trace!("Received shutdown signal, signalling server shutdown");
            graceful.shutdown().await;
            trace!("Exiting main loop");
            break;
          }
        }
      }

      trace!("Mock server main loop done");
    }.instrument(trace_span!("mock server", key = %server_key, port = address.port())));

    Ok(address)
  }

  fn update_mock_server_address(&mut self, address: &SocketAddr) {
    self.test_context.insert("mockServer".to_string(), json!({
      "href": format!("http://{}:{}", address.ip(), address.port()),
      "port": address.port()
    }));
  }
}

impl Service<Request<Incoming>> for GrpcMockServer  {
  type Response = Response<BoxBody>;
  type Error = hyper::Error;
  type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

  fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
    Poll::Ready(Ok(()))
  }

  /// Process gRPC call.
  /// Looks up descriptors and interaction config in `self.routes` based on the request path, 
  /// then uses them to respond to construct a `mock_service.MockService` instance and call it.
  /// The actual work is done in `mock_service.MockService::handle_message()`.
  #[instrument(skip(self), level = "trace")]
  fn call(&mut self, req: Request<Incoming>) -> Self::Future {
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
            if let Some((service_full_name, method)) = parse_grpc_route(request_path) {
              if let Some((file, _file_descriptor, method_descriptor, message)) = routes.get(request_path) {
                trace!(message = message.description.as_str(), "Found route for service call");
                
                let service_and_method = format!("{service_full_name}/{method}");  // just for logging
                let input_name = method_descriptor.input_type.as_ref().expect(format!(
                  "Input message name is empty for service {}", service_and_method.as_str()).as_str());
                let output_name = method_descriptor.output_type.as_ref().expect(format!(
                  "Output message name is empty for service {}", service_and_method.as_str()).as_str());

                if let Ok((input_message, _)) = find_message_descriptor_for_type(input_name, &file) {
                  if let Ok((output_message, _)) = find_message_descriptor_for_type(output_name, &file) {
                    let codec = PactCodec::new(file, &input_message, &output_message, message);
                    let mock_service = MockService::new(file, service_full_name.as_str(),
                      method_descriptor, &input_message, &output_message, message, server_key.as_str(),
                      pact
                    );
                    let mut grpc = tonic::server::Grpc::new(codec);
                    let response = grpc.unary(mock_service, req).await;
                    trace!(?response, ">> sending response");
                    Ok(response)
                  } else {
                    error!("Did not find the descriptor for the output message {}", output_name);
                    Ok(failed_precondition())
                  }
                } else {
                  error!("Did not find the descriptor for the input message {}", input_name);
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
