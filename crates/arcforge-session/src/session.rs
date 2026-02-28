//! Session types: the data structures that represent a player's connection.
//!
//! A "session" is the server's record of a connected player. It tracks:
//! - WHO the player is (`PlayerId`)
//! - WHAT state they're in (connected, disconnected, expired)
//! - HOW they can reconnect (a secret token)
//! - WHEN they disconnected (so we know when to expire them)

use std::time::Instant;

use arcforge_protocol::PlayerId;

// ---------------------------------------------------------------------------
// SessionConfig
// ---------------------------------------------------------------------------

/// Configuration for session behavior.
///
/// This controls timeouts and limits. Game developers can customize these
/// when setting up the server. Sensible defaults are provided.
///
/// `#[derive(Clone)]` is needed because the config is shared — the
/// `SessionManager` stores one copy, and individual sessions may
/// reference it too.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// How long (in seconds) a disconnected player has to reconnect
    /// before their session is permanently expired.
    ///
    /// Default: 30 seconds. Set to 0 to disable reconnection entirely.
    pub reconnect_grace_secs: u64,
}

/// `Default` provides a "sensible starting point" for a type.
/// You can create a config with `SessionConfig::default()` and then
/// override just the fields you care about.
impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            reconnect_grace_secs: 30,
        }
    }
}

// ---------------------------------------------------------------------------
// SessionState
// ---------------------------------------------------------------------------

/// The current state of a player's session.
///
/// This is a state machine with three states:
///
/// ```text
///   Connected ──(disconnect)──→ Disconnected ──(timeout)──→ Expired
///       ↑                            │
///       └────────(reconnect)─────────┘
/// ```
///
/// - **Connected**: Player is actively connected and can send/receive.
/// - **Disconnected**: Player lost connection but may come back.
///   The `since` field records WHEN they disconnected, so we can
///   check if the grace period has elapsed.
/// - **Expired**: Grace period elapsed. Session is dead and will be
///   cleaned up. The player must authenticate again to get a new session.
///
/// `Instant` is Rust's monotonic clock — it always moves forward and
/// isn't affected by system clock changes. Perfect for measuring elapsed
/// time.
#[derive(Debug, Clone)]
pub enum SessionState {
    /// Player is actively connected.
    Connected,

    /// Player disconnected at the given instant.
    /// They have until `since + grace_period` to reconnect.
    Disconnected { since: Instant },

    /// Session has expired and will be cleaned up.
    Expired,
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

/// A single player's session on the server.
///
/// Created when a player successfully authenticates. Lives until the
/// player disconnects and the grace period expires (or the server shuts down).
#[derive(Debug, Clone)]
pub struct Session {
    /// Which player this session belongs to.
    pub player_id: PlayerId,

    /// Current lifecycle state (connected, disconnected, or expired).
    pub state: SessionState,

    /// A secret token the player can use to reconnect after a disconnect.
    ///
    /// When a player first connects, the server generates a random token
    /// and sends it to the client. If the client disconnects (e.g., WiFi
    /// drops), it can reconnect by presenting this token instead of
    /// re-authenticating. This avoids kicking players from a game just
    /// because of a brief network hiccup.
    ///
    /// The token is a 32-character hex string (128 bits of randomness).
    pub reconnect_token: String,
}
