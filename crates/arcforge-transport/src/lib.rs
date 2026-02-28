//! Transport abstraction layer for Arcforge.
//!
//! Provides the [`Transport`] and [`Connection`] traits that abstract over
//! different network protocols (WebSocket, WebTransport).
//!
//! # Feature Flags
//!
//! - `websocket` (default) — WebSocket transport via `tokio-tungstenite`

#![allow(async_fn_in_trait)]

mod error;
#[cfg(feature = "websocket")]
mod websocket;

pub use error::TransportError;
#[cfg(feature = "websocket")]
pub use websocket::{WebSocketConnection, WebSocketTransport};

use std::fmt;

/// Opaque identifier for a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionId(u64);

impl ConnectionId {
    /// Creates a new `ConnectionId` from a raw `u64`.
    pub fn new(id: u64) -> Self {
        Self(id)
    }

    /// Returns the underlying `u64` value.
    pub fn into_inner(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ConnectionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "conn-{}", self.0)
    }
}

/// Accepts new incoming connections.
pub trait Transport: Send + Sync + 'static {
    /// The connection type produced by this transport.
    type Connection: Connection;
    /// The error type for transport operations.
    type Error: std::error::Error + Send + Sync;

    /// Waits for and accepts the next incoming connection.
    async fn accept(&mut self) -> Result<Self::Connection, Self::Error>;

    /// Gracefully shuts down the transport, stopping new connections.
    async fn shutdown(&self) -> Result<(), Self::Error>;
}

/// A single connection that can send and receive bytes.
pub trait Connection: Send + Sync + 'static {
    /// The error type for connection operations.
    type Error: std::error::Error + Send + Sync;

    /// Sends data to the remote peer.
    async fn send(&self, data: &[u8]) -> Result<(), Self::Error>;

    /// Receives the next message from the remote peer.
    ///
    /// Returns `Ok(None)` when the connection is cleanly closed.
    async fn recv(&self) -> Result<Option<Vec<u8>>, Self::Error>;

    /// Sends data over an unreliable channel.
    ///
    /// Defaults to reliable send. Transports that support unreliable delivery
    /// (e.g., WebTransport) should override this.
    async fn send_unreliable(
        &self,
        data: &[u8],
    ) -> Result<(), Self::Error> {
        self.send(data).await
    }

    /// Closes the connection.
    async fn close(&self) -> Result<(), Self::Error>;

    /// Returns the unique identifier for this connection.
    fn id(&self) -> ConnectionId;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_id_new_and_into_inner() {
        let id = ConnectionId::new(42);
        assert_eq!(id.into_inner(), 42);
    }

    #[test]
    fn test_connection_id_display() {
        let id = ConnectionId::new(7);
        assert_eq!(id.to_string(), "conn-7");
    }

    #[test]
    fn test_connection_id_equality() {
        let a = ConnectionId::new(1);
        let b = ConnectionId::new(1);
        let c = ConnectionId::new(2);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_connection_id_hash_works_as_map_key() {
        // ConnectionId derives Hash, so it should work as a HashMap key.
        use std::collections::HashMap;
        let mut map = HashMap::new();
        map.insert(ConnectionId::new(1), "alice");
        map.insert(ConnectionId::new(2), "bob");
        assert_eq!(map[&ConnectionId::new(1)], "alice");
    }
}
