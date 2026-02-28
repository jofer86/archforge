//! Room actor: an isolated Tokio task that owns a game instance.
//!
//! Each room runs in its own task, communicating with the outside world
//! through an mpsc channel. This is the "actor model" — no shared
//! mutable state, just message passing.

use std::collections::HashSet;

use arcforge_protocol::{PlayerId, Recipient, RoomId};
use tokio::sync::{mpsc, oneshot};

use crate::{GameLogic, RoomConfig, RoomError, RoomState};

/// An outbound message from the room actor to a player's connection handler.
#[derive(Debug)]
pub enum RoomOutbound<G: GameLogic> {
    /// Full game state snapshot (sent on join).
    State(G::State),
    /// A game message from the game logic.
    Message(G::ServerMessage),
}

impl<G: GameLogic> Clone for RoomOutbound<G> {
    fn clone(&self) -> Self {
        match self {
            Self::State(s) => Self::State(s.clone()),
            Self::Message(m) => Self::Message(m.clone()),
        }
    }
}

/// Channel sender for delivering outbound messages to a player.
pub type PlayerSender<G> = mpsc::UnboundedSender<RoomOutbound<G>>;

/// Commands sent to a room actor through its channel.
///
/// Each variant represents an operation the outside world can request.
/// The `oneshot::Sender` in some variants is a "reply channel" — the
/// caller sends a command and waits for the response on that channel.
pub(crate) enum RoomCommand<G: GameLogic> {
    /// Add a player to the room.
    Join {
        player_id: PlayerId,
        sender: PlayerSender<G>,
        reply: oneshot::Sender<Result<(), RoomError>>,
    },

    /// Remove a player from the room.
    Leave {
        player_id: PlayerId,
        reply: oneshot::Sender<Result<(), RoomError>>,
    },

    /// Deliver a game message from a player.
    Message {
        sender: PlayerId,
        msg: G::ClientMessage,
    },

    /// Request the current room state.
    GetState {
        reply: oneshot::Sender<RoomInfo>,
    },

    /// Shut down the room.
    Shutdown,
}

/// A snapshot of room metadata (not the game state itself).
#[derive(Debug, Clone)]
pub struct RoomInfo {
    /// The room's unique ID.
    pub room_id: RoomId,
    /// Current lifecycle state.
    pub state: RoomState,
    /// Number of players currently in the room.
    pub player_count: usize,
    /// Maximum players allowed.
    pub max_players: usize,
}

/// Handle to a running room actor. Used to send commands to it.
///
/// This is cheap to clone — it's just an `mpsc::Sender` wrapper.
/// The `RoomManager` holds one of these per room.
#[derive(Clone)]
pub struct RoomHandle<G: GameLogic> {
    room_id: RoomId,
    sender: mpsc::Sender<RoomCommand<G>>,
}

impl<G: GameLogic> RoomHandle<G> {
    /// Returns the room's unique ID.
    pub fn room_id(&self) -> RoomId {
        self.room_id
    }

    /// Sends a join request to the room.
    pub async fn join(
        &self,
        player_id: PlayerId,
        sender: PlayerSender<G>,
    ) -> Result<(), RoomError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(RoomCommand::Join {
                player_id,
                sender,
                reply: reply_tx,
            })
            .await
            .map_err(|_| RoomError::Unavailable(self.room_id))?;
        reply_rx
            .await
            .map_err(|_| RoomError::Unavailable(self.room_id))?
    }

    /// Sends a leave request to the room.
    pub async fn leave(
        &self,
        player_id: PlayerId,
    ) -> Result<(), RoomError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(RoomCommand::Leave {
                player_id,
                reply: reply_tx,
            })
            .await
            .map_err(|_| RoomError::Unavailable(self.room_id))?;
        reply_rx
            .await
            .map_err(|_| RoomError::Unavailable(self.room_id))?
    }

    /// Sends a game message to the room (fire-and-forget).
    pub async fn send_message(
        &self,
        sender: PlayerId,
        msg: G::ClientMessage,
    ) -> Result<(), RoomError> {
        self.sender
            .send(RoomCommand::Message { sender, msg })
            .await
            .map_err(|_| RoomError::Unavailable(self.room_id))
    }

    /// Requests the current room info.
    pub async fn get_info(&self) -> Result<RoomInfo, RoomError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(RoomCommand::GetState { reply: reply_tx })
            .await
            .map_err(|_| RoomError::Unavailable(self.room_id))?;
        reply_rx
            .await
            .map_err(|_| RoomError::Unavailable(self.room_id))
    }

    /// Tells the room to shut down.
    pub async fn shutdown(&self) -> Result<(), RoomError> {
        self.sender
            .send(RoomCommand::Shutdown)
            .await
            .map_err(|_| RoomError::Unavailable(self.room_id))
    }
}

/// The internal room actor state. Runs inside a Tokio task.
struct RoomActor<G: GameLogic> {
    room_id: RoomId,
    state: RoomState,
    config: RoomConfig,
    players: HashSet<PlayerId>,
    /// Per-player outbound channels.
    senders: std::collections::HashMap<PlayerId, PlayerSender<G>>,
    game_state: Option<G::State>,
    game_config: G::Config,
    receiver: mpsc::Receiver<RoomCommand<G>>,
}

impl<G: GameLogic> RoomActor<G> {
    /// Runs the actor loop, processing commands until shutdown.
    async fn run(mut self) {
        tracing::info!(room_id = %self.room_id, "room actor started");

        while let Some(cmd) = self.receiver.recv().await {
            match cmd {
                RoomCommand::Join {
                    player_id,
                    sender,
                    reply,
                } => {
                    let result = self.handle_join(player_id, sender);
                    let _ = reply.send(result);
                }
                RoomCommand::Leave { player_id, reply } => {
                    let result = self.handle_leave(player_id);
                    let _ = reply.send(result);
                }
                RoomCommand::Message { sender, msg } => {
                    self.handle_message(sender, msg);
                }
                RoomCommand::GetState { reply } => {
                    let _ = reply.send(self.info());
                }
                RoomCommand::Shutdown => {
                    tracing::info!(room_id = %self.room_id, "room shutting down");
                    self.state = RoomState::Destroying;
                    break;
                }
            }
        }

        tracing::info!(room_id = %self.room_id, "room actor stopped");
    }

    fn handle_join(
        &mut self,
        player_id: PlayerId,
        sender: PlayerSender<G>,
    ) -> Result<(), RoomError> {
        if !self.state.is_joinable() {
            return Err(RoomError::InvalidState(format!(
                "cannot join room in state {}",
                self.state
            )));
        }
        if self.players.contains(&player_id) {
            return Err(RoomError::AlreadyInRoom(
                player_id,
                self.room_id,
            ));
        }
        if self.players.len() >= self.config.max_players {
            return Err(RoomError::RoomFull(self.room_id));
        }

        self.players.insert(player_id);
        self.senders.insert(player_id, sender);
        tracing::info!(
            room_id = %self.room_id,
            %player_id,
            players = self.players.len(),
            "player joined"
        );

        // Auto-start when minimum players reached.
        if self.players.len() >= self.config.min_players {
            self.transition_to_starting();
        }

        // NOTE: State snapshot on join is handled by transition_to_starting
        // (broadcasts to all players). For late-join/reconnection into an
        // already-running game (Phase 2), add a snapshot send here.

        Ok(())
    }

    fn handle_leave(
        &mut self,
        player_id: PlayerId,
    ) -> Result<(), RoomError> {
        if !self.players.remove(&player_id) {
            return Err(RoomError::NotInRoom(player_id, self.room_id));
        }
        self.senders.remove(&player_id);

        tracing::info!(
            room_id = %self.room_id,
            %player_id,
            players = self.players.len(),
            "player left"
        );

        // Notify game logic if game is active.
        if self.state.is_active() {
            if let Some(game_state) = &mut self.game_state {
                let msgs =
                    G::on_player_disconnect(game_state, player_id);
                self.dispatch(msgs);
            }
        }

        Ok(())
    }

    fn handle_message(
        &mut self,
        sender: PlayerId,
        msg: G::ClientMessage,
    ) {
        if !self.players.contains(&sender) {
            tracing::warn!(
                room_id = %self.room_id,
                %sender,
                "message from non-member, ignoring"
            );
            return;
        }

        let game_state = match &mut self.game_state {
            Some(s) => s,
            None => return,
        };

        if let Err(reason) = G::validate_message(game_state, sender, &msg)
        {
            tracing::debug!(
                room_id = %self.room_id,
                %sender,
                %reason,
                "message validation failed"
            );
            return;
        }

        let msgs = G::handle_message(game_state, sender, msg);
        let finished = G::is_finished(game_state);

        // Dispatch after releasing the mutable borrow on game_state.
        self.dispatch(msgs);

        if finished {
            self.state = RoomState::Finished;
            tracing::info!(room_id = %self.room_id, "game finished");
        }
    }

    fn transition_to_starting(&mut self) {
        self.state = RoomState::Starting;
        let players: Vec<PlayerId> = self.players.iter().copied().collect();
        self.game_state =
            Some(G::init(&self.game_config, &players));
        self.state = RoomState::InProgress;
        tracing::info!(
            room_id = %self.room_id,
            players = players.len(),
            "game started"
        );

        // Broadcast initial state to all players.
        if let Some(game_state) = &self.game_state {
            let msg = RoomOutbound::State(game_state.clone());
            for pid in &self.players {
                self.send_to(*pid, msg.clone());
            }
        }
    }

    /// Dispatches outbound messages to the correct recipients.
    fn dispatch(&self, msgs: Vec<(Recipient, G::ServerMessage)>) {
        for (recipient, msg) in msgs {
            let outbound = RoomOutbound::Message(msg);
            match recipient {
                Recipient::All => {
                    for pid in &self.players {
                        self.send_to(*pid, outbound.clone());
                    }
                }
                Recipient::Player(pid) => {
                    self.send_to(pid, outbound);
                }
                Recipient::AllExcept(excluded) => {
                    for pid in &self.players {
                        if *pid != excluded {
                            self.send_to(*pid, outbound.clone());
                        }
                    }
                }
            }
        }
    }

    /// Sends an outbound message to a single player. Silently drops
    /// if the receiver is gone (player disconnected).
    fn send_to(&self, player_id: PlayerId, msg: RoomOutbound<G>) {
        if let Some(sender) = self.senders.get(&player_id) {
            let _ = sender.send(msg);
        }
    }

    fn info(&self) -> RoomInfo {
        RoomInfo {
            room_id: self.room_id,
            state: self.state,
            player_count: self.players.len(),
            max_players: self.config.max_players,
        }
    }
}

/// Spawns a new room actor task and returns a handle to communicate with it.
///
/// `channel_size` controls backpressure — if the channel fills up,
/// senders will wait (bounded channel).
pub(crate) fn spawn_room<G: GameLogic>(
    room_id: RoomId,
    config: RoomConfig,
    game_config: G::Config,
    channel_size: usize,
) -> RoomHandle<G> {
    let (tx, rx) = mpsc::channel(channel_size);

    let actor = RoomActor::<G> {
        room_id,
        state: RoomState::WaitingForPlayers,
        config,
        players: HashSet::new(),
        senders: std::collections::HashMap::new(),
        game_state: None,
        game_config,
        receiver: rx,
    };

    tokio::spawn(actor.run());

    RoomHandle {
        room_id,
        sender: tx,
    }
}
