//! The session manager: tracks all active player sessions.
//!
//! This is the central piece of the session layer. It's responsible for:
//! - Creating sessions when players authenticate
//! - Tracking which players are connected/disconnected
//! - Validating reconnection tokens
//! - Expiring sessions after the grace period
//! - Cleaning up dead sessions to free memory
//!
//! # Concurrency note
//!
//! `SessionManager` is NOT thread-safe by itself — it uses a plain
//! `HashMap`, not a concurrent one. This is intentional: the session
//! manager is owned by a single task (the server's accept loop) and
//! accessed through a channel or mutex at a higher level. Keeping it
//! simple here avoids hidden locking overhead.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use arcforge_protocol::PlayerId;
use rand::Rng;

use crate::{Session, SessionConfig, SessionError, SessionState};

/// Manages all active player sessions.
///
/// Think of this as a "registry" — it knows about every player currently
/// connected (or recently disconnected) to the server.
///
/// ## Lifecycle
///
/// ```text
/// authenticate() ──→ create() ──→ disconnect() ──→ reconnect()
///                       │               │                │
///                       │               ▼                │
///                       │          expire_stale()        │
///                       │               │                │
///                       ▼               ▼                ▼
///                    [Connected]   [Disconnected]   [Connected]
///                                      │
///                                      ▼ (after grace period)
///                                  [Expired] ──→ cleanup()
/// ```
pub struct SessionManager {
    /// All active sessions, keyed by player ID.
    ///
    /// `HashMap` is Rust's hash table — O(1) average lookup by key.
    /// We use `PlayerId` as the key because a player can only have
    /// one session at a time.
    sessions: HashMap<PlayerId, Session>,

    /// An index from reconnection tokens to player IDs.
    ///
    /// When a client reconnects, they send a token (not a player ID).
    /// This map lets us quickly find which session the token belongs to
    /// without scanning every session. It's kept in sync with `sessions`.
    tokens: HashMap<String, PlayerId>,

    /// Configuration (grace period, etc.).
    config: SessionConfig,
}

impl SessionManager {
    /// Creates a new, empty session manager with the given config.
    pub fn new(config: SessionConfig) -> Self {
        Self {
            sessions: HashMap::new(),
            tokens: HashMap::new(),
            config,
        }
    }

    /// Creates a new session for a player after successful authentication.
    ///
    /// Generates a random reconnection token and stores the session.
    ///
    /// # Errors
    /// Returns [`SessionError::AlreadyConnected`] if the player already
    /// has an active (Connected) session.
    pub fn create(
        &mut self,
        player_id: PlayerId,
    ) -> Result<&Session, SessionError> {
        // Check if this player already has a connected session.
        // `if let` is Rust's way of pattern-matching a single case.
        // It says: "if this value matches the pattern, run this block."
        if let Some(existing) = self.sessions.get(&player_id) {
            if matches!(existing.state, SessionState::Connected) {
                return Err(SessionError::AlreadyConnected(player_id));
            }
            // If they have a disconnected/expired session, remove the
            // old token before creating a new session.
            self.tokens.remove(&existing.reconnect_token);
        }

        let token = generate_token();

        let session = Session {
            player_id,
            state: SessionState::Connected,
            reconnect_token: token.clone(),
        };

        // Insert into both maps to keep them in sync.
        self.tokens.insert(token, player_id);
        self.sessions.insert(player_id, session);

        tracing::info!(%player_id, "session created");

        // `unwrap` is safe here because we just inserted the entry.
        // This is one of the rare cases where unwrap is acceptable —
        // the invariant is guaranteed by the line above.
        Ok(self.sessions.get(&player_id).expect("just inserted"))
    }

    /// Marks a player as disconnected. Starts the reconnection grace period.
    ///
    /// The player's session isn't destroyed yet — they have
    /// `config.reconnect_grace_secs` to reconnect with their token.
    ///
    /// # Errors
    /// Returns [`SessionError::NotFound`] if no session exists.
    pub fn disconnect(
        &mut self,
        player_id: PlayerId,
    ) -> Result<(), SessionError> {
        let session = self
            .sessions
            .get_mut(&player_id)
            .ok_or(SessionError::NotFound(player_id))?;

        session.state = SessionState::Disconnected {
            since: Instant::now(),
        };

        tracing::info!(%player_id, "player disconnected, grace period started");
        Ok(())
    }

    /// Reconnects a player using their reconnection token.
    ///
    /// The client sends the token it received during the initial handshake.
    /// If the token is valid and the session hasn't expired, the session
    /// transitions back to Connected.
    ///
    /// # Errors
    /// - [`SessionError::InvalidToken`] — token not recognized
    /// - [`SessionError::SessionExpired`] — grace period elapsed
    pub fn reconnect(
        &mut self,
        token: &str,
    ) -> Result<&Session, SessionError> {
        // Look up which player this token belongs to.
        let player_id = self
            .tokens
            .get(token)
            .copied()
            .ok_or(SessionError::InvalidToken)?;

        let session = self
            .sessions
            .get_mut(&player_id)
            .ok_or(SessionError::InvalidToken)?;

        // Check if the session is in a reconnectable state.
        match &session.state {
            SessionState::Disconnected { since } => {
                let grace =
                    Duration::from_secs(self.config.reconnect_grace_secs);
                if since.elapsed() > grace {
                    // Too late — expire the session.
                    session.state = SessionState::Expired;
                    return Err(SessionError::SessionExpired(player_id));
                }
                // Welcome back!
                session.state = SessionState::Connected;
                tracing::info!(%player_id, "player reconnected");
                Ok(self.sessions.get(&player_id).expect("just modified"))
            }
            SessionState::Connected => {
                Err(SessionError::AlreadyConnected(player_id))
            }
            SessionState::Expired => {
                Err(SessionError::SessionExpired(player_id))
            }
        }
    }

    /// Scans all sessions and expires any that have exceeded the grace period.
    ///
    /// Call this periodically (e.g., every few seconds) to clean up
    /// disconnected players who didn't reconnect in time.
    ///
    /// Returns the list of player IDs that were expired.
    pub fn expire_stale(&mut self) -> Vec<PlayerId> {
        let grace = Duration::from_secs(self.config.reconnect_grace_secs);
        let mut expired = Vec::new();

        for session in self.sessions.values_mut() {
            if let SessionState::Disconnected { since } = &session.state {
                if since.elapsed() > grace {
                    session.state = SessionState::Expired;
                    expired.push(session.player_id);
                    tracing::info!(
                        player_id = %session.player_id,
                        "session expired (grace period elapsed)"
                    );
                }
            }
        }

        expired
    }

    /// Removes all expired sessions, freeing memory.
    ///
    /// Call this after `expire_stale()` to actually remove the dead
    /// sessions from the maps. We separate expiring from cleanup so
    /// that higher layers can react to expirations (e.g., notify the
    /// room that a player is gone for good) before the data is deleted.
    pub fn cleanup_expired(&mut self) {
        // `retain` keeps only entries where the closure returns `true`.
        // It's like `filter` but modifies the map in place.
        self.sessions.retain(|_, session| {
            if matches!(session.state, SessionState::Expired) {
                self.tokens.remove(&session.reconnect_token);
                false // remove this entry
            } else {
                true // keep this entry
            }
        });
    }

    /// Looks up a session by player ID.
    ///
    /// Returns `None` if no session exists for this player.
    pub fn get(&self, player_id: &PlayerId) -> Option<&Session> {
        self.sessions.get(player_id)
    }

    /// Returns the number of active sessions (any state).
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Returns `true` if there are no sessions.
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }
}

/// Generates a random 32-character hex string (128 bits of entropy).
///
/// This is used as a reconnection token — a secret that only the server
/// and the specific client know. 128 bits is enough that guessing a valid
/// token is computationally infeasible (2^128 possibilities).
///
/// `fn` (not `pub fn`) means this is private to this module — it's an
/// implementation detail, not part of the public API.
fn generate_token() -> String {
    let mut rng = rand::rng();
    // Generate 16 random bytes (128 bits), then format each byte as
    // two hex characters. `{:02x}` means: lowercase hex, zero-padded
    // to 2 digits. So byte 0x0A becomes "0a", byte 0xFF becomes "ff".
    let bytes: [u8; 16] = rng.random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    //! Unit tests for `SessionManager`.
    //!
    //! These tests follow the naming convention from the coding standards:
    //!   `test_{function}_{scenario}_{expected}`
    //!
    //! We test the full session lifecycle state machine:
    //!   Connected → Disconnected → Reconnected (or Expired → Cleaned up)
    //!
    //! # Testing time-dependent behavior
    //!
    //! Some operations depend on elapsed time (grace period expiration).
    //! Instead of using `std::thread::sleep` (which makes tests slow and
    //! flaky), we use two strategies:
    //!   - `reconnect_grace_secs: 0` → sessions expire immediately
    //!   - `reconnect_grace_secs: 3600` → sessions never expire during test
    //!
    //! This keeps tests fast and deterministic.

    use super::*;

    // -- Helpers ----------------------------------------------------------

    /// Creates a `SessionManager` where disconnected sessions expire
    /// immediately (0-second grace period). Useful for testing expiration.
    fn manager_with_instant_expiry() -> SessionManager {
        SessionManager::new(SessionConfig {
            reconnect_grace_secs: 0,
        })
    }

    /// Creates a `SessionManager` where disconnected sessions effectively
    /// never expire (1-hour grace period). Useful for testing reconnection.
    fn manager_with_long_grace() -> SessionManager {
        SessionManager::new(SessionConfig {
            reconnect_grace_secs: 3600,
        })
    }

    /// Shorthand for creating a `PlayerId`. Writing `pid(1)` is easier
    /// to read in tests than `PlayerId(1)`.
    fn pid(id: u64) -> PlayerId {
        PlayerId(id)
    }

    // =====================================================================
    // create()
    // =====================================================================

    #[test]
    fn test_create_new_player_returns_connected_session() {
        // The simplest case: create a session for a brand-new player.
        let mut mgr = manager_with_long_grace();

        let session = mgr.create(pid(1)).expect("should succeed");

        // The session should be in the Connected state.
        assert!(matches!(session.state, SessionState::Connected));
        // The player ID should match what we passed in.
        assert_eq!(session.player_id, pid(1));
        // A reconnection token should have been generated (32 hex chars).
        assert_eq!(session.reconnect_token.len(), 32);
    }

    #[test]
    fn test_create_multiple_players_each_gets_unique_token() {
        // Each player should get a different reconnection token.
        // If tokens collided, reconnection would break.
        let mut mgr = manager_with_long_grace();

        let s1 = mgr.create(pid(1)).expect("should succeed");
        let token1 = s1.reconnect_token.clone();

        let s2 = mgr.create(pid(2)).expect("should succeed");
        let token2 = s2.reconnect_token.clone();

        assert_ne!(token1, token2, "tokens must be unique per player");
    }

    #[test]
    fn test_create_already_connected_returns_error() {
        // A player can only have ONE active session. Trying to create
        // a second one while the first is still Connected should fail.
        let mut mgr = manager_with_long_grace();
        mgr.create(pid(1)).expect("first create should succeed");

        let result = mgr.create(pid(1));

        assert!(
            matches!(result, Err(SessionError::AlreadyConnected(p)) if p == pid(1)),
            "should reject duplicate connected session"
        );
    }

    #[test]
    fn test_create_replaces_disconnected_session() {
        // If a player disconnected and then authenticates again (instead
        // of using their reconnect token), we should allow a fresh session.
        let mut mgr = manager_with_long_grace();
        mgr.create(pid(1)).unwrap();
        mgr.disconnect(pid(1)).unwrap();

        // Creating again should succeed because the old session is Disconnected.
        let session =
            mgr.create(pid(1)).expect("should replace disconnected session");
        assert!(matches!(session.state, SessionState::Connected));
    }

    #[test]
    fn test_create_replaces_expired_session() {
        // Same as above but for expired sessions.
        let mut mgr = manager_with_instant_expiry();
        mgr.create(pid(1)).unwrap();
        mgr.disconnect(pid(1)).unwrap();
        mgr.expire_stale(); // now it's Expired

        let session =
            mgr.create(pid(1)).expect("should replace expired session");
        assert!(matches!(session.state, SessionState::Connected));
    }

    // =====================================================================
    // disconnect()
    // =====================================================================

    #[test]
    fn test_disconnect_connected_player_becomes_disconnected() {
        let mut mgr = manager_with_long_grace();
        mgr.create(pid(1)).unwrap();

        mgr.disconnect(pid(1)).expect("should succeed");

        // Verify the state changed.
        let session = mgr.get(&pid(1)).expect("session should still exist");
        assert!(
            matches!(session.state, SessionState::Disconnected { .. }),
            "state should be Disconnected, got {:?}",
            session.state
        );
    }

    #[test]
    fn test_disconnect_unknown_player_returns_not_found() {
        // Can't disconnect someone who was never connected.
        let mut mgr = manager_with_long_grace();

        let result = mgr.disconnect(pid(99));

        assert!(
            matches!(result, Err(SessionError::NotFound(p)) if p == pid(99)),
            "should return NotFound for unknown player"
        );
    }

    #[test]
    fn test_disconnect_preserves_reconnect_token() {
        // The reconnect token should survive a disconnect — the player
        // needs it to reconnect!
        let mut mgr = manager_with_long_grace();
        let token = mgr.create(pid(1)).unwrap().reconnect_token.clone();

        mgr.disconnect(pid(1)).unwrap();

        let session = mgr.get(&pid(1)).unwrap();
        assert_eq!(
            session.reconnect_token, token,
            "token should be preserved across disconnect"
        );
    }

    // =====================================================================
    // reconnect()
    // =====================================================================

    #[test]
    fn test_reconnect_valid_token_restores_connected() {
        // The happy path: player disconnects, then reconnects with
        // their token.
        let mut mgr = manager_with_long_grace();
        let token = mgr.create(pid(1)).unwrap().reconnect_token.clone();
        mgr.disconnect(pid(1)).unwrap();

        let session = mgr.reconnect(&token).expect("should succeed");

        assert!(matches!(session.state, SessionState::Connected));
        assert_eq!(session.player_id, pid(1));
    }

    #[test]
    fn test_reconnect_invalid_token_returns_error() {
        // A made-up token should be rejected.
        let mut mgr = manager_with_long_grace();
        mgr.create(pid(1)).unwrap();
        mgr.disconnect(pid(1)).unwrap();

        let result = mgr.reconnect("not-a-real-token");

        assert!(
            matches!(result, Err(SessionError::InvalidToken)),
            "should reject unknown token"
        );
    }

    #[test]
    fn test_reconnect_after_grace_period_returns_expired() {
        // With a 0-second grace period, the session expires immediately
        // after disconnect. Reconnecting should fail.
        let mut mgr = manager_with_instant_expiry();
        let token = mgr.create(pid(1)).unwrap().reconnect_token.clone();
        mgr.disconnect(pid(1)).unwrap();
        // Grace period is 0 seconds, so any elapsed time means expired.

        let result = mgr.reconnect(&token);

        assert!(
            matches!(result, Err(SessionError::SessionExpired(p)) if p == pid(1)),
            "should reject reconnection after grace period"
        );
    }

    #[test]
    fn test_reconnect_already_connected_returns_error() {
        // If the player is still Connected (never disconnected), trying
        // to "reconnect" doesn't make sense.
        let mut mgr = manager_with_long_grace();
        let token = mgr.create(pid(1)).unwrap().reconnect_token.clone();

        let result = mgr.reconnect(&token);

        assert!(
            matches!(result, Err(SessionError::AlreadyConnected(p)) if p == pid(1)),
            "should reject reconnect when already connected"
        );
    }

    // =====================================================================
    // expire_stale()
    // =====================================================================

    #[test]
    fn test_expire_stale_expires_timed_out_sessions() {
        // With 0-second grace, disconnected sessions expire immediately.
        let mut mgr = manager_with_instant_expiry();
        mgr.create(pid(1)).unwrap();
        mgr.create(pid(2)).unwrap();
        mgr.disconnect(pid(1)).unwrap();
        // Player 2 stays connected.

        let expired = mgr.expire_stale();

        // Only player 1 should be expired (they disconnected).
        assert_eq!(expired, vec![pid(1)]);
        // Player 2 should be unaffected.
        let s2 = mgr.get(&pid(2)).unwrap();
        assert!(matches!(s2.state, SessionState::Connected));
    }

    #[test]
    fn test_expire_stale_skips_sessions_within_grace() {
        // With a long grace period, nothing should expire.
        let mut mgr = manager_with_long_grace();
        mgr.create(pid(1)).unwrap();
        mgr.disconnect(pid(1)).unwrap();

        let expired = mgr.expire_stale();

        assert!(
            expired.is_empty(),
            "nothing should expire within grace period"
        );
    }

    #[test]
    fn test_expire_stale_returns_empty_when_no_sessions() {
        let mut mgr = manager_with_long_grace();

        let expired = mgr.expire_stale();

        assert!(expired.is_empty());
    }

    // =====================================================================
    // cleanup_expired()
    // =====================================================================

    #[test]
    fn test_cleanup_expired_removes_expired_sessions() {
        // Full lifecycle: create → disconnect → expire → cleanup.
        let mut mgr = manager_with_instant_expiry();
        mgr.create(pid(1)).unwrap();
        mgr.disconnect(pid(1)).unwrap();
        mgr.expire_stale();

        // Session still exists (expired but not cleaned up).
        assert_eq!(mgr.len(), 1);

        mgr.cleanup_expired();

        // Now it's gone.
        assert_eq!(mgr.len(), 0);
        assert!(mgr.get(&pid(1)).is_none(), "session should be removed");
    }

    #[test]
    fn test_cleanup_expired_preserves_active_sessions() {
        // Cleanup should only remove Expired sessions, not Connected.
        let mut mgr = manager_with_instant_expiry();
        mgr.create(pid(1)).unwrap();
        mgr.create(pid(2)).unwrap();
        mgr.disconnect(pid(1)).unwrap();
        mgr.expire_stale();
        // Player 1 is Expired, Player 2 is Connected.

        mgr.cleanup_expired();

        assert_eq!(mgr.len(), 1);
        assert!(
            mgr.get(&pid(1)).is_none(),
            "expired session should be gone"
        );
        assert!(
            mgr.get(&pid(2)).is_some(),
            "active session should remain"
        );
    }

    #[test]
    fn test_cleanup_expired_invalidates_old_token() {
        // After cleanup, the old reconnect token should no longer work.
        // This prevents someone from using a stale token after the
        // session has been fully removed.
        let mut mgr = manager_with_instant_expiry();
        let token = mgr.create(pid(1)).unwrap().reconnect_token.clone();
        mgr.disconnect(pid(1)).unwrap();
        mgr.expire_stale();
        mgr.cleanup_expired();

        let result = mgr.reconnect(&token);

        assert!(
            matches!(result, Err(SessionError::InvalidToken)),
            "old token should be invalid after cleanup"
        );
    }

    // =====================================================================
    // get() / len() / is_empty()
    // =====================================================================

    #[test]
    fn test_get_returns_none_for_unknown_player() {
        let mgr = manager_with_long_grace();

        assert!(mgr.get(&pid(99)).is_none());
    }

    #[test]
    fn test_len_tracks_session_count() {
        let mut mgr = manager_with_long_grace();
        assert_eq!(mgr.len(), 0);
        assert!(mgr.is_empty());

        mgr.create(pid(1)).unwrap();
        assert_eq!(mgr.len(), 1);
        assert!(!mgr.is_empty());

        mgr.create(pid(2)).unwrap();
        assert_eq!(mgr.len(), 2);
    }

    // =====================================================================
    // Full lifecycle integration
    // =====================================================================

    #[test]
    fn test_full_lifecycle_connect_disconnect_reconnect() {
        // Simulates a real scenario: player connects, WiFi drops,
        // they reconnect within the grace period.
        let mut mgr = manager_with_long_grace();

        // 1. Player authenticates and gets a session.
        let token = mgr.create(pid(1)).unwrap().reconnect_token.clone();
        assert!(matches!(
            mgr.get(&pid(1)).unwrap().state,
            SessionState::Connected
        ));

        // 2. Network drops — player disconnects.
        mgr.disconnect(pid(1)).unwrap();
        assert!(matches!(
            mgr.get(&pid(1)).unwrap().state,
            SessionState::Disconnected { .. }
        ));

        // 3. Player reconnects with their token.
        mgr.reconnect(&token).unwrap();
        assert!(matches!(
            mgr.get(&pid(1)).unwrap().state,
            SessionState::Connected
        ));
    }

    #[test]
    fn test_full_lifecycle_connect_disconnect_expire_cleanup() {
        // Simulates: player connects, disconnects, never comes back,
        // session expires and gets cleaned up.
        let mut mgr = manager_with_instant_expiry();

        // 1. Player connects.
        mgr.create(pid(1)).unwrap();

        // 2. Player disconnects.
        mgr.disconnect(pid(1)).unwrap();

        // 3. Grace period elapses (instant with 0s config).
        let expired = mgr.expire_stale();
        assert_eq!(expired, vec![pid(1)]);

        // 4. Cleanup removes the dead session.
        mgr.cleanup_expired();
        assert!(mgr.is_empty());
    }

    #[test]
    fn test_multiple_players_independent_lifecycles() {
        // Two players with independent session lifecycles shouldn't
        // interfere with each other.
        let mut mgr = manager_with_long_grace();

        let token1 = mgr.create(pid(1)).unwrap().reconnect_token.clone();
        let token2 = mgr.create(pid(2)).unwrap().reconnect_token.clone();

        // Player 1 disconnects and reconnects.
        mgr.disconnect(pid(1)).unwrap();
        mgr.reconnect(&token1).unwrap();

        // Player 2 should be completely unaffected.
        let s2 = mgr.get(&pid(2)).unwrap();
        assert!(matches!(s2.state, SessionState::Connected));

        // Player 2 can independently disconnect and reconnect.
        mgr.disconnect(pid(2)).unwrap();
        mgr.reconnect(&token2).unwrap();

        // Both players should be Connected.
        assert!(matches!(
            mgr.get(&pid(1)).unwrap().state,
            SessionState::Connected
        ));
        assert!(matches!(
            mgr.get(&pid(2)).unwrap().state,
            SessionState::Connected
        ));
    }
}
