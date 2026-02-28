//! Integration tests for the room system using a mock game.

use std::time::Duration;

use arcforge_protocol::{PlayerId, Recipient};
use arcforge_room::{GameLogic, RoomConfig, RoomManager, RoomState};
use serde::{Deserialize, Serialize};

// =========================================================================
// Mock game: a simple counter that finishes at a target value.
// =========================================================================

struct CounterGame;

#[derive(Clone, Default)]
struct CounterConfig {
    finish_at: u32,
}

#[derive(Clone, Serialize, Deserialize)]
struct CounterState {
    count: u32,
    target: u32,
}

#[derive(Clone, Serialize, Deserialize)]
struct Increment;

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
enum CounterEvent {
    Counted(u32),
    Finished,
}

impl GameLogic for CounterGame {
    type Config = CounterConfig;
    type State = CounterState;
    type ClientMessage = Increment;
    type ServerMessage = CounterEvent;

    fn init(config: &CounterConfig, _players: &[PlayerId]) -> CounterState {
        CounterState {
            count: 0,
            target: config.finish_at,
        }
    }

    fn handle_message(
        state: &mut CounterState,
        _sender: PlayerId,
        _msg: Increment,
    ) -> Vec<(Recipient, CounterEvent)> {
        state.count += 1;
        if state.count >= state.target {
            vec![(Recipient::All, CounterEvent::Finished)]
        } else {
            vec![(Recipient::All, CounterEvent::Counted(state.count))]
        }
    }

    fn is_finished(state: &CounterState) -> bool {
        state.count >= state.target
    }

    fn room_config() -> RoomConfig {
        RoomConfig {
            min_players: 2,
            max_players: 4,
            ..RoomConfig::default()
        }
    }
}

/// A variant with min_players == max_players for testing the "full" path.
struct FullGame;

impl GameLogic for FullGame {
    type Config = CounterConfig;
    type State = CounterState;
    type ClientMessage = Increment;
    type ServerMessage = CounterEvent;

    fn init(config: &CounterConfig, _players: &[PlayerId]) -> CounterState {
        CounterState { count: 0, target: config.finish_at }
    }

    fn handle_message(
        state: &mut CounterState,
        _sender: PlayerId,
        _msg: Increment,
    ) -> Vec<(Recipient, CounterEvent)> {
        state.count += 1;
        vec![]
    }

    fn is_finished(state: &CounterState) -> bool {
        state.count >= state.target
    }

    fn room_config() -> RoomConfig {
        RoomConfig {
            min_players: 4,
            max_players: 4,
            ..RoomConfig::default()
        }
    }
}

// =========================================================================
// Helper
// =========================================================================

fn pid(id: u64) -> PlayerId {
    PlayerId(id)
}

// =========================================================================
// RoomManager tests
// =========================================================================

#[tokio::test]
async fn test_create_room_returns_unique_ids() {
    let mut mgr = RoomManager::<CounterGame>::new();
    let r1 = mgr.create_room(CounterConfig::default());
    let r2 = mgr.create_room(CounterConfig::default());
    assert_ne!(r1, r2);
    assert_eq!(mgr.room_count(), 2);
}

#[tokio::test]
async fn test_join_room_success() {
    let mut mgr = RoomManager::<CounterGame>::new();
    let room = mgr.create_room(CounterConfig::default());

    mgr.join_room(pid(1), room).await.unwrap();

    assert_eq!(mgr.player_room(&pid(1)), Some(room));
}

#[tokio::test]
async fn test_join_room_not_found() {
    let mut mgr = RoomManager::<CounterGame>::new();
    let result = mgr.join_room(pid(1), arcforge_protocol::RoomId(999)).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_join_room_one_room_at_a_time() {
    let mut mgr = RoomManager::<CounterGame>::new();
    let r1 = mgr.create_room(CounterConfig::default());
    let r2 = mgr.create_room(CounterConfig::default());

    mgr.join_room(pid(1), r1).await.unwrap();
    let result = mgr.join_room(pid(1), r2).await;
    assert!(result.is_err(), "player should not join two rooms");
}

#[tokio::test]
async fn test_join_room_already_in_same_room() {
    let mut mgr = RoomManager::<CounterGame>::new();
    let room = mgr.create_room(CounterConfig::default());

    mgr.join_room(pid(1), room).await.unwrap();
    let result = mgr.join_room(pid(1), room).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_join_room_full() {
    let mut mgr = RoomManager::<CounterGame>::new();
    let room = mgr.create_room(CounterConfig::default());

    // min_players is 2, max is 4. After 2 join, game auto-starts
    // and no more joins are allowed (room is InProgress).
    mgr.join_room(pid(1), room).await.unwrap();
    mgr.join_room(pid(2), room).await.unwrap();

    // 3rd player can't join — game already started
    let result = mgr.join_room(pid(3), room).await;
    assert!(result.is_err(), "should not join a running game");
}

#[tokio::test]
async fn test_join_room_at_max_capacity() {
    // FullGame has min_players=4, max_players=4.
    // Fill all 4 slots, then try a 5th.
    let mut mgr = RoomManager::<FullGame>::new();
    let room = mgr.create_room(CounterConfig::default());

    for i in 1..=4 {
        mgr.join_room(pid(i), room).await.unwrap();
    }
    // Room is now full AND game started
    let result = mgr.join_room(pid(5), room).await;
    assert!(result.is_err(), "room should reject 5th player");
}

#[tokio::test]
async fn test_leave_room_success() {
    let mut mgr = RoomManager::<CounterGame>::new();
    let room = mgr.create_room(CounterConfig::default());
    mgr.join_room(pid(1), room).await.unwrap();

    mgr.leave_room(pid(1)).await.unwrap();

    assert_eq!(mgr.player_room(&pid(1)), None);
}

#[tokio::test]
async fn test_leave_room_not_in_any_room() {
    let mut mgr = RoomManager::<CounterGame>::new();
    let result = mgr.leave_room(pid(1)).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_get_room_info() {
    let mut mgr = RoomManager::<CounterGame>::new();
    let room = mgr.create_room(CounterConfig::default());
    mgr.join_room(pid(1), room).await.unwrap();

    let info = mgr.get_room_info(room).await.unwrap();

    assert_eq!(info.room_id, room);
    assert_eq!(info.player_count, 1);
    assert_eq!(info.max_players, 4);
    assert_eq!(info.state, RoomState::WaitingForPlayers);
}

#[tokio::test]
async fn test_auto_start_when_min_players_reached() {
    let mut mgr = RoomManager::<CounterGame>::new();
    let room = mgr.create_room(CounterConfig::default());

    mgr.join_room(pid(1), room).await.unwrap();
    let info = mgr.get_room_info(room).await.unwrap();
    assert_eq!(info.state, RoomState::WaitingForPlayers);

    // min_players is 2 — joining second player should auto-start
    mgr.join_room(pid(2), room).await.unwrap();
    let info = mgr.get_room_info(room).await.unwrap();
    assert_eq!(info.state, RoomState::InProgress);
}

#[tokio::test]
async fn test_cannot_join_after_game_started() {
    let mut mgr = RoomManager::<CounterGame>::new();
    let room = mgr.create_room(CounterConfig::default());
    mgr.join_room(pid(1), room).await.unwrap();
    mgr.join_room(pid(2), room).await.unwrap();
    // Game is now InProgress

    let result = mgr.join_room(pid(3), room).await;
    assert!(result.is_err(), "should not join a running game");
}

#[tokio::test]
async fn test_route_message() {
    let mut mgr = RoomManager::<CounterGame>::new();
    let room = mgr.create_room(CounterConfig { finish_at: 100 });
    mgr.join_room(pid(1), room).await.unwrap();
    mgr.join_room(pid(2), room).await.unwrap();

    // Game is InProgress, send a message
    mgr.route_message(pid(1), Increment).await.unwrap();

    // Give the actor a moment to process
    tokio::time::sleep(Duration::from_millis(10)).await;

    let info = mgr.get_room_info(room).await.unwrap();
    assert_eq!(info.state, RoomState::InProgress);
}

#[tokio::test]
async fn test_route_message_not_in_room() {
    let mgr = RoomManager::<CounterGame>::new();
    let result = mgr.route_message(pid(1), Increment).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_destroy_room() {
    let mut mgr = RoomManager::<CounterGame>::new();
    let room = mgr.create_room(CounterConfig::default());
    mgr.join_room(pid(1), room).await.unwrap();

    mgr.destroy_room(room).await.unwrap();

    assert_eq!(mgr.room_count(), 0);
    assert_eq!(mgr.player_room(&pid(1)), None);
}

#[tokio::test]
async fn test_destroy_room_not_found() {
    let mut mgr = RoomManager::<CounterGame>::new();
    let result = mgr.destroy_room(arcforge_protocol::RoomId(999)).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_room_ids() {
    let mut mgr = RoomManager::<CounterGame>::new();
    let r1 = mgr.create_room(CounterConfig::default());
    let r2 = mgr.create_room(CounterConfig::default());

    let mut ids = mgr.room_ids();
    ids.sort_by_key(|r| r.0);
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&r1));
    assert!(ids.contains(&r2));
}

#[tokio::test]
async fn test_game_finishes_on_target() {
    let mut mgr = RoomManager::<CounterGame>::new();
    let room = mgr.create_room(CounterConfig { finish_at: 2 });
    mgr.join_room(pid(1), room).await.unwrap();
    mgr.join_room(pid(2), room).await.unwrap();

    // Send 2 increments to reach the target
    mgr.route_message(pid(1), Increment).await.unwrap();
    mgr.route_message(pid(1), Increment).await.unwrap();

    tokio::time::sleep(Duration::from_millis(10)).await;

    let info = mgr.get_room_info(room).await.unwrap();
    assert_eq!(info.state, RoomState::Finished);
}
