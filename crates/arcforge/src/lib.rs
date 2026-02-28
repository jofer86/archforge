//! # Arcforge
//!
//! Low-latency game backend framework for web games.
//!
//! Arcforge provides a server-authoritative architecture where game developers
//! implement a single [`GameLogic`] trait and the framework handles transport,
//! sessions, rooms, and state synchronization.
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use arcforge::prelude::*;
//!
//! // Implement GameLogic for your game, then:
//! // let server = ArcforgeServer::builder()
//! //     .bind("0.0.0.0:8080")
//! //     .game::<MyGame>()
//! //     .build()
//! //     .await?;
//! // server.run().await
//! ```

pub mod prelude {}