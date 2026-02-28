//! Error types for the room layer.

use arcforge_protocol::{PlayerId, RoomId};

/// Errors that can occur during room operations.
#[derive(Debug, thiserror::Error)]
pub enum RoomError {
    /// The room does not exist.
    #[error("room {0} not found")]
    NotFound(RoomId),

    /// The room is full — no more player slots available.
    #[error("room {0} is full")]
    RoomFull(RoomId),

    /// The player is already in this room.
    #[error("player {0} already in room {1}")]
    AlreadyInRoom(PlayerId, RoomId),

    /// The player is not in this room.
    #[error("player {0} not in room {1}")]
    NotInRoom(PlayerId, RoomId),

    /// The room is in a state that doesn't allow this operation.
    /// For example, trying to join a room that's already Finished.
    #[error("invalid room state for this operation: {0}")]
    InvalidState(String),

    /// The room's command channel is full or closed.
    #[error("room {0} is unavailable")]
    Unavailable(RoomId),
}
