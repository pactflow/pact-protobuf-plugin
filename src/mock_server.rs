//! gRPC mock server implementation

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use anyhow::anyhow;
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
use pact_models::prelude::v4::V4Pact;
use pact_models::v4::sync_message::SynchronousMessage;
use prost_types::MethodDescriptorProto;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::select;
use tokio::sync::oneshot::{channel, Sender};
use tonic::body::Body;
use tonic::metadata::MetadataMap;
use tower::ServiceBuilder;
use tower_http::ServiceBuilderExt;
use tower_service::Service;
use tracing::{debug, error, Instrument, instrument, trace, trace_span};

use crate::dynamic_message::PactCodec;
use crate::metadata::MetadataMatchResult;
use crate::mock_service::MockService;
use crate::utils::{
  build_grpc_route,
  get_descriptors_for_interaction,
  lookup_interaction_config,
  lookup_plugin_config,
  lookup_service_and_method_for_interaction,
  parse_grpc_route,
  to_fully_qualified_name,
  DescriptorCache
};
use pact_models::json_utils::json_to_string;

lazy_static! {
  pub static ref MOCK_SERVER_STATE: Mutex<HashMap<String, (Sender<()>, HashMap<String, (usize, Vec<(BodyMatchResult, MetadataMatchResult)>)>)>> = Mutex::new(hashmap!{});
}

/// Mock server route that maps a set of Protobuf descriptors to one or more messages
#[derive(Debug, Clone)]
pub struct MockServerRoute {
  /// All descriptors (shared via Arc to avoid cloning the actual data)
  pub fds: Arc<DescriptorCache>,
  /// Method descriptor for this route
  pub method_descriptor: MethodDescriptorProto,
  /// Messages for this route
  pub messages: Vec<SynchronousMessage>
}

impl MockServerRoute {
  /// Convenience function to create a new route
  pub fn new(
    descriptor_cache: Arc<DescriptorCache>,
    method: MethodDescriptorProto,
    i: SynchronousMessage
  ) -> Self {
    MockServerRoute {
      fds: descriptor_cache,
      method_descriptor: method,
      messages: vec![i]
    }
  }
}

/// Main mock server that will use the provided Pact to provide behaviour
#[derive(Debug, Clone)]
pub struct GrpcMockServer {
  pact: V4Pact,
  routes: HashMap<String, MockServerRoute>,
  /// Server key for this mock server
  pub server_key: String,
  /// test context pass in from the test framework
  pub test_context: HashMap<String, Value>,
}

impl GrpcMockServer
{
  /// Create a new mock server
  pub fn new(pact: V4Pact, test_context: HashMap<String, Value>) -> Self {
    GrpcMockServer {
      pact,
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
    // Step 1: Collect all unique descriptor keys from interactions (lightweight - just strings)
    let descriptor_keys: HashSet<String> = self.pact.interactions.iter()
      .filter_map(|i| i.as_v4_sync_message())
      .filter_map(|i| lookup_interaction_config(&i)
        .and_then(|c| c.get("descriptorKey").map(json_to_string)))
      .collect();
    
    trace!("Collected {} unique descriptor keys: {:?}", descriptor_keys.len(), descriptor_keys);
    
    // Step 2: Parse each unique descriptor key once (only parses unique keys!)
    let plugin_config = lookup_plugin_config(&self.pact)?;
    let descriptor_caches: HashMap<String, Arc<DescriptorCache>> = descriptor_keys.iter()
      .filter_map(|key| {
        match get_descriptors_for_interaction(key, &plugin_config) {
          Ok(fds) => {
            trace!("Successfully loaded descriptors for key: {}", key);
            Some((key.clone(), Arc::new(DescriptorCache::new(fds))))
          }
          Err(e) => {
            error!("Failed to load descriptors for key {}: {}", key, e);
            None
          }
        }
      })
      .collect();
    
    if descriptor_caches.is_empty() {
      return Err(anyhow!("Pact file does not contain any Protobuf descriptors"));
    }

    trace!("Built {} descriptor caches", descriptor_caches.len());
    
    // Step 3: Build routes using pre-built descriptor caches
    self.routes = self.pact.interactions.iter()
      .filter_map(|i| i.as_v4_sync_message())
      .filter_map(|i| {
        // Get descriptor key for this interaction
        let descriptor_key = lookup_interaction_config(&i)
          .and_then(|c| c.get("descriptorKey").map(json_to_string))?;
        
        // Get the pre-built cache (no parsing here!)
        let descriptor_cache = descriptor_caches.get(&descriptor_key)?.clone();
        
        // Lookup service/method from cache (no loading descriptors from the Pact file!)
        match lookup_service_and_method_for_interaction(&i, &descriptor_cache) {
          Ok((service, method, file)) => Some((descriptor_cache, service, method, file, i.clone())),
          Err(e) => {
            error!("Failed to lookup service/method for interaction {}: {}", i.description, e);
            None
          }
        }
      })
      .filter_map(|(descriptor_cache, service, method, file, i)| {
        match to_fully_qualified_name(service.name(), file.package()) {
          Ok(service_full_name) => {
            match build_grpc_route(service_full_name.as_str(), method.name()) {
              Ok(route) => Some((route, MockServerRoute::new(descriptor_cache, method, i))),
              Err(e) => {
                error!("Failed to build gRPC route for service {}, method {}: {}", service.name(), method.name(), e);
                None
              }
            }
          },
          Err(e) => {
            error!("Failed to build fully qualified name for service {}, package {:?}: {}", service.name(), file.package(), e);
            None
          }
        }
      })
      .fold(hashmap!{}, |mut acc, (key, route)| {
        match acc.entry(key) {
          Entry::Occupied(entry) => {
            entry.into_mut().messages.extend_from_slice(&route.messages);
          }
          Entry::Vacant(entry) => {
            entry.insert(route);
          }
        }
        acc
      });
    
    trace!("Mock server routes created: {:?}", self.routes.keys().collect::<Vec<_>>());

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
  type Response = Response<Body>;
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
        Ok(content_type) => if content_type.to_string().starts_with("application/grpc") {
          let method = req.method();
          if method == Method::POST {
            let request_path = req.uri().path();
            debug!(?request_path, "gRPC request received");
            if let Some((service_full_name, method)) = parse_grpc_route(request_path) {
              if let Some(route) = routes.get(request_path) &&
                 let Some(message) = route.messages.first() {
                trace!(message = message.description.as_str(), "Found route for service call");
                
                let service_and_method = format!("{service_full_name}/{method}");  // just for logging
                let input_name = route.method_descriptor.input_type.as_ref().expect(format!(
                  "Input message name is empty for service {}", service_and_method.as_str()).as_str());
                let output_name = route.method_descriptor.output_type.as_ref().expect(format!(
                  "Output message name is empty for service {}", service_and_method.as_str()).as_str());

                let descriptor_cache = &route.fds;
                if let Ok((input_message, _)) = descriptor_cache.find_message_descriptor_for_type(input_name) {
                  if let Ok((output_message, _)) = descriptor_cache.find_message_descriptor_for_type(output_name) {
                    let codec = PactCodec::new(descriptor_cache, &input_message, &output_message, message);
                    let mock_service = MockService::new(descriptor_cache, service_full_name.as_str(),
                      route, &input_message, &output_message, server_key.as_str(), pact);
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

fn invalid_media() -> Response<Body> {
  http::Response::builder()
    .status(415)
    .body(Body::empty())
    .unwrap()
}

fn invalid_method() -> Response<Body> {
  http::Response::builder()
    .status(405)
    .body(Body::empty())
    .unwrap()
}

fn invalid_path() -> Response<Body> {
  http::Response::builder()
    .status(200)
    .header("grpc-status", "12")
    .header("content-type", "application/grpc")
    .body(Body::empty())
    .unwrap()
}

fn failed_precondition() -> Response<Body> {
  http::Response::builder()
    .status(200)
    .header("grpc-status", "9")
    .header("content-type", "application/grpc")
    .body(Body::empty())
    .unwrap()
}
