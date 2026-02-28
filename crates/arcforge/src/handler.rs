//! Per-connection handler: handshake, auth, and message routing.
//!
//! Each accepted connection gets its own Tokio task running this handler.
//! The flow is:
//!   1. Receive Handshake → validate version
//!   2. Authenticate token → get PlayerId
//!   3. Send HandshakeAck → player is connected
//!   4. Loop: receive envelopes → dispatch system or game messages

use std::sync::Arc;
use std::time::{Duration, Instant};

use arcforge_protocol::{
    Codec, Channel, Envelope, Payload, PlayerId, RoomListEntry,
    SystemMessage,
};
use arcforge_room::GameLogic;
use arcforge_session::Authenticator;
use arcforge_transport::{Connection, WebSocketConnection};

use crate::server::{ServerState, PROTOCOL_VERSION};
use crate::ArcforgeError;

/// Drop guard that disconnects a player's session when the handler exits.
///
/// This ensures cleanup happens even if the handler panics. Since `Drop`
/// is synchronous, we spawn a fire-and-forget task for the async lock.
struct SessionGuard<G: GameLogic, A: Authenticator, C: Codec> {
    player_id: PlayerId,
    state: Arc<ServerState<G, A, C>>,
}

impl<G: GameLogic, A: Authenticator, C: Codec> Drop
    for SessionGuard<G, A, C>
{
    fn drop(&mut self) {
        let player_id = self.player_id;
        let state = Arc::clone(&self.state);
        tokio::spawn(async move {
            let mut sessions = state.sessions.lock().await;
            let _ = sessions.disconnect(player_id);
        });
    }
}

/// Handles a single connection from accept to close.
pub(crate) async fn handle_connection<G, A, C>(
    conn: WebSocketConnection,
    state: Arc<ServerState<G, A, C>>,
) -> Result<(), ArcforgeError>
where
    G: GameLogic,
    A: Authenticator,
    C: Codec,
{
    let conn_id = conn.id();
    tracing::debug!(%conn_id, "handling new connection");

    // --- Step 1: Handshake ---
    let player_id = perform_handshake(&conn, &state).await?;

    tracing::info!(%conn_id, %player_id, "player authenticated");

    // Create session and guard atomically — if session creation fails,
    // no guard is needed. If it succeeds, the guard is immediately active.
    {
        let mut sessions = state.sessions.lock().await;
        sessions.create(player_id).map_err(ArcforgeError::Session)?;
    }
    let _guard = SessionGuard {
        player_id,
        state: Arc::clone(&state),
    };

    // --- Step 2: Message loop ---
    let mut seq: u64 = 1;
    let start = Instant::now();

    loop {
        let data = match tokio::time::timeout(
            Duration::from_secs(15),
            conn.recv(),
        )
        .await
        {
            Ok(Ok(Some(data))) => data,
            Ok(Ok(None)) => {
                tracing::info!(%player_id, "connection closed cleanly");
                break;
            }
            Ok(Err(e)) => {
                tracing::debug!(%player_id, error = %e, "recv error");
                break;
            }
            Err(_) => {
                tracing::info!(%player_id, "connection timed out");
                break;
            }
        };

        let envelope: Envelope = match state.codec.decode(&data) {
            Ok(env) => env,
            Err(e) => {
                tracing::debug!(
                    %player_id, error = %e, "failed to decode envelope"
                );
                continue;
            }
        };

        match envelope.payload {
            Payload::System(sys_msg) => {
                let should_close = handle_system_message(
                    &conn, &state, player_id, sys_msg, &mut seq, &start,
                )
                .await?;
                if should_close {
                    break;
                }
            }
            Payload::Game(game_data) => {
                handle_game_message::<G, A, C>(
                    &conn, &state, player_id, game_data, &mut seq, &start,
                )
                .await?;
            }
        }
    }

    // _guard drops here → session disconnect fires.
    Ok(())
}

/// Performs the initial handshake: receive Handshake, validate, auth, send Ack.
async fn perform_handshake<G, A, C>(
    conn: &WebSocketConnection,
    state: &Arc<ServerState<G, A, C>>,
) -> Result<PlayerId, ArcforgeError>
where
    G: GameLogic,
    A: Authenticator,
    C: Codec,
{
    let start = Instant::now();

    let data = match tokio::time::timeout(
        Duration::from_secs(5),
        conn.recv(),
    )
    .await
    {
        Ok(Ok(Some(data))) => data,
        Ok(Ok(None)) => {
            return Err(ArcforgeError::Protocol(
                arcforge_protocol::ProtocolError::InvalidMessage(
                    "connection closed before handshake".into(),
                ),
            ));
        }
        Ok(Err(e)) => return Err(ArcforgeError::Transport(e)),
        Err(_) => {
            return Err(ArcforgeError::Protocol(
                arcforge_protocol::ProtocolError::InvalidMessage(
                    "handshake timed out".into(),
                ),
            ));
        }
    };

    let envelope: Envelope = state.codec.decode(&data)?;

    let (version, token) = match envelope.payload {
        Payload::System(SystemMessage::Handshake { version, token }) => {
            (version, token)
        }
        _ => {
            send_error(conn, &state.codec, 400, "expected Handshake", 0, &start)
                .await?;
            return Err(ArcforgeError::Protocol(
                arcforge_protocol::ProtocolError::InvalidMessage(
                    "first message must be Handshake".into(),
                ),
            ));
        }
    };

    if version != PROTOCOL_VERSION {
        send_error(
            conn,
            &state.codec,
            400,
            &format!(
                "version mismatch: expected {PROTOCOL_VERSION}, got {version}"
            ),
            0,
            &start,
        )
        .await?;
        return Err(ArcforgeError::Protocol(
            arcforge_protocol::ProtocolError::InvalidMessage(
                "protocol version mismatch".into(),
            ),
        ));
    }

    let token_str = token.as_deref().unwrap_or("");
    let player_id = match state.auth.authenticate(token_str).await {
        Ok(pid) => pid,
        Err(e) => {
            send_error(conn, &state.codec, 401, "unauthorized", 0, &start)
                .await?;
            return Err(ArcforgeError::Session(e));
        }
    };

    let ack = Envelope {
        seq: 0,
        timestamp: start.elapsed().as_millis() as u64,
        channel: Channel::ReliableOrdered,
        payload: Payload::System(SystemMessage::HandshakeAck {
            player_id,
            server_time: start.elapsed().as_millis() as u64,
        }),
    };
    let ack_bytes = state.codec.encode(&ack)?;
    conn.send(&ack_bytes).await.map_err(ArcforgeError::Transport)?;

    Ok(player_id)
}

/// Handles a system message. Returns `true` if the connection should close.
async fn handle_system_message<G, A, C>(
    conn: &WebSocketConnection,
    state: &Arc<ServerState<G, A, C>>,
    player_id: PlayerId,
    msg: SystemMessage,
    seq: &mut u64,
    start: &Instant,
) -> Result<bool, ArcforgeError>
where
    G: GameLogic,
    A: Authenticator,
    C: Codec,
{
    match msg {
        SystemMessage::Heartbeat { client_time } => {
            let ack = Envelope {
                seq: next_seq(seq),
                timestamp: start.elapsed().as_millis() as u64,
                channel: Channel::ReliableOrdered,
                payload: Payload::System(SystemMessage::HeartbeatAck {
                    client_time,
                    server_time: start.elapsed().as_millis() as u64,
                }),
            };
            let bytes = state.codec.encode(&ack)?;
            conn.send(&bytes).await.map_err(ArcforgeError::Transport)?;
        }

        SystemMessage::JoinRoom { room_id } => {
            // Lock only for the join operation, drop before network I/O.
            let join_result = {
                let mut rooms = state.rooms.lock().await;
                rooms.join_room(player_id, room_id).await
            };

            match join_result {
                Ok(()) => {
                    let resp = Envelope {
                        seq: next_seq(seq),
                        timestamp: start.elapsed().as_millis() as u64,
                        channel: Channel::ReliableOrdered,
                        payload: Payload::System(
                            SystemMessage::RoomJoined {
                                room_id,
                                // TODO: populate with reconnection token
                                session_id: String::new(),
                            },
                        ),
                    };
                    let bytes = state.codec.encode(&resp)?;
                    conn.send(&bytes)
                        .await
                        .map_err(ArcforgeError::Transport)?;
                }
                Err(e) => {
                    send_error(
                        conn,
                        &state.codec,
                        404,
                        &e.to_string(),
                        next_seq(seq),
                        start,
                    )
                    .await?;
                }
            }
        }

        SystemMessage::JoinOrCreate { .. } => {
            // MVP: `name` and `options` are ignored — single game type,
            // default config. Phase 2 will use these for multi-game servers.
            let result = {
                let mut rooms = state.rooms.lock().await;
                rooms
                    .join_or_create(player_id, G::Config::default())
                    .await
            };

            match result {
                Ok(room_id) => {
                    let resp = Envelope {
                        seq: next_seq(seq),
                        timestamp: start.elapsed().as_millis() as u64,
                        channel: Channel::ReliableOrdered,
                        payload: Payload::System(
                            SystemMessage::RoomJoined {
                                room_id,
                                // TODO: populate with reconnection token
                                session_id: String::new(),
                            },
                        ),
                    };
                    let bytes = state.codec.encode(&resp)?;
                    conn.send(&bytes)
                        .await
                        .map_err(ArcforgeError::Transport)?;
                }
                Err(e) => {
                    send_error(
                        conn,
                        &state.codec,
                        409,
                        &e.to_string(),
                        next_seq(seq),
                        start,
                    )
                    .await?;
                }
            }
        }

        SystemMessage::ListRooms => {
            let handles = state.rooms.lock().await.room_handles();

            let mut entries = Vec::with_capacity(handles.len());
            for handle in &handles {
                if let Ok(info) = handle.get_info().await {
                    if info.state.is_joinable() {
                        entries.push(RoomListEntry {
                            room_id: info.room_id,
                            player_count: info.player_count,
                            max_players: info.max_players,
                        });
                    }
                }
            }

            let resp = Envelope {
                seq: next_seq(seq),
                timestamp: start.elapsed().as_millis() as u64,
                channel: Channel::ReliableOrdered,
                payload: Payload::System(SystemMessage::RoomList {
                    rooms: entries,
                }),
            };
            let bytes = state.codec.encode(&resp)?;
            conn.send(&bytes)
                .await
                .map_err(ArcforgeError::Transport)?;
        }

        SystemMessage::LeaveRoom => {
            let mut rooms = state.rooms.lock().await;
            if let Err(e) = rooms.leave_room(player_id).await {
                tracing::debug!(
                    %player_id, error = %e, "leave room failed"
                );
            }
        }

        SystemMessage::Disconnect { reason } => {
            tracing::info!(%player_id, %reason, "client disconnected");
            return Ok(true);
        }

        _ => {
            tracing::debug!(
                %player_id, "ignoring unexpected system message"
            );
        }
    }

    Ok(false)
}

/// Handles a game message: decode, route to the player's room.
async fn handle_game_message<G, A, C>(
    conn: &WebSocketConnection,
    state: &Arc<ServerState<G, A, C>>,
    player_id: PlayerId,
    game_data: Vec<u8>,
    seq: &mut u64,
    start: &Instant,
) -> Result<(), ArcforgeError>
where
    G: GameLogic,
    A: Authenticator,
    C: Codec,
{
    let client_msg: G::ClientMessage = match state.codec.decode(&game_data)
    {
        Ok(msg) => msg,
        Err(e) => {
            send_error(
                conn,
                &state.codec,
                400,
                &format!("invalid game message: {e}"),
                next_seq(seq),
                start,
            )
            .await?;
            return Ok(());
        }
    };

    // PERF: cache room handle per-connection to avoid global lock on
    // every game message. Acceptable for MVP (<100 CCU).
    let result = state
        .rooms
        .lock()
        .await
        .route_message(player_id, client_msg)
        .await;

    if let Err(e) = result {
        send_error(
            conn,
            &state.codec,
            400,
            &e.to_string(),
            next_seq(seq),
            start,
        )
        .await?;
    }

    Ok(())
}

/// Sends a SystemMessage::Error envelope to the client.
async fn send_error(
    conn: &WebSocketConnection,
    codec: &impl Codec,
    code: u16,
    message: &str,
    seq: u64,
    start: &Instant,
) -> Result<(), ArcforgeError> {
    let envelope = Envelope {
        seq,
        timestamp: start.elapsed().as_millis() as u64,
        channel: Channel::ReliableOrdered,
        payload: Payload::System(SystemMessage::Error {
            code,
            message: message.to_string(),
        }),
    };
    let bytes = codec.encode(&envelope)?;
    conn.send(&bytes).await.map_err(ArcforgeError::Transport)?;
    Ok(())
}

/// Increments and returns the next sequence number.
fn next_seq(seq: &mut u64) -> u64 {
    let current = *seq;
    *seq += 1;
    current
}
