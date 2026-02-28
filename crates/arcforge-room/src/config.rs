//! Room configuration and state machine.

use std::time::Duration;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// RoomConfig
// ---------------------------------------------------------------------------

/// Configuration for a room instance.
///
/// Game developers can override these defaults by implementing
/// `GameLogic::room_config()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomConfig {
    /// Minimum players required to start the game.
    pub min_players: usize,

    /// Maximum players allowed in the room.
    pub max_players: usize,

    /// Tick rate in Hz. 0 means event-driven (no tick loop).
    pub tick_rate: u32,

    /// How long to wait for a disconnected player before removing them.
    pub reconnect_grace: Duration,

    /// Whether spectators are allowed.
    pub allow_spectators: bool,

    /// Maximum number of spectators (0 = unlimited when allowed).
    pub max_spectators: usize,
}

impl Default for RoomConfig {
    fn default() -> Self {
        Self {
            min_players: 2,
            max_players: 8,
            tick_rate: 0,
            reconnect_grace: Duration::from_secs(30),
            allow_spectators: false,
            max_spectators: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// RoomState
// ---------------------------------------------------------------------------

/// The lifecycle state of a room.
///
/// Transitions are strictly ordered — no skipping states:
///
/// ```text
/// WaitingForPlayers → Starting → InProgress → Finished → Destroying
/// ```
///
/// - **WaitingForPlayers**: Room exists, accepting joins. Not enough
///   players to start yet.
/// - **Starting**: Minimum players reached. Game is initializing
///   (loading state, sending initial data to clients).
/// - **InProgress**: Game is actively running. Players send game
///   messages, tick loop is active (if configured).
/// - **Finished**: Game ended (someone won, draw, etc.). Players
///   can see final state but can't send game messages.
/// - **Destroying**: Room is being cleaned up. All players removed,
///   resources freed. After this the room is gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoomState {
    WaitingForPlayers,
    Starting,
    InProgress,
    Finished,
    Destroying,
}

impl RoomState {
    /// Returns `true` if the room is accepting new players.
    pub fn is_joinable(&self) -> bool {
        matches!(self, Self::WaitingForPlayers)
    }

    /// Returns `true` if the room is actively running a game.
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Starting | Self::InProgress)
    }

    /// Attempts to transition to the next state.
    ///
    /// Returns `Some(next)` if the transition is valid, `None` if not.
    /// This enforces the strict ordering of the state machine.
    pub fn next(self) -> Option<Self> {
        match self {
            Self::WaitingForPlayers => Some(Self::Starting),
            Self::Starting => Some(Self::InProgress),
            Self::InProgress => Some(Self::Finished),
            Self::Finished => Some(Self::Destroying),
            Self::Destroying => None,
        }
    }

    /// Returns `true` if transitioning to `target` is valid.
    pub fn can_transition_to(self, target: Self) -> bool {
        self.next() == Some(target)
    }
}

impl std::fmt::Display for RoomState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WaitingForPlayers => write!(f, "WaitingForPlayers"),
            Self::Starting => write!(f, "Starting"),
            Self::InProgress => write!(f, "InProgress"),
            Self::Finished => write!(f, "Finished"),
            Self::Destroying => write!(f, "Destroying"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_room_state_next_follows_strict_order() {
        assert_eq!(
            RoomState::WaitingForPlayers.next(),
            Some(RoomState::Starting)
        );
        assert_eq!(RoomState::Starting.next(), Some(RoomState::InProgress));
        assert_eq!(
            RoomState::InProgress.next(),
            Some(RoomState::Finished)
        );
        assert_eq!(
            RoomState::Finished.next(),
            Some(RoomState::Destroying)
        );
        assert_eq!(RoomState::Destroying.next(), None);
    }

    #[test]
    fn test_room_state_can_transition_to() {
        assert!(RoomState::WaitingForPlayers
            .can_transition_to(RoomState::Starting));
        assert!(!RoomState::WaitingForPlayers
            .can_transition_to(RoomState::InProgress));
        assert!(!RoomState::Finished
            .can_transition_to(RoomState::WaitingForPlayers));
    }

    #[test]
    fn test_room_state_is_joinable() {
        assert!(RoomState::WaitingForPlayers.is_joinable());
        assert!(!RoomState::Starting.is_joinable());
        assert!(!RoomState::InProgress.is_joinable());
        assert!(!RoomState::Finished.is_joinable());
        assert!(!RoomState::Destroying.is_joinable());
    }

    #[test]
    fn test_room_state_is_active() {
        assert!(!RoomState::WaitingForPlayers.is_active());
        assert!(RoomState::Starting.is_active());
        assert!(RoomState::InProgress.is_active());
        assert!(!RoomState::Finished.is_active());
        assert!(!RoomState::Destroying.is_active());
    }

    #[test]
    fn test_room_state_display() {
        assert_eq!(RoomState::WaitingForPlayers.to_string(), "WaitingForPlayers");
        assert_eq!(RoomState::InProgress.to_string(), "InProgress");
    }

    #[test]
    fn test_room_config_default() {
        let config = RoomConfig::default();
        assert_eq!(config.min_players, 2);
        assert_eq!(config.max_players, 8);
        assert_eq!(config.tick_rate, 0);
        assert!(!config.allow_spectators);
    }
}
