//! `ArcforgeServer` builder and server loop.
//!
//! This is the entry point for running an Arcforge game server. It ties
//! together all the layers: transport → protocol → session → room.

use std::sync::Arc;

use arcforge_protocol::{
    Codec, JsonCodec,
};
use arcforge_room::{GameLogic, RoomManager};
use arcforge_session::{Authenticator, SessionConfig, SessionManager};
use arcforge_transport::{Transport, WebSocketTransport};
use tokio::sync::Mutex;

use crate::handler::handle_connection;
use crate::ArcforgeError;

/// The current protocol version. Clients must send this in their
/// handshake or be rejected.
pub const PROTOCOL_VERSION: u32 = 1;

/// Shared server state passed to each connection handler task.
///
/// Wrapped in `Arc` so it can be cheaply cloned across tasks.
/// Interior mutability via `Mutex` where needed.
pub(crate) struct ServerState<G: GameLogic, A: Authenticator, C: Codec> {
    pub(crate) sessions: Mutex<SessionManager>,
    pub(crate) rooms: Mutex<RoomManager<G>>,
    pub(crate) auth: A,
    pub(crate) codec: C,
}

/// Builder for configuring and starting an Arcforge server.
///
/// # Example
///
/// ```rust,ignore
/// use arcforge::prelude::*;
///
/// let server = ArcforgeServer::builder()
///     .bind("0.0.0.0:8080")
///     .build::<MyGame>(my_auth)
///     .await?;
/// server.run().await
/// ```
pub struct ArcforgeServerBuilder {
    bind_addr: String,
    session_config: SessionConfig,
}

impl ArcforgeServerBuilder {
    /// Creates a new builder with default settings.
    pub fn new() -> Self {
        Self {
            bind_addr: "127.0.0.1:8080".to_string(),
            session_config: SessionConfig::default(),
        }
    }

    /// Sets the address to bind the server to.
    pub fn bind(mut self, addr: &str) -> Self {
        self.bind_addr = addr.to_string();
        self
    }

    /// Sets the session configuration.
    pub fn session_config(mut self, config: SessionConfig) -> Self {
        self.session_config = config;
        self
    }

    /// Builds and starts the server with the given authenticator.
    ///
    /// Uses `JsonCodec` and `WebSocketTransport` as defaults (MVP).
    pub async fn build<G: GameLogic>(
        self,
        auth: impl Authenticator,
    ) -> Result<ArcforgeServer<G, impl Authenticator, JsonCodec>, ArcforgeError>
    {
        let transport =
            WebSocketTransport::bind(&self.bind_addr).await?;

        let state = Arc::new(ServerState {
            sessions: Mutex::new(SessionManager::new(self.session_config)),
            rooms: Mutex::new(RoomManager::new()),
            auth,
            codec: JsonCodec,
        });

        Ok(ArcforgeServer { transport, state })
    }
}

impl Default for ArcforgeServerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// A running Arcforge game server.
///
/// Call [`run()`](Self::run) to start accepting connections.
pub struct ArcforgeServer<G: GameLogic, A: Authenticator, C: Codec> {
    transport: WebSocketTransport,
    state: Arc<ServerState<G, A, C>>,
}

impl<G, A, C> ArcforgeServer<G, A, C>
where
    G: GameLogic,
    A: Authenticator,
    C: Codec + Clone + 'static,
{
    /// Creates a new builder.
    pub fn builder() -> ArcforgeServerBuilder {
        ArcforgeServerBuilder::new()
    }

    /// Returns the local address the server is bound to.
    pub fn local_addr(&self) -> std::io::Result<std::net::SocketAddr> {
        self.transport.local_addr()
    }

    /// Runs the server accept loop.
    ///
    /// Accepts incoming connections, performs the handshake, and spawns
    /// a handler task for each connected player. Runs until the process
    /// is terminated.
    pub async fn run(mut self) -> Result<(), ArcforgeError> {
        tracing::info!("Arcforge server running");

        loop {
            match self.transport.accept().await {
                Ok(conn) => {
                    let state = Arc::clone(&self.state);
                    tokio::spawn(async move {
                        if let Err(e) =
                            handle_connection::<G, A, C>(conn, state).await
                        {
                            tracing::debug!(
                                error = %e,
                                "connection ended with error"
                            );
                        }
                    });
                }
                Err(e) => {
                    tracing::error!(error = %e, "accept failed");
                }
            }
        }
    }
}
