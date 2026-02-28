//! The `GameLogic` trait — the main extension point for game developers.
//!
//! This is the single trait that game developers implement. The framework
//! calls these methods at the right time; the developer just writes game
//! rules.

use std::time::Duration;

use arcforge_protocol::{PlayerId, Recipient};
use serde::{de::DeserializeOwned, Serialize};

use crate::RoomConfig;

/// The core trait that game developers implement.
///
/// Each associated type defines the shape of the game's data:
/// - `Config` — game-specific settings (board size, time limit, etc.)
/// - `State` — the full game state (board, scores, whose turn, etc.)
/// - `ClientMessage` — what clients can send (moves, actions)
/// - `ServerMessage` — what the server sends back (state updates, events)
///
/// The framework calls `init` to create the initial state, routes client
/// messages through `handle_message`, and optionally calls `tick` for
/// real-time games.
pub trait GameLogic: Send + Sync + 'static {
    /// Game-specific configuration (e.g., board size, time limit).
    type Config: Send + Sync + Clone + Default;

    /// The full game state. Must be serializable so the framework can
    /// send snapshots to clients.
    type State: Send + Sync + Clone + Serialize + DeserializeOwned;

    /// Messages that clients send to the server (e.g., "place marker at row 1, col 2").
    type ClientMessage: Send + Sync + Clone + Serialize + DeserializeOwned;

    /// Messages that the server sends to clients (e.g., "marker placed", "your turn").
    type ServerMessage: Send + Sync + Clone + Serialize + DeserializeOwned;

    /// Creates the initial game state when a room starts.
    ///
    /// Called once when the room transitions from WaitingForPlayers → Starting.
    /// `players` contains the IDs of all players who joined.
    fn init(config: &Self::Config, players: &[PlayerId]) -> Self::State;

    /// Processes a message from a client.
    ///
    /// This is where game rules live. Returns a list of messages to send
    /// back — each paired with a `Recipient` specifying who gets it.
    fn handle_message(
        state: &mut Self::State,
        sender: PlayerId,
        msg: Self::ClientMessage,
    ) -> Vec<(Recipient, Self::ServerMessage)>;

    /// Returns `true` if the game is over.
    ///
    /// Called after every `handle_message` and `tick`. When this returns
    /// `true`, the room transitions to Finished.
    fn is_finished(state: &Self::State) -> bool;

    /// Called every tick for real-time games.
    ///
    /// `dt` is the time since the last tick. Only called if
    /// `room_config().tick_rate > 0`. Default: no-op.
    fn tick(
        _state: &mut Self::State,
        _dt: Duration,
    ) -> Vec<(Recipient, Self::ServerMessage)> {
        Vec::new()
    }

    /// Validates a client message before processing.
    ///
    /// Called before `handle_message`. If this returns `Err`, the message
    /// is rejected and the error is sent back to the client. Default: accept all.
    fn validate_message(
        _state: &Self::State,
        _sender: PlayerId,
        _msg: &Self::ClientMessage,
    ) -> Result<(), String> {
        Ok(())
    }

    /// Called when a player disconnects from the room.
    ///
    /// Use this to pause the game, skip their turn, etc. Default: no-op.
    fn on_player_disconnect(
        _state: &mut Self::State,
        _player: PlayerId,
    ) -> Vec<(Recipient, Self::ServerMessage)> {
        Vec::new()
    }

    /// Called when a player reconnects to the room.
    ///
    /// Use this to resume the game, send them the current state, etc.
    /// Default: no-op.
    fn on_player_reconnect(
        _state: &mut Self::State,
        _player: PlayerId,
    ) -> Vec<(Recipient, Self::ServerMessage)> {
        Vec::new()
    }

    /// Returns the room configuration for this game type.
    ///
    /// Override to customize min/max players, tick rate, etc.
    /// Default: `RoomConfig::default()` (2-8 players, event-driven).
    fn room_config() -> RoomConfig {
        RoomConfig::default()
    }
}
