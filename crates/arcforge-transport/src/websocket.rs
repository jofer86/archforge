//! WebSocket transport implementation using `tokio-tungstenite`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;

use crate::{Connection, ConnectionId, Transport, TransportError};

/// Counter for generating unique connection IDs.
static NEXT_CONNECTION_ID: AtomicU64 = AtomicU64::new(1);

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>;

/// A WebSocket-based [`Transport`] that listens for incoming connections.
pub struct WebSocketTransport {
    listener: TcpListener,
}

impl WebSocketTransport {
    /// Binds a new WebSocket transport to the given address.
    pub async fn bind(addr: &str) -> Result<Self, TransportError> {
        let listener = TcpListener::bind(addr).await.map_err(|e| {
            TransportError::AcceptFailed(e)
        })?;
        tracing::info!(addr, "WebSocket transport listening");
        Ok(Self { listener })
    }
}

impl Transport for WebSocketTransport {
    type Connection = WebSocketConnection;
    type Error = TransportError;

    async fn accept(&mut self) -> Result<Self::Connection, Self::Error> {
        let (stream, addr) = self
            .listener
            .accept()
            .await
            .map_err(TransportError::AcceptFailed)?;

        let ws = tokio_tungstenite::accept_async(stream)
            .await
            .map_err(|e| {
                TransportError::AcceptFailed(std::io::Error::new(
                    std::io::ErrorKind::ConnectionRefused,
                    e,
                ))
            })?;

        let id = ConnectionId::new(
            NEXT_CONNECTION_ID.fetch_add(1, Ordering::Relaxed),
        );
        tracing::debug!(%id, %addr, "accepted WebSocket connection");

        Ok(WebSocketConnection {
            id,
            ws: Arc::new(Mutex::new(ws)),
        })
    }

    async fn shutdown(&self) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// A single WebSocket connection.
pub struct WebSocketConnection {
    id: ConnectionId,
    ws: Arc<Mutex<WsStream>>,
}

impl Connection for WebSocketConnection {
    type Error = TransportError;

    async fn send(&self, data: &[u8]) -> Result<(), Self::Error> {
        use futures_util::SinkExt;
        let msg = Message::Binary(data.to_vec().into());
        self.ws
            .lock()
            .await
            .send(msg)
            .await
            .map_err(|e| {
                TransportError::SendFailed(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    e,
                ))
            })
    }

    async fn recv(&self) -> Result<Option<Vec<u8>>, Self::Error> {
        use futures_util::StreamExt;
        loop {
            let msg = self.ws.lock().await.next().await;
            match msg {
                Some(Ok(Message::Binary(data))) => {
                    return Ok(Some(data.into()));
                }
                Some(Ok(Message::Text(text))) => {
                    return Ok(Some(text.as_bytes().to_vec()));
                }
                Some(Ok(Message::Close(_))) | None => return Ok(None),
                Some(Ok(_)) => continue, // skip ping/pong/frame
                Some(Err(e)) => {
                    return Err(TransportError::ReceiveFailed(
                        std::io::Error::new(
                            std::io::ErrorKind::ConnectionReset,
                            e,
                        ),
                    ));
                }
            }
        }
    }

    async fn close(&self) -> Result<(), Self::Error> {
        self.ws.lock().await.close(None).await.map_err(|e| {
            TransportError::SendFailed(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                e,
            ))
        })
    }

    fn id(&self) -> ConnectionId {
        self.id
    }
}
