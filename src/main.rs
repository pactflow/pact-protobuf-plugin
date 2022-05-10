//! Pact plugin for Protobuf and gRPC.
//!
//! Implements the version 1 of the Pact plugin interface described at `https://github.com/pact-foundation/pact-plugins/blob/main/docs/content-matcher-design.md`.

use std::env;
use std::iter::once;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use clap::{App, Arg, ErrorKind};
use hyper::header;
use lazy_static::lazy_static;
use pact_plugin_driver::proto::pact_plugin_server::PactPluginServer;
use tokio::net::TcpListener;
use tokio::sync::oneshot::channel;
use tokio::time;
use tonic::{Request, Status};
use tonic::service::{interceptor, Interceptor};
use tonic::transport::Server;
use tower::ServiceBuilder;
use tower_http::compression::CompressionLayer;
use tower_http::sensitive_headers::SetSensitiveHeadersLayer;
use tower_http::trace::{DefaultMakeSpan, TraceLayer};
use tracing::{info, warn};
use tracing_bunyan_formatter::{BunyanFormattingLayer, JsonStorageLayer};
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::{FmtSubscriber, Registry};
use tracing_subscriber::layer::SubscriberExt;
use uuid::Uuid;

use pact_protobuf_plugin::server::ProtobufPactPlugin;
use pact_protobuf_plugin::tcp::TcpIncoming;

/// Interceptor to check the server key for the request
#[derive(Debug, Clone)]
struct AuthInterceptor {
  pub server_key: String
}

impl Interceptor for AuthInterceptor {
  fn call(&mut self, request: Request<()>) -> Result<Request<()>, Status> {
    update_access_time();
    if let Some(auth) = request.metadata().get("authorization") {
      if let Ok(auth) = auth.to_str() {
        if self.server_key == auth {
          Ok(request)
        } else {
          Err(Status::unauthenticated("invalid credentials supplied"))
        }
      } else {
        Err(Status::unauthenticated("could not read credentials supplied"))
      }
    } else {
      Err(Status::unauthenticated("no credentials supplied"))
    }
  }
}

lazy_static! {
  pub static ref SHUTDOWN_TIMER: Mutex<Option<Instant>> = Mutex::new(None);
}

/// Maximum time to wait when there is no activity to shut the plugin down (10 minutes)
const MAX_TIME: u64 = 600;

fn integer_value(v: String) -> Result<(), String> {
  v.parse::<u64>().map(|_| ()).map_err(|e| format!("'{}' is not a valid integer value: {}", v, e) )
}

/// Main method of the plugin process. This will start a gRPC server using the plugin proto file
/// (`https://github.com/pact-foundation/pact-plugins/blob/main/proto/plugin.proto`) and then
/// output the port the server is running on as well as a server key required to access the
/// gRPC server.
///
/// Log level will be passed in using the `LOG_LEVEL` environment variable.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Setup the logging system based on the LOG_LEVEL environment variable
    let log_level = env::var("LOG_LEVEL").unwrap_or_else(|_| "INFO".to_string());
    let file_appender = tracing_appender::rolling::daily("./log", "plugin.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    let json_appender = tracing_appender::rolling::daily("./log", "plugin.log.json");
    let (json_non_blocking, _json_guard) = tracing_appender::non_blocking(json_appender);

    // Setup tracing
    let formatting_layer = BunyanFormattingLayer::new("pact-protobuf-plugin".into(), json_non_blocking);
    let subscriber = FmtSubscriber::builder()
      .with_max_level(tracing_core::LevelFilter::from_str(log_level.as_str())
        .unwrap_or(tracing_core::LevelFilter::INFO))
      .with_thread_names(true)
      .with_writer(non_blocking.and(std::io::stdout))
      .finish()
      .with(JsonStorageLayer)
      .with(formatting_layer);

    if let Err(err) = tracing::subscriber::set_global_default(subscriber) {
      warn!("Failed to initialise global tracing subscriber - {err}");
    };

    let args: Vec<String> = env::args().collect();
    let program = args[0].clone();
    let app = App::new(program)
      .version(clap::crate_version!())
      .about("Pact Protobuf plugin")
      .version_short("v")
      .arg(Arg::with_name("timeout")
        .short("t")
        .long("timeout")
        .takes_value(true)
        .use_delimiter(false)
        .help("Timeout to use for inactivity to shutdown the plugin process. Default is 600 seconds (10 minutes)")
        .validator(integer_value)
      );

    let matches = match app.get_matches_safe() {
      Ok(matches) => matches,
      Err(err) => return match err.kind {
        ErrorKind::HelpDisplayed => {
          println!("{}", err.message);
          Ok(())
        },
        ErrorKind::VersionDisplayed => {
          println!();
          Ok(())
        },
        _ => {
          err.exit();
        }
      }
    };

    // Bind to a OS provided port and create a TCP listener
    let addr: SocketAddr = "[::1]:0".parse()?;
    let listener = TcpListener::bind(addr).await?;
    let address = listener.local_addr()?;

    // Generate a server key and then output the required startup JSON message to standard out
    let server_key = Uuid::new_v4().to_string();
    println!("{{\"port\":{}, \"serverKey\":\"{}\"}}", address.port(), server_key);

    // Build our middleware stack
    let layer = ServiceBuilder::new()
      .layer(interceptor(AuthInterceptor { server_key }))
      // Compress responses
      .layer(CompressionLayer::new())
      // Mark the `Authorization` header as sensitive so it doesn't show in logs
      .layer(SetSensitiveHeadersLayer::new(once(header::AUTHORIZATION)))
      // Log all requests and responses
      .layer(
        TraceLayer::new_for_grpc()
          .make_span_with(DefaultMakeSpan::new().include_headers(true)),
      )
      .into_inner();

    // Create the gRPC server listening on the previously created TCP listener
    let plugin = ProtobufPactPlugin::new();
    let (snd, rcr) = channel::<()>();
    update_access_time();

    let timeout = matches.value_of("timeout")
      .map(|port| port.parse::<u64>().unwrap())
      .unwrap_or(MAX_TIME);
    tokio::spawn(async move {
      let mut interval = time::interval(Duration::from_secs(10));
      let mut elapsed = false;
      while !elapsed {
        interval.tick().await;
        {
          let guard = SHUTDOWN_TIMER.lock().unwrap();
          if let Some(i) = &*guard {
            if i.elapsed().as_secs() > timeout {
              info!("No activity for more than {timeout} seconds, sending shutdown signal");
              elapsed = true;
            }
          }
        }
      }
      let _ = snd.send(());
    });
    Server::builder()
      .layer(layer)
      .add_service(PactPluginServer::new(plugin))
      .serve_with_incoming_shutdown(
        TcpIncoming { inner: listener },
        async move {
          let _ = rcr.await;
          info!("Received shutdown signal, shutting plugin down");
        }
      ).await?;

    Ok(())
}

pub fn update_access_time() {
  let mut guard = SHUTDOWN_TIMER.lock().unwrap();
  *guard = Some(Instant::now());
}
