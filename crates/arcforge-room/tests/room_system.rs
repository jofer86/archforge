//! Integration tests for the room system using a mock game.

use std::time::Duration;

use arcforge_protocol::{PlayerId, Recipient};
use arcforge_room::{GameLogic, PlayerSender, RoomConfig, RoomManager, RoomState};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

// =========================================================================
// Mock game: a simple counter that finishes at a target value.
// =========================================================================

#[derive(Debug)]
struct CounterGame;

#[derive(Clone, Debug, Default)]
struct CounterConfig {
    finish_at: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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

/// Creates a dummy player sender (receiver is dropped immediately).
fn dummy_sender<G: GameLogic>() -> PlayerSender<G> {
    mpsc::unbounded_channel().0
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

    mgr.join_room(pid(1), room, dummy_sender()).await.unwrap();

    assert_eq!(mgr.player_room(&pid(1)), Some(room));
}

#[tokio::test]
async fn test_join_room_not_found() {
    let mut mgr = RoomManager::<CounterGame>::new();
    let result = mgr.join_room(pid(1), arcforge_protocol::RoomId(999), dummy_sender()).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_join_room_one_room_at_a_time() {
    let mut mgr = RoomManager::<CounterGame>::new();
    let r1 = mgr.create_room(CounterConfig::default());
    let r2 = mgr.create_room(CounterConfig::default());

    mgr.join_room(pid(1), r1, dummy_sender()).await.unwrap();
    let result = mgr.join_room(pid(1), r2, dummy_sender()).await;
    assert!(result.is_err(), "player should not join two rooms");
}

#[tokio::test]
async fn test_join_room_already_in_same_room() {
    let mut mgr = RoomManager::<CounterGame>::new();
    let room = mgr.create_room(CounterConfig::default());

    mgr.join_room(pid(1), room, dummy_sender()).await.unwrap();
    let result = mgr.join_room(pid(1), room, dummy_sender()).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_join_room_full() {
    let mut mgr = RoomManager::<CounterGame>::new();
    let room = mgr.create_room(CounterConfig::default());

    // min_players is 2, max is 4. After 2 join, game auto-starts
    // and no more joins are allowed (room is InProgress).
    mgr.join_room(pid(1), room, dummy_sender()).await.unwrap();
    mgr.join_room(pid(2), room, dummy_sender()).await.unwrap();

    // 3rd player can't join — game already started
    let result = mgr.join_room(pid(3), room, dummy_sender()).await;
    assert!(result.is_err(), "should not join a running game");
}

#[tokio::test]
async fn test_join_room_at_max_capacity() {
    // FullGame has min_players=4, max_players=4.
    // Fill all 4 slots, then try a 5th.
    let mut mgr = RoomManager::<FullGame>::new();
    let room = mgr.create_room(CounterConfig::default());

    for i in 1..=4 {
        mgr.join_room(pid(i), room, dummy_sender()).await.unwrap();
    }
    // Room is now full AND game started
    let result = mgr.join_room(pid(5), room, dummy_sender()).await;
    assert!(result.is_err(), "room should reject 5th player");
}

#[tokio::test]
async fn test_leave_room_success() {
    let mut mgr = RoomManager::<CounterGame>::new();
    let room = mgr.create_room(CounterConfig::default());
    mgr.join_room(pid(1), room, dummy_sender()).await.unwrap();

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
    mgr.join_room(pid(1), room, dummy_sender()).await.unwrap();

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

    mgr.join_room(pid(1), room, dummy_sender()).await.unwrap();
    let info = mgr.get_room_info(room).await.unwrap();
    assert_eq!(info.state, RoomState::WaitingForPlayers);

    // min_players is 2 — joining second player should auto-start
    mgr.join_room(pid(2), room, dummy_sender()).await.unwrap();
    let info = mgr.get_room_info(room).await.unwrap();
    assert_eq!(info.state, RoomState::InProgress);
}

#[tokio::test]
async fn test_cannot_join_after_game_started() {
    let mut mgr = RoomManager::<CounterGame>::new();
    let room = mgr.create_room(CounterConfig::default());
    mgr.join_room(pid(1), room, dummy_sender()).await.unwrap();
    mgr.join_room(pid(2), room, dummy_sender()).await.unwrap();
    // Game is now InProgress

    let result = mgr.join_room(pid(3), room, dummy_sender()).await;
    assert!(result.is_err(), "should not join a running game");
}

#[tokio::test]
async fn test_route_message() {
    let mut mgr = RoomManager::<CounterGame>::new();
    let room = mgr.create_room(CounterConfig { finish_at: 100 });
    mgr.join_room(pid(1), room, dummy_sender()).await.unwrap();
    mgr.join_room(pid(2), room, dummy_sender()).await.unwrap();

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
    mgr.join_room(pid(1), room, dummy_sender()).await.unwrap();

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
    mgr.join_room(pid(1), room, dummy_sender()).await.unwrap();
    mgr.join_room(pid(2), room, dummy_sender()).await.unwrap();

    // Send 2 increments to reach the target
    mgr.route_message(pid(1), Increment).await.unwrap();
    mgr.route_message(pid(1), Increment).await.unwrap();

    tokio::time::sleep(Duration::from_millis(10)).await;

    let info = mgr.get_room_info(room).await.unwrap();
    assert_eq!(info.state, RoomState::Finished);
}

#[tokio::test]
async fn test_list_rooms_empty() {
    let mgr = RoomManager::<CounterGame>::new();
    let rooms = mgr.list_rooms().await;
    assert!(rooms.is_empty());
}

#[tokio::test]
async fn test_list_rooms_returns_joinable_only() {
    let mut mgr = RoomManager::<CounterGame>::new();
    let r1 = mgr.create_room(CounterConfig::default());
    let r2 = mgr.create_room(CounterConfig::default());

    // r2 gets filled → starts → no longer joinable
    mgr.join_room(pid(10), r2, dummy_sender()).await.unwrap();
    mgr.join_room(pid(11), r2, dummy_sender()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;

    let rooms = mgr.list_rooms().await;
    assert_eq!(rooms.len(), 1);
    assert_eq!(rooms[0].room_id, r1);
}

#[tokio::test]
async fn test_join_or_create_creates_when_empty() {
    let mut mgr = RoomManager::<CounterGame>::new();
    let room_id = mgr
        .join_or_create(pid(1), CounterConfig::default(), dummy_sender())
        .await
        .unwrap();

    assert_eq!(mgr.room_count(), 1);
    assert_eq!(mgr.player_room(&pid(1)), Some(room_id));
}

#[tokio::test]
async fn test_join_or_create_joins_existing() {
    let mut mgr = RoomManager::<CounterGame>::new();
    let _r1 = mgr.create_room(CounterConfig::default());

    let room_id = mgr
        .join_or_create(pid(1), CounterConfig::default(), dummy_sender())
        .await
        .unwrap();

    // Should have joined the existing room, not created a new one.
    assert_eq!(mgr.room_count(), 1);
    assert_eq!(room_id, _r1);
}

#[tokio::test]
async fn test_join_or_create_already_in_room() {
    let mut mgr = RoomManager::<CounterGame>::new();
    mgr.join_or_create(pid(1), CounterConfig::default(), dummy_sender())
        .await
        .unwrap();

    let result = mgr
        .join_or_create(pid(1), CounterConfig::default(), dummy_sender())
        .await;
    assert!(result.is_err());
}

// =========================================================================
// State synchronization tests
// =========================================================================

#[tokio::test]
async fn test_state_broadcast_on_game_start() {
    use arcforge_room::RoomOutbound;

    let mut mgr = RoomManager::<CounterGame>::new();
    let room = mgr.create_room(CounterConfig { finish_at: 10 });

    let (tx1, mut rx1) = mpsc::unbounded_channel();
    let (tx2, mut rx2) = mpsc::unbounded_channel();

    mgr.join_room(pid(1), room, tx1).await.unwrap();
    mgr.join_room(pid(2), room, tx2).await.unwrap();

    // Game auto-starts at min_players=2. Both players should get state.
    tokio::time::sleep(Duration::from_millis(10)).await;

    let msg1 = rx1.try_recv().expect("player 1 should get state");
    let msg2 = rx2.try_recv().expect("player 2 should get state");

    assert!(matches!(msg1, RoomOutbound::State(_)));
    assert!(matches!(msg2, RoomOutbound::State(_)));
}

#[tokio::test]
async fn test_game_message_broadcast() {
    use arcforge_room::RoomOutbound;

    let mut mgr = RoomManager::<CounterGame>::new();
    let room = mgr.create_room(CounterConfig { finish_at: 10 });

    let (tx1, mut rx1) = mpsc::unbounded_channel();
    let (tx2, mut rx2) = mpsc::unbounded_channel();

    mgr.join_room(pid(1), room, tx1).await.unwrap();
    mgr.join_room(pid(2), room, tx2).await.unwrap();

    // Drain initial state messages.
    tokio::time::sleep(Duration::from_millis(10)).await;
    let _ = rx1.try_recv();
    let _ = rx2.try_recv();

    // Send a game message.
    mgr.route_message(pid(1), Increment).await.unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Both players should receive the game message (Recipient::All).
    let msg1 = rx1.try_recv().expect("player 1 should get message");
    let msg2 = rx2.try_recv().expect("player 2 should get message");

    match (msg1, msg2) {
        (
            RoomOutbound::Message(CounterEvent::Counted(1)),
            RoomOutbound::Message(CounterEvent::Counted(1)),
        ) => {}
        other => panic!("expected Counted(1) for both, got {other:?}"),
    }
}

#[tokio::test]
async fn test_leave_stops_receiving() {
    let mut mgr = RoomManager::<CounterGame>::new();
    let room = mgr.create_room(CounterConfig { finish_at: 10 });

    let (tx1, mut rx1) = mpsc::unbounded_channel();
    let (tx2, _rx2) = mpsc::unbounded_channel();

    mgr.join_room(pid(1), room, tx1).await.unwrap();
    mgr.join_room(pid(2), room, tx2).await.unwrap();

    // Drain initial state.
    tokio::time::sleep(Duration::from_millis(10)).await;
    while rx1.try_recv().is_ok() {}

    // Player 1 leaves.
    mgr.leave_room(pid(1)).await.unwrap();

    // Player 2 sends a message — player 1 should NOT receive it.
    mgr.route_message(pid(2), Increment).await.unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;

    assert!(rx1.try_recv().is_err());
}
