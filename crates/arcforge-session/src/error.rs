//! Error types for the session layer.

/// Errors that can occur during session management.
///
/// These cover the full lifecycle of a player session: authentication,
/// creation, reconnection, and expiration.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// Authentication failed — the token was invalid, expired, or rejected
    /// by the [`Authenticator`](crate::Authenticator).
    #[error("authentication failed: {0}")]
    AuthFailed(String),

    /// No session exists for the given player.
    /// This happens when trying to disconnect or reconnect a player
    /// who was never connected (or whose session already expired).
    #[error("session not found for player {0}")]
    NotFound(arcforge_protocol::PlayerId),

    /// The reconnection token doesn't match what the server issued.
    /// Could be a stale token, a typo, or a malicious attempt.
    #[error("invalid reconnection token")]
    InvalidToken,

    /// The session's reconnection grace period has elapsed.
    /// The player took too long to reconnect after disconnecting.
    #[error("session expired for player {0}")]
    SessionExpired(arcforge_protocol::PlayerId),

    /// The player already has an active (Connected) session.
    /// A player can only have one session at a time.
    #[error("player {0} already has an active session")]
    AlreadyConnected(arcforge_protocol::PlayerId),
}
