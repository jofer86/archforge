//! Room lifecycle management for Arcforge.
//!
//! Each room runs as an isolated Tokio task (actor model) with its own
//! game state, player list, and optional tick loop.