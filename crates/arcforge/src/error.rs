//! Unified error type for the Arcforge framework.

use arcforge_protocol::ProtocolError;
use arcforge_room::RoomError;
use arcforge_session::SessionError;
use arcforge_transport::TransportError;

/// Top-level error that wraps all crate-specific errors.
///
/// When using the `arcforge` meta-crate, you deal with this single
/// error type instead of importing errors from each sub-crate.
/// The `#[from]` attribute on each variant auto-generates `From` impls,
/// so the `?` operator converts sub-crate errors automatically.
#[derive(Debug, thiserror::Error)]
pub enum ArcforgeError {
    /// A transport-level error (connection, send, recv).
    #[error(transparent)]
    Transport(#[from] TransportError),

    /// A protocol-level error (encode, decode, invalid message).
    #[error(transparent)]
    Protocol(#[from] ProtocolError),

    /// A session-level error (auth, reconnect, expired).
    #[error(transparent)]
    Session(#[from] SessionError),

    /// A room-level error (full, not found, invalid state).
    #[error(transparent)]
    Room(#[from] RoomError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_transport_error() {
        let err = TransportError::ConnectionClosed("gone".into());
        let arcforge_err: ArcforgeError = err.into();
        assert!(matches!(arcforge_err, ArcforgeError::Transport(_)));
        assert!(arcforge_err.to_string().contains("gone"));
    }

    #[test]
    fn test_from_protocol_error() {
        let err = ProtocolError::InvalidMessage("bad".into());
        let arcforge_err: ArcforgeError = err.into();
        assert!(matches!(arcforge_err, ArcforgeError::Protocol(_)));
    }

    #[test]
    fn test_from_session_error() {
        let err = SessionError::AuthFailed("nope".into());
        let arcforge_err: ArcforgeError = err.into();
        assert!(matches!(arcforge_err, ArcforgeError::Session(_)));
    }

    #[test]
    fn test_from_room_error() {
        let err = RoomError::NotFound(arcforge_protocol::RoomId(1));
        let arcforge_err: ArcforgeError = err.into();
        assert!(matches!(arcforge_err, ArcforgeError::Room(_)));
    }
}
