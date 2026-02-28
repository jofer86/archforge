//! Room lifecycle management for Arcforge.
//!
//! Each room runs as an isolated Tokio task (actor model) with its own
//! game state, player list, and optional tick loop.
//!
//! # Key types
//!
//! - [`GameLogic`] — the trait game developers implement
//! - [`RoomManager`] — creates/destroys rooms, routes players
//! - [`RoomHandle`] — send commands to a running room actor
//! - [`RoomState`] — lifecycle state machine
//! - [`RoomConfig`] — room settings (player limits, tick rate, etc.)

#![allow(async_fn_in_trait)]

mod config;
mod error;
mod logic;
mod manager;
mod room;

pub use config::{RoomConfig, RoomState};
pub use error::RoomError;
pub use logic::GameLogic;
pub use manager::RoomManager;
pub use room::{RoomHandle, RoomInfo, RoomOutbound, PlayerSender};
