/// Errors that can occur in the transport layer.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    /// The connection was closed.
    #[error("connection closed: {0}")]
    ConnectionClosed(String),

    /// Sending data failed.
    #[error("send failed: {0}")]
    SendFailed(#[source] std::io::Error),

    /// Receiving data failed.
    #[error("receive failed: {0}")]
    ReceiveFailed(#[source] std::io::Error),

    /// Binding or accepting connections failed.
    #[error("accept failed: {0}")]
    AcceptFailed(#[source] std::io::Error),

    /// The transport was shut down.
    #[error("transport shut down")]
    Shutdown,
}
