//! Core protocol types for Arcforge's wire format.
//!
//! This module defines every type that travels "on the wire" — meaning these
//! are the structures that get serialized to bytes, sent over the network,
//! and deserialized on the other side.
//!
//! Think of this as the "language" that the client and server speak.

// We import traits and macros from the `serde` crate. Serde is Rust's standard
// library for **ser**ializing and **de**serializing data. The two key traits:
//   - `Serialize`:   "I can be turned INTO bytes/JSON/etc."
//   - `Deserialize`: "I can be created FROM bytes/JSON/etc."
// The `derive` macro auto-generates these implementations for our types.
use serde::{Deserialize, Serialize};

// We also need `fmt` for implementing Display (human-readable printing).
use std::fmt;

// ---------------------------------------------------------------------------
// Identity types
// ---------------------------------------------------------------------------

/// A unique identifier for a player.
///
/// This is a "newtype wrapper" — a common Rust pattern where you wrap a
/// primitive type (here `u64`) in a named struct. Why bother?
///
/// 1. **Type safety**: You can't accidentally pass a `RoomId` where a
///    `PlayerId` is expected, even though both are `u64` underneath.
/// 2. **Readability**: Function signatures like `fn kick(player: PlayerId)`
///    are clearer than `fn kick(player: u64)`.
///
/// The `#[derive(...)]` attribute auto-generates trait implementations:
///   - `Debug`       → enables `{:?}` formatting for logging
///   - `Clone, Copy` → allows cheap duplication (it's just a u64)
///   - `PartialEq, Eq` → enables `==` comparison
///   - `Hash`        → enables use as a HashMap key
///   - `Serialize, Deserialize` → enables JSON/binary conversion
///
/// The `#[serde(transparent)]` attribute tells serde to serialize this as
/// just the inner `u64`, not as `{ "0": 42 }`. So a PlayerId(42) becomes
/// just `42` in JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlayerId(pub u64);

/// Display lets us use `{}` in format strings and logging.
/// `tracing::info!("player {} joined", player_id)` will print "player P-42 joined".
impl fmt::Display for PlayerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "P-{}", self.0)
    }
}

/// A unique identifier for a room (a game instance).
///
/// Same newtype pattern as `PlayerId`. A room is one instance of a game —
/// for example, one tic-tac-toe match between two players.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RoomId(pub u64);

impl fmt::Display for RoomId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "R-{}", self.0)
    }
}

// ---------------------------------------------------------------------------
// Recipient — who should receive a message?
// ---------------------------------------------------------------------------

/// Specifies who should receive a server message.
///
/// When game logic processes a player's action, it returns a list of
/// `(Recipient, ServerMessage)` pairs. This enum tells the framework
/// WHERE to deliver each message.
///
/// This is a Rust `enum` — but unlike enums in most languages (which are
/// just named integers), Rust enums can carry data in each variant.
/// This is called a "tagged union" or "sum type".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Recipient {
    /// Send to every player in the room.
    All,

    /// Send to one specific player.
    /// The `PlayerId` inside tells us which one.
    Player(PlayerId),

    /// Send to everyone EXCEPT the specified player.
    /// Useful for broadcasting "Player X moved" to everyone else.
    AllExcept(PlayerId),
}

// ---------------------------------------------------------------------------
// Channel — delivery guarantees
// ---------------------------------------------------------------------------

/// The delivery guarantee for a message.
///
/// Different types of game data need different delivery guarantees.
/// A chat message MUST arrive (reliable), but a position update that's
/// sent 60 times per second can afford to lose a few (unreliable).
///
/// `#[serde(rename_all = "PascalCase")]` makes the JSON representation
/// use PascalCase: `"ReliableOrdered"` instead of `"reliable_ordered"`.
/// This matches the wire protocol spec.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "PascalCase")]
pub enum Channel {
    /// Delivered in order, no loss. Like TCP.
    /// This is the default for most game messages.
    /// The `#[default]` attribute makes this the value returned by
    /// `Channel::default()`.
    #[default]
    ReliableOrdered,

    /// Delivered (no loss), but may arrive out of order.
    /// Good for non-critical reliable data like chat.
    ReliableUnordered,

    /// May be lost, may arrive out of order. Like UDP.
    /// Good for frequent updates (positions, animations) where the
    /// latest value matters more than every value.
    Unreliable,
}

// ---------------------------------------------------------------------------
// SystemMessage — framework-level messages
// ---------------------------------------------------------------------------

/// A summary of a room returned in room listings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomListEntry {
    /// The room's unique ID.
    pub room_id: RoomId,
    /// Number of players currently in the room.
    pub player_count: usize,
    /// Maximum players allowed.
    pub max_players: usize,
}

/// Messages used by the framework itself (not game-specific).
///
/// These handle the "plumbing": connecting, authenticating, joining rooms,
/// heartbeats (keep-alive pings), and errors. Game developers don't create
/// these — the framework does.
///
/// `#[serde(tag = "type")]` is a serde attribute that controls how this enum
/// is represented in JSON. Instead of:
///   `{ "Handshake": { "version": 1 } }`
/// it produces:
///   `{ "type": "Handshake", "version": 1 }`
/// This "internally tagged" format is cleaner and easier to work with in
/// JavaScript/TypeScript on the client side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SystemMessage {
    // -- Connection lifecycle --

    /// Client → Server: "Hello, I want to connect."
    /// `version` is the protocol version so the server can reject
    /// incompatible clients. `token` is an optional auth token.
    Handshake {
        version: u32,
        token: Option<String>,
    },

    /// Server → Client: "Welcome, you're connected."
    /// The server assigns a `player_id` and tells the client the
    /// current `server_time` so they can synchronize clocks.
    HandshakeAck {
        player_id: PlayerId,
        server_time: u64,
    },

    /// Either direction: "I'm disconnecting."
    /// Includes a human-readable reason for logging/debugging.
    Disconnect { reason: String },

    // -- Heartbeat (keep-alive) --

    /// Client → Server: "I'm still here."
    /// Sent every ~5 seconds. `client_time` is the client's local
    /// timestamp so the server can echo it back for RTT calculation.
    Heartbeat { client_time: u64 },

    /// Server → Client: "I see you, here's timing info."
    /// The client uses both timestamps to calculate:
    ///   RTT = now - client_time
    ///   clock_offset = server_time - (client_time + RTT/2)
    HeartbeatAck {
        client_time: u64,
        server_time: u64,
    },

    // -- Room management --

    /// Client → Server: "Put me in this specific room."
    JoinRoom { room_id: RoomId },

    /// Client → Server: "Find me a room or create a new one."
    /// `name` is the game/room type. `options` is opaque config data
    /// (serialized by the game's codec).
    JoinOrCreate {
        name: String,
        options: Vec<u8>,
    },

    /// Client → Server: "I'm leaving the room."
    LeaveRoom,

    /// Client → Server: "Show me available rooms."
    ListRooms,

    /// Server → Client: "Here are the available rooms."
    RoomList {
        rooms: Vec<RoomListEntry>,
    },

    /// Server → Client: "Here's the current game state."
    /// The `Vec<u8>` is the game state serialized by the codec.
    /// It's opaque to the protocol layer — only the game logic
    /// knows how to interpret these bytes.
    RoomState { data: Vec<u8> },

    /// Server → Client: "You've joined a room."
    RoomJoined {
        room_id: RoomId,
        session_id: String,
    },

    // -- Errors --

    /// Server → Client: "Something went wrong."
    /// `code` follows HTTP-style conventions (400 = bad request,
    /// 401 = unauthorized, 404 = not found, etc.).
    Error { code: u16, message: String },
}

// ---------------------------------------------------------------------------
// Payload — what's inside an envelope
// ---------------------------------------------------------------------------

/// The content of a message: either a system message or game data.
///
/// `#[serde(tag = "type", content = "data")]` produces "adjacently tagged"
/// JSON. For a system message:
///   `{ "type": "System", "data": { "type": "Heartbeat", "client_time": 123 } }`
/// For a game message:
///   `{ "type": "Game", "data": [104, 101, 108, 108, 111] }`
///
/// This two-level tagging lets the framework quickly check: "Is this a
/// system message I handle, or game data I pass through to game logic?"
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum Payload {
    /// A framework-level message (handshake, heartbeat, room management).
    System(SystemMessage),

    /// Game-specific data, opaque to the framework.
    /// These bytes are the game's `ClientMessage` or `ServerMessage`
    /// serialized by the codec. The framework just passes them through.
    Game(Vec<u8>),
}

// ---------------------------------------------------------------------------
// Envelope — the top-level wire format
// ---------------------------------------------------------------------------

/// The top-level message wrapper. Every message on the wire is an Envelope.
///
/// Think of it like a postal envelope: it has metadata on the outside
/// (sequence number, timestamp, delivery method) and the actual content
/// (payload) inside.
///
/// ```text
/// ┌─────────────────────────────────┐
/// │ seq: 42                         │  ← message ordering
/// │ timestamp: 15000                │  ← when it was sent
/// │ channel: ReliableOrdered        │  ← delivery guarantee
/// │ ┌─────────────────────────────┐ │
/// │ │ payload: Game([...bytes...]) │ │  ← the actual content
/// │ └─────────────────────────────┘ │
/// └─────────────────────────────────┘
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    /// Auto-incrementing sequence number.
    /// Each side (client and server) maintains their own counter.
    /// Used to detect missing or out-of-order messages.
    pub seq: u64,

    /// Milliseconds since the server started.
    /// Used for timing, lag compensation, and debugging.
    pub timestamp: u64,

    /// The delivery guarantee for this message.
    /// Defaults to `ReliableOrdered` if not specified (via `#[serde(default)]`).
    #[serde(default)]
    pub channel: Channel,

    /// The actual message content (system or game data).
    pub payload: Payload,
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    //! Tests for protocol types and their JSON serialization.
    //!
    //! The wire protocol spec defines exact JSON shapes. These tests
    //! verify that our serde attributes produce the correct format,
    //! because a mismatch means the client SDK can't parse our messages.

    use super::*;

    // =====================================================================
    // Identity types: PlayerId, RoomId
    // =====================================================================

    #[test]
    fn test_player_id_serializes_as_plain_number() {
        // `#[serde(transparent)]` means PlayerId(42) → `42`, not `{"0":42}`.
        // This matters because the client SDK expects a plain number.
        let json = serde_json::to_string(&PlayerId(42)).unwrap();
        assert_eq!(json, "42");
    }

    #[test]
    fn test_player_id_deserializes_from_plain_number() {
        let pid: PlayerId = serde_json::from_str("42").unwrap();
        assert_eq!(pid, PlayerId(42));
    }

    #[test]
    fn test_player_id_display() {
        assert_eq!(PlayerId(7).to_string(), "P-7");
    }

    #[test]
    fn test_room_id_serializes_as_plain_number() {
        let json = serde_json::to_string(&RoomId(99)).unwrap();
        assert_eq!(json, "99");
    }

    #[test]
    fn test_room_id_display() {
        assert_eq!(RoomId(3).to_string(), "R-3");
    }

    // =====================================================================
    // Channel
    // =====================================================================

    #[test]
    fn test_channel_default_is_reliable_ordered() {
        // The wire protocol spec says ReliableOrdered is the default.
        assert_eq!(Channel::default(), Channel::ReliableOrdered);
    }

    #[test]
    fn test_channel_serializes_as_pascal_case() {
        // `#[serde(rename_all = "PascalCase")]` produces "ReliableOrdered",
        // not "reliable_ordered" or "RELIABLE_ORDERED".
        let json = serde_json::to_string(&Channel::ReliableOrdered).unwrap();
        assert_eq!(json, "\"ReliableOrdered\"");

        let json = serde_json::to_string(&Channel::Unreliable).unwrap();
        assert_eq!(json, "\"Unreliable\"");
    }

    // =====================================================================
    // SystemMessage — one test per variant to verify JSON shape
    // =====================================================================

    #[test]
    fn test_system_message_handshake_json_format() {
        // `#[serde(tag = "type")]` produces internally tagged JSON:
        //   { "type": "Handshake", "version": 1, "token": "abc" }
        let msg = SystemMessage::Handshake {
            version: 1,
            token: Some("abc".into()),
        };
        let json: serde_json::Value = serde_json::to_value(&msg).unwrap();

        assert_eq!(json["type"], "Handshake");
        assert_eq!(json["version"], 1);
        assert_eq!(json["token"], "abc");
    }

    #[test]
    fn test_system_message_handshake_without_token() {
        // Token is optional — `None` becomes `null` in JSON.
        let msg = SystemMessage::Handshake {
            version: 1,
            token: None,
        };
        let json: serde_json::Value = serde_json::to_value(&msg).unwrap();

        assert_eq!(json["type"], "Handshake");
        assert!(json["token"].is_null());
    }

    #[test]
    fn test_system_message_handshake_ack_json_format() {
        let msg = SystemMessage::HandshakeAck {
            player_id: PlayerId(42),
            server_time: 15000,
        };
        let json: serde_json::Value = serde_json::to_value(&msg).unwrap();

        assert_eq!(json["type"], "HandshakeAck");
        assert_eq!(json["player_id"], 42);
        assert_eq!(json["server_time"], 15000);
    }

    #[test]
    fn test_system_message_heartbeat_round_trip() {
        let msg = SystemMessage::Heartbeat { client_time: 5000 };
        let bytes = serde_json::to_vec(&msg).unwrap();
        let decoded: SystemMessage = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_system_message_heartbeat_ack_round_trip() {
        let msg = SystemMessage::HeartbeatAck {
            client_time: 5000,
            server_time: 5002,
        };
        let bytes = serde_json::to_vec(&msg).unwrap();
        let decoded: SystemMessage = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_system_message_join_room_round_trip() {
        let msg = SystemMessage::JoinRoom {
            room_id: RoomId(10),
        };
        let bytes = serde_json::to_vec(&msg).unwrap();
        let decoded: SystemMessage = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_system_message_join_or_create_round_trip() {
        let msg = SystemMessage::JoinOrCreate {
            name: "battle".into(),
            options: vec![1, 2, 3],
        };
        let bytes = serde_json::to_vec(&msg).unwrap();
        let decoded: SystemMessage = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_system_message_leave_room_round_trip() {
        let msg = SystemMessage::LeaveRoom;
        let bytes = serde_json::to_vec(&msg).unwrap();
        let decoded: SystemMessage = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_system_message_room_state_round_trip() {
        let msg = SystemMessage::RoomState {
            data: vec![10, 20, 30],
        };
        let bytes = serde_json::to_vec(&msg).unwrap();
        let decoded: SystemMessage = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_system_message_room_joined_round_trip() {
        let msg = SystemMessage::RoomJoined {
            room_id: RoomId(5),
            session_id: "sess-abc".into(),
        };
        let bytes = serde_json::to_vec(&msg).unwrap();
        let decoded: SystemMessage = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_system_message_error_json_format() {
        let msg = SystemMessage::Error {
            code: 401,
            message: "Unauthorized".into(),
        };
        let json: serde_json::Value = serde_json::to_value(&msg).unwrap();

        assert_eq!(json["type"], "Error");
        assert_eq!(json["code"], 401);
        assert_eq!(json["message"], "Unauthorized");
    }

    #[test]
    fn test_system_message_disconnect_round_trip() {
        let msg = SystemMessage::Disconnect {
            reason: "server shutting down".into(),
        };
        let bytes = serde_json::to_vec(&msg).unwrap();
        let decoded: SystemMessage = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    // =====================================================================
    // Payload
    // =====================================================================

    #[test]
    fn test_payload_system_json_format() {
        // `#[serde(tag = "type", content = "data")]` produces:
        //   { "type": "System", "data": { ... } }
        let payload = Payload::System(SystemMessage::LeaveRoom);
        let json: serde_json::Value =
            serde_json::to_value(&payload).unwrap();

        assert_eq!(json["type"], "System");
        assert!(json["data"].is_object());
    }

    #[test]
    fn test_payload_game_json_format() {
        // Game payload wraps opaque bytes:
        //   { "type": "Game", "data": [1, 2, 3] }
        let payload = Payload::Game(vec![1, 2, 3]);
        let json: serde_json::Value =
            serde_json::to_value(&payload).unwrap();

        assert_eq!(json["type"], "Game");
        assert_eq!(json["data"], serde_json::json!([1, 2, 3]));
    }

    // =====================================================================
    // Envelope
    // =====================================================================

    #[test]
    fn test_envelope_round_trip() {
        let envelope = Envelope {
            seq: 42,
            timestamp: 15000,
            channel: Channel::Unreliable,
            payload: Payload::Game(vec![1, 2, 3]),
        };
        let bytes = serde_json::to_vec(&envelope).unwrap();
        let decoded: Envelope = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(envelope, decoded);
    }

    #[test]
    fn test_envelope_channel_defaults_when_missing() {
        // `#[serde(default)]` on the channel field means if the JSON
        // doesn't include "channel", it defaults to ReliableOrdered.
        // This is important for backward compatibility.
        let json = r#"{
            "seq": 1,
            "timestamp": 100,
            "payload": { "type": "Game", "data": [1] }
        }"#;
        let envelope: Envelope = serde_json::from_str(json).unwrap();
        assert_eq!(envelope.channel, Channel::ReliableOrdered);
    }

    // =====================================================================
    // Recipient
    // =====================================================================

    #[test]
    fn test_recipient_all_round_trip() {
        let r = Recipient::All;
        let bytes = serde_json::to_vec(&r).unwrap();
        let decoded: Recipient = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(r, decoded);
    }

    #[test]
    fn test_recipient_player_round_trip() {
        let r = Recipient::Player(PlayerId(7));
        let bytes = serde_json::to_vec(&r).unwrap();
        let decoded: Recipient = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(r, decoded);
    }

    #[test]
    fn test_recipient_all_except_round_trip() {
        let r = Recipient::AllExcept(PlayerId(3));
        let bytes = serde_json::to_vec(&r).unwrap();
        let decoded: Recipient = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(r, decoded);
    }

    // =====================================================================
    // Error cases — malformed input
    // =====================================================================

    #[test]
    fn test_system_message_list_rooms_round_trip() {
        let msg = SystemMessage::ListRooms;
        let bytes = serde_json::to_vec(&msg).unwrap();
        let decoded: SystemMessage = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_system_message_room_list_round_trip() {
        let msg = SystemMessage::RoomList {
            rooms: vec![
                RoomListEntry {
                    room_id: RoomId(1),
                    player_count: 2,
                    max_players: 4,
                },
                RoomListEntry {
                    room_id: RoomId(2),
                    player_count: 0,
                    max_players: 8,
                },
            ],
        };
        let bytes = serde_json::to_vec(&msg).unwrap();
        let decoded: SystemMessage = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_system_message_room_list_empty() {
        let msg = SystemMessage::RoomList { rooms: vec![] };
        let bytes = serde_json::to_vec(&msg).unwrap();
        let decoded: SystemMessage = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_decode_garbage_returns_error() {
        // Random bytes should fail to parse as an Envelope.
        let garbage = b"not json at all";
        let result: Result<Envelope, _> = serde_json::from_slice(garbage);
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_wrong_type_returns_error() {
        // Valid JSON but wrong shape — missing required fields.
        let wrong = r#"{"name": "hello"}"#;
        let result: Result<Envelope, _> = serde_json::from_str(wrong);
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_unknown_system_message_type_returns_error() {
        // A system message with an unknown "type" tag should fail.
        let unknown = r#"{"type": "FlyToMoon", "speed": 9000}"#;
        let result: Result<SystemMessage, _> = serde_json::from_str(unknown);
        assert!(result.is_err());
    }
}
