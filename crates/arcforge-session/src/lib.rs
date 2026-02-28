//! Player session management for Arcforge.
//!
//! This crate handles the lifecycle of player connections:
//!
//! 1. **Authentication** — validating who a player is ([`Authenticator`] trait)
//! 2. **Session tracking** — knowing who's connected ([`SessionManager`])
//! 3. **Reconnection** — letting players resume after brief disconnects
//!    (token-based, with configurable grace period)
//!
//! # How it fits in the stack
//!
//! ```text
//! Room Layer (above)  ← uses sessions to know which players are in which rooms
//!     ↕
//! Session Layer (this crate)  ← manages player identity and connection state
//!     ↕
//! Protocol Layer (below)  ← provides PlayerId, SystemMessage types
//! ```

#![allow(async_fn_in_trait)]

mod auth;
mod error;
mod manager;
mod session;

pub use auth::Authenticator;
pub use error::SessionError;
pub use manager::SessionManager;
pub use session::{Session, SessionConfig, SessionState};
