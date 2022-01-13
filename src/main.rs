//! Pact plugin for Protobuf and gRPC.
//!
//! Implements the version 1 of the Pact plugin interface described at `https://github.com/pact-foundation/pact-plugins/blob/main/docs/content-matcher-design.md`.

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::pin::Pin;
use std::str::FromStr;

use clap::{App, ErrorKind};
use futures::Stream;
use futures::task::{Context, Poll};
use log4rs;
use log4rs::append::console::ConsoleAppender;
use log4rs::append::file::FileAppender;
use log4rs::{Config, Handle};
use log4rs::config::{Appender, load_config_file, Logger, Root};
use log4rs::encode::pattern::PatternEncoder;
use log::LevelFilter;
use pact_plugin_driver::proto::pact_plugin_server::PactPluginServer;
use tokio::net::{TcpListener, TcpStream};
use tonic::transport::Server;
use uuid::Uuid;

use pact_protobuf_plugin::server::ProtobufPactPlugin;

/// This struct is required, because we want to get the port of the running server to display
/// to standard out. This maps a TcpListener (which we use to get the port) to a futures Stream
/// required by the Tonic Server builder.
struct TcpIncoming {
    inner: TcpListener
}

// Implement futures Stream required by Tonic
impl Stream for TcpIncoming {
    type Item = Result<TcpStream, std::io::Error>;

    // Delegates to the poll_accept method of the inner TcpListener
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner).poll_accept(cx)
          .map_ok(|(stream, _)| stream).map(Some)
    }
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
    let log_config = PathBuf::new().join("./log-config.yaml");
    let log_level = env::var("LOG_LEVEL").unwrap_or("INFO".to_string());
    let level = LevelFilter::from_str(log_level.as_str()).unwrap_or(LevelFilter::Info);
    if log_config.exists() {
      let mut config = load_config_file(log_config, Default::default())?;
      config.root_mut().set_level(level);
      log4rs::init_config(config)?;
    } else {
      init_default_logging(level)?;
    };

    let args: Vec<String> = env::args().collect();
    let program = args[0].clone();
    let app = App::new(program)
      .version(clap::crate_version!())
      .about("Pact Protobuf plugin")
      .version_short("v");

    if let Err(err) = app.get_matches_safe() {
      match err.kind {
        ErrorKind::HelpDisplayed => {
          println!("{}", err.message);
          return Ok(())
        },
        ErrorKind::VersionDisplayed => {
          println!();
          return Ok(())
        },
        _ => {}
      }
    }

    // Bind to a OS provided port and create a TCP listener
    let addr: SocketAddr = "0.0.0.0:0".parse()?;
    let listener = TcpListener::bind(addr).await?;
    let address = listener.local_addr()?;

    // Generate a server key and then output the required startup JSON message to standard out
    let server_key = Uuid::new_v4().to_string();
    println!("{{\"port\":{}, \"serverKey\":\"{}\"}}", address.port(), server_key);

    // Create the gRPC server listening on the previously created TCP listener
    let plugin = ProtobufPactPlugin::new();
    Server::builder()
      .add_service(PactPluginServer::new(plugin))
      .serve_with_incoming(TcpIncoming { inner: listener }).await?;

    Ok(())
}

fn init_default_logging(log_level: LevelFilter) -> anyhow::Result<Handle> {
  let encoder = PatternEncoder::new("{d(%Y-%m-%dT%H:%M:%S%Z)} {l} [{T}] {t} - {m}{n}");
  let stdout = ConsoleAppender::builder()
    .encoder(Box::new(encoder.clone()))
    .build();
  let file = FileAppender::builder()
    .encoder(Box::new(encoder))
    .build("plugin.log")?;

  let config = Config::builder()
    .appender(Appender::builder().build("stdout", Box::new(stdout)))
    .appender(Appender::builder().build("file", Box::new(file)))
    .logger(Logger::builder().build("h2", LevelFilter::Info))
    .logger(Logger::builder().build("hyper", LevelFilter::Info))
    .logger(Logger::builder().build("tracing", LevelFilter::Warn))
    .logger(Logger::builder().build("tokio", LevelFilter::Info))
    .logger(Logger::builder().build("tokio_util", LevelFilter::Info))
    .logger(Logger::builder().build("mio", LevelFilter::Info))
    .build(Root::builder()
      .appender("stdout")
      .appender("file")
      .build(log_level))?;

  Ok(log4rs::init_config(config)?)
}
