//! Pact plugin for Protobuf and gRPC.
//!
//! Implements the version 1 of the Pact plugin interface described at `https://github.com/pact-foundation/pact-plugins/blob/main/docs/content-matcher-design.md`.

use std::net::SocketAddr;
use std::pin::Pin;

use env_logger::Env;
use futures::Stream;
use futures::task::{Context, Poll};
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
          .map_ok(|(stream, _)| stream).map(|v| Some(v))
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
    let env = Env::new().filter("LOG_LEVEL");
    env_logger::init_from_env(env);

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
