//! # Arcforge
//!
//! Low-latency game backend framework for web games.
//!
//! Arcforge provides a server-authoritative architecture where game developers
//! implement a single [`GameLogic`](arcforge_room::GameLogic) trait and the
//! framework handles transport, sessions, rooms, and state synchronization.
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use arcforge::prelude::*;
//!
//! // Implement GameLogic for your game, then:
//! // let server = ArcforgeServer::builder()
//! //     .bind("0.0.0.0:8080")
//! //     .build::<MyGame>(my_auth)
//! //     .await?;
//! // server.run().await
//! ```

#![allow(async_fn_in_trait)]

mod error;
mod handler;
mod server;

pub use error::ArcforgeError;
pub use server::{ArcforgeServer, ArcforgeServerBuilder, PROTOCOL_VERSION};

/// Re-exports everything a game developer needs.
///
/// ```rust
/// use arcforge::prelude::*;
/// ```
///
/// This gives you access to all the key types without importing
/// from individual sub-crates.
pub mod prelude {
    // Meta-crate
    pub use crate::{
        ArcforgeError, ArcforgeServer, ArcforgeServerBuilder,
        PROTOCOL_VERSION,
    };

    // Protocol types
    pub use arcforge_protocol::{
        Channel, Codec, Envelope, JsonCodec, Payload, PlayerId,
        ProtocolError, Recipient, RoomId, SystemMessage,
    };

    // Session types
    pub use arcforge_session::{
        Authenticator, Session, SessionConfig, SessionError,
        SessionManager, SessionState,
    };

    // Room types
    pub use arcforge_room::{
        GameLogic, RoomConfig, RoomError, RoomHandle, RoomInfo,
        RoomManager, RoomState,
    };

    // Transport types
    pub use arcforge_transport::{
        Connection, ConnectionId, Transport, TransportError,
        WebSocketTransport,
    };
}
