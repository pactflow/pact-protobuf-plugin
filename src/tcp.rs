//! TCP support classes

use std::pin::Pin;
use std::task::{Context, Poll};

use futures::Stream;
use tokio::net::{TcpListener, TcpStream};
use tracing::error;

/// This struct is required, because we want to get the port of the running server to display
/// to standard out. This maps a TcpListener (which we use to get the port) to a futures Stream
/// required by the Tonic Server builder.
pub struct TcpIncoming {
  pub inner: TcpListener
}

// Implement futures Stream required by Tonic
impl Stream for TcpIncoming {
  type Item = Result<TcpStream, std::io::Error>;

  // Delegates to the poll_accept method of the inner TcpListener
  fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
    Pin::new(&mut self.inner).poll_accept(cx)
      .map_err(|err| {
        error!("Failed to accept connection: {}", err);
        err
      })
      .map_ok(|(stream, _)| stream)
      .map(Some)
  }
}
