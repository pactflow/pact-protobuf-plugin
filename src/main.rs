//! Pact plugin for Protobuf and gRPC.
//!
//! Implements the version 1 of the Pact plugin interface described at `https://github.com/pact-foundation/pact-plugins/blob/main/docs/content-matcher-design.md`.

use std::env;
use std::iter::once;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::Context;
use clap::{Arg, ArgAction, Command, command};
use clap::error::ErrorKind;
use hyper::header;
use lazy_static::lazy_static;
use pact_plugin_driver::proto::pact_plugin_server::PactPluginServer;
use tokio::net::TcpListener;
use tokio::sync::oneshot::channel;
use tokio::time;
use tonic::{Request, Status};
use tonic::service::Interceptor;
use tonic::transport::Server;
use tower::ServiceBuilder;
use tower_http::compression::CompressionLayer;
use tower_http::sensitive_headers::SetSensitiveHeadersLayer;
use tower_http::trace::{DefaultMakeSpan, TraceLayer};
use tracing::info;
use tracing_bunyan_formatter::{BunyanFormattingLayer, JsonStorageLayer};
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::FmtSubscriber;
use tracing_subscriber::layer::SubscriberExt;
use uuid::Uuid;

use pact_protobuf_plugin::server::ProtobufPactPlugin;
use pact_protobuf_plugin::tcp::TcpIncoming;

/// Interceptor to check the server key for the request
#[derive(Debug, Clone, Default)]
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

fn integer_value(v: &str) -> Result<u64, String> {
  v.parse::<u64>().map_err(|e| format!("'{}' is not a valid integer value: {}", v, e) )
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
      .with_ansi(false)
      .with_writer(non_blocking.and(std::io::stdout))
      .finish()
      .with(JsonStorageLayer)
      .with(formatting_layer);

    if let Err(err) = tracing::subscriber::set_global_default(subscriber) {
      eprintln!("WARN: Failed to initialise global tracing subscriber - {err}");
    };

    let app = cli();
    let matches = match app.try_get_matches() {
      Ok(matches) => matches,
      Err(err) => return match err.kind() {
        ErrorKind::DisplayHelp => {
          println!("{}", err);
          Ok(())
        },
        ErrorKind::DisplayVersion => {
          println!("{}", clap::crate_version!());
          Ok(())
        },
        _ => {
          err.exit();
        }
      }
    };

    let plugin = ProtobufPactPlugin::new();

    // Bind to a OS provided port and create a TCP listener
    let host = plugin.host_to_bind_to()
      .or_else(|| matches.get_one::<String>("host").cloned())
      .unwrap_or_else(|| "[::1]".to_string());
    let addr: SocketAddr = format!("{}:0", host).parse()
      .with_context(|| format!("Failed to parse the host '{}'", host))?;
    let listener = TcpListener::bind(addr)
      .await
      .with_context(|| format!("Failed to bind to host '{}'", host))?;
    let address = listener.local_addr()?;

    // Generate a server key and then output the required startup JSON message to standard out
    let server_key = Uuid::new_v4().to_string();
    println!("{{\"port\":{}, \"serverKey\":\"{}\"}}", address.port(), server_key);

    // Build our middleware stack
    let layer = ServiceBuilder::new()
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
    let (snd, rcr) = channel::<()>();
    update_access_time();

    let timeout = matches.get_one::<u64>("timeout").copied()
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
      .add_service(PactPluginServer::with_interceptor(plugin, AuthInterceptor { server_key }))
      .serve_with_incoming_shutdown(
        TcpIncoming { inner: listener },
        async move {
          let _ = rcr.await;
          info!("Received shutdown signal, shutting plugin down");
        }
      ).await?;

    Ok(())
}

fn cli() -> Command {
  command!()
    .disable_version_flag(true)
    .disable_help_flag(true)
    .arg(Arg::new("help")
      .long("help")
      .action(ArgAction::Help)
      .help("Print help and exit"))
    .arg(Arg::new("version")
      .short('v')
      .long("version")
      .action(ArgAction::Version)
      .help("Print version information and exit"))
    .arg(Arg::new("timeout")
      .short('t')
      .long("timeout")
      .action(ArgAction::Set)
      .help("Timeout in seconds to use for inactivity to shutdown the plugin process. Default is 600 seconds (10 minutes)")
      .value_parser(integer_value)
    )
    .arg(Arg::new("host")
      .short('h')
      .long("host")
      .action(ArgAction::Set)
      .help("Host to bind to. Defaults to [::1], which is the IP6 loopback address")
    )
}

pub fn update_access_time() {
  let mut guard = SHUTDOWN_TIMER.lock().unwrap();
  *guard = Some(Instant::now());
}

#[cfg(test)]
mod tests {
  use crate::cli;

  #[test]
  fn verify_cli() {
    cli().debug_assert();
  }
}
