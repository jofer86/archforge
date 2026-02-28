//! Room manager: creates, tracks, and routes players to rooms.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use arcforge_protocol::{PlayerId, RoomId};

use crate::{GameLogic, PlayerSender, RoomError, RoomHandle, RoomInfo};
use crate::room::spawn_room;

/// Counter for generating unique room IDs.
static NEXT_ROOM_ID: AtomicU64 = AtomicU64::new(1);

/// Default command channel size for room actors.
const DEFAULT_CHANNEL_SIZE: usize = 64;

/// Manages all active rooms and tracks which player is in which room.
///
/// This is the entry point for room operations from higher layers
/// (session layer, server accept loop).
pub struct RoomManager<G: GameLogic> {
    /// Active rooms, keyed by room ID.
    rooms: HashMap<RoomId, RoomHandle<G>>,

    /// Maps each player to the room they're currently in.
    /// A player can be in at most ONE room at a time (key invariant).
    player_rooms: HashMap<PlayerId, RoomId>,
}

impl<G: GameLogic> RoomManager<G> {
    /// Creates a new, empty room manager.
    pub fn new() -> Self {
        Self {
            rooms: HashMap::new(),
            player_rooms: HashMap::new(),
        }
    }

    /// Creates a new room and returns its ID.
    pub fn create_room(&mut self, game_config: G::Config) -> RoomId {
        let room_id =
            RoomId(NEXT_ROOM_ID.fetch_add(1, Ordering::Relaxed));
        let config = G::room_config();
        let handle = spawn_room::<G>(
            room_id,
            config,
            game_config,
            DEFAULT_CHANNEL_SIZE,
        );
        self.rooms.insert(room_id, handle);
        tracing::info!(%room_id, "room created");
        room_id
    }

    /// Adds a player to a room.
    ///
    /// Enforces the "one room at a time" invariant.
    pub async fn join_room(
        &mut self,
        player_id: PlayerId,
        room_id: RoomId,
        sender: PlayerSender<G>,
    ) -> Result<(), RoomError> {
        if let Some(current) = self.player_rooms.get(&player_id) {
            if *current == room_id {
                return Err(RoomError::AlreadyInRoom(player_id, room_id));
            }
            return Err(RoomError::InvalidState(format!(
                "player {} is already in room {}",
                player_id, current
            )));
        }

        let handle = self
            .rooms
            .get(&room_id)
            .ok_or(RoomError::NotFound(room_id))?;

        handle.join(player_id, sender).await?;
        self.player_rooms.insert(player_id, room_id);
        Ok(())
    }

    /// Removes a player from their current room.
    pub async fn leave_room(
        &mut self,
        player_id: PlayerId,
    ) -> Result<(), RoomError> {
        let room_id = self
            .player_rooms
            .get(&player_id)
            .copied()
            .ok_or(RoomError::InvalidState(format!(
                "player {} is not in any room",
                player_id
            )))?;

        if let Some(handle) = self.rooms.get(&room_id) {
            handle.leave(player_id).await?;
        }

        self.player_rooms.remove(&player_id);
        Ok(())
    }

    /// Routes a game message from a player to their current room.
    pub async fn route_message(
        &self,
        player_id: PlayerId,
        msg: G::ClientMessage,
    ) -> Result<(), RoomError> {
        let room_id = self
            .player_rooms
            .get(&player_id)
            .ok_or(RoomError::InvalidState(format!(
                "player {} is not in any room",
                player_id
            )))?;

        let handle = self
            .rooms
            .get(room_id)
            .ok_or(RoomError::NotFound(*room_id))?;

        handle.send_message(player_id, msg).await
    }

    /// Returns info about a specific room.
    pub async fn get_room_info(
        &self,
        room_id: RoomId,
    ) -> Result<RoomInfo, RoomError> {
        let handle = self
            .rooms
            .get(&room_id)
            .ok_or(RoomError::NotFound(room_id))?;
        handle.get_info().await
    }

    /// Shuts down a room and removes all its players from the index.
    pub async fn destroy_room(
        &mut self,
        room_id: RoomId,
    ) -> Result<(), RoomError> {
        let handle = self
            .rooms
            .remove(&room_id)
            .ok_or(RoomError::NotFound(room_id))?;

        let _ = handle.shutdown().await;

        // Remove all players that were in this room.
        self.player_rooms.retain(|_, rid| *rid != room_id);

        tracing::info!(%room_id, "room destroyed");
        Ok(())
    }

    /// Returns the room ID a player is currently in, if any.
    pub fn player_room(&self, player_id: &PlayerId) -> Option<RoomId> {
        self.player_rooms.get(player_id).copied()
    }

    /// Lists all rooms that are currently joinable.
    ///
    /// Queries each room actor for its current info. Rooms that fail
    /// to respond (e.g., shutting down) are silently skipped.
    pub async fn list_rooms(&self) -> Vec<RoomInfo> {
        let mut infos = Vec::with_capacity(self.rooms.len());
        for handle in self.rooms.values() {
            if let Ok(info) = handle.get_info().await {
                if info.state.is_joinable() {
                    infos.push(info);
                }
            }
        }
        infos
    }

    /// Returns cloned handles to all active rooms.
    ///
    /// Useful when callers need to perform async operations on rooms
    /// without holding the manager lock.
    pub fn room_handles(&self) -> Vec<RoomHandle<G>> {
        self.rooms.values().cloned().collect()
    }

    /// Finds a joinable room or creates a new one, then joins the player.
    ///
    /// This is the simple matchmaking for MVP: scan existing rooms for
    /// one that's still accepting players, join it. If none found, create
    /// a new room with the default game config and join that.
    pub async fn join_or_create(
        &mut self,
        player_id: PlayerId,
        game_config: G::Config,
        sender: PlayerSender<G>,
    ) -> Result<RoomId, RoomError> {
        // Check if player is already in a room.
        if let Some(existing) = self.player_rooms.get(&player_id) {
            return Err(RoomError::InvalidState(format!(
                "player {} is already in room {}",
                player_id, existing
            )));
        }

        // Try to find a joinable room.  If join() fails due to a race
        // (room filled between get_info and join), keep searching.
        for handle in self.rooms.values() {
            if let Ok(info) = handle.get_info().await {
                if info.state.is_joinable()
                    && info.player_count < info.max_players
                {
                    if let Ok(()) = handle.join(player_id, sender.clone()).await {
                        self.player_rooms.insert(player_id, info.room_id);
                        return Ok(info.room_id);
                    }
                }
            }
        }

        // No joinable room found — create one.
        let room_id = self.create_room(game_config);
        let handle = self
            .rooms
            .get(&room_id)
            .expect("just created this room");
        handle.join(player_id, sender).await?;
        self.player_rooms.insert(player_id, room_id);
        Ok(room_id)
    }

    /// Returns the number of active rooms.
    pub fn room_count(&self) -> usize {
        self.rooms.len()
    }

    /// Lists all active room IDs.
    pub fn room_ids(&self) -> Vec<RoomId> {
        self.rooms.keys().copied().collect()
    }
}

impl<G: GameLogic> Default for RoomManager<G> {
    fn default() -> Self {
        Self::new()
    }
}
