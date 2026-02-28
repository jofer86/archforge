//! Integration tests for the Arcforge server, handler, and full connection flow.

use std::time::Duration;

use arcforge::prelude::*;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio_tungstenite::tungstenite::Message;

// =========================================================================
// Mock game and authenticator
// =========================================================================

struct EchoGame;

#[derive(Clone, Default, Serialize, Deserialize)]
struct EchoState {
    messages: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct EchoMsg {
    text: String,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
struct EchoReply {
    from: u64,
    text: String,
}

impl GameLogic for EchoGame {
    type Config = ();
    type State = EchoState;
    type ClientMessage = EchoMsg;
    type ServerMessage = EchoReply;

    fn init(_config: &(), _players: &[PlayerId]) -> EchoState {
        EchoState::default()
    }

    fn handle_message(
        state: &mut EchoState,
        sender: PlayerId,
        msg: EchoMsg,
    ) -> Vec<(Recipient, EchoReply)> {
        state.messages.push(msg.text.clone());
        vec![(
            Recipient::All,
            EchoReply {
                from: sender.0,
                text: msg.text,
            },
        )]
    }

    fn is_finished(state: &EchoState) -> bool {
        state.messages.len() >= 100
    }

    fn room_config() -> RoomConfig {
        RoomConfig {
            min_players: 2,
            max_players: 4,
            ..RoomConfig::default()
        }
    }
}

/// Accepts any numeric token as a PlayerId.
struct TestAuth;

impl Authenticator for TestAuth {
    async fn authenticate(
        &self,
        token: &str,
    ) -> Result<PlayerId, SessionError> {
        let id: u64 = token
            .parse()
            .map_err(|_| SessionError::AuthFailed("not a number".into()))?;
        Ok(PlayerId(id))
    }
}

// =========================================================================
// Helpers
// =========================================================================

type ClientWs = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

/// Starts a server on a random port and returns the address.
async fn start_server() -> String {
    let server = ArcforgeServerBuilder::new()
        .bind("127.0.0.1:0")
        .build::<EchoGame>(TestAuth)
        .await
        .expect("server should build");

    let addr = server
        .local_addr()
        .expect("should have local addr")
        .to_string();

    tokio::spawn(async move {
        let _ = server.run().await;
    });

    // Give the accept loop a moment to start.
    tokio::time::sleep(Duration::from_millis(10)).await;
    addr
}

async fn connect(addr: &str) -> ClientWs {
    let (ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
        .await
        .expect("should connect");
    ws
}

fn encode_envelope(envelope: &Envelope) -> Message {
    let bytes = serde_json::to_vec(envelope).expect("encode");
    Message::Binary(bytes.into())
}

fn decode_envelope(msg: Message) -> Envelope {
    serde_json::from_slice(&msg.into_data()).expect("decode")
}

/// Sends a handshake and returns the HandshakeAck envelope.
async fn handshake(ws: &mut ClientWs, player_id: u64) -> Envelope {
    let hs = Envelope {
        seq: 0,
        timestamp: 0,
        channel: Channel::ReliableOrdered,
        payload: Payload::System(SystemMessage::Handshake {
            version: PROTOCOL_VERSION,
            token: Some(player_id.to_string()),
        }),
    };
    ws.send(encode_envelope(&hs)).await.expect("send handshake");
    let msg = ws.next().await.unwrap().expect("recv ack");
    decode_envelope(msg)
}

// =========================================================================
// Tests
// =========================================================================

#[tokio::test]
async fn test_handshake_success() {
    let addr = start_server().await;
    let mut ws = connect(&addr).await;

    let ack = handshake(&mut ws, 42).await;
    match ack.payload {
        Payload::System(SystemMessage::HandshakeAck {
            player_id,
            ..
        }) => {
            assert_eq!(player_id, PlayerId(42));
        }
        other => panic!("expected HandshakeAck, got {other:?}"),
    }
}

#[tokio::test]
async fn test_handshake_version_mismatch() {
    let addr = start_server().await;
    let mut ws = connect(&addr).await;

    let hs = Envelope {
        seq: 0,
        timestamp: 0,
        channel: Channel::ReliableOrdered,
        payload: Payload::System(SystemMessage::Handshake {
            version: 999,
            token: Some("1".into()),
        }),
    };
    ws.send(encode_envelope(&hs)).await.expect("send");

    let msg = ws.next().await.unwrap().expect("recv");
    let env = decode_envelope(msg);
    match env.payload {
        Payload::System(SystemMessage::Error { code, .. }) => {
            assert_eq!(code, 400);
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

#[tokio::test]
async fn test_handshake_auth_failure() {
    let addr = start_server().await;
    let mut ws = connect(&addr).await;

    let hs = Envelope {
        seq: 0,
        timestamp: 0,
        channel: Channel::ReliableOrdered,
        payload: Payload::System(SystemMessage::Handshake {
            version: PROTOCOL_VERSION,
            token: Some("not-a-number".into()),
        }),
    };
    ws.send(encode_envelope(&hs)).await.expect("send");

    let msg = ws.next().await.unwrap().expect("recv");
    let env = decode_envelope(msg);
    match env.payload {
        Payload::System(SystemMessage::Error { code, .. }) => {
            assert_eq!(code, 401);
        }
        other => panic!("expected Error 401, got {other:?}"),
    }
}

#[tokio::test]
async fn test_heartbeat_response() {
    let addr = start_server().await;
    let mut ws = connect(&addr).await;
    handshake(&mut ws, 1).await;

    let hb = Envelope {
        seq: 1,
        timestamp: 0,
        channel: Channel::ReliableOrdered,
        payload: Payload::System(SystemMessage::Heartbeat {
            client_time: 12345,
        }),
    };
    ws.send(encode_envelope(&hb)).await.expect("send");

    let msg = ws.next().await.unwrap().expect("recv");
    let env = decode_envelope(msg);
    match env.payload {
        Payload::System(SystemMessage::HeartbeatAck {
            client_time,
            ..
        }) => {
            assert_eq!(client_time, 12345);
            // server_time is millis since connection start; may be 0 if fast.
        }
        other => panic!("expected HeartbeatAck, got {other:?}"),
    }
}

#[tokio::test]
async fn test_disconnect_closes_connection() {
    let addr = start_server().await;
    let mut ws = connect(&addr).await;
    handshake(&mut ws, 1).await;

    let disc = Envelope {
        seq: 1,
        timestamp: 0,
        channel: Channel::ReliableOrdered,
        payload: Payload::System(SystemMessage::Disconnect {
            reason: "bye".into(),
        }),
    };
    ws.send(encode_envelope(&disc)).await.expect("send");

    // Server should close the connection after Disconnect.
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        ws.next(),
    )
    .await;

    match result {
        Ok(Some(Ok(Message::Close(_)))) | Ok(None) => {} // expected
        Ok(Some(Err(_))) => {}                           // also fine
        other => panic!("expected close, got {other:?}"),
    }
}

#[tokio::test]
async fn test_join_room_not_found() {
    let addr = start_server().await;
    let mut ws = connect(&addr).await;
    handshake(&mut ws, 1).await;

    let join = Envelope {
        seq: 1,
        timestamp: 0,
        channel: Channel::ReliableOrdered,
        payload: Payload::System(SystemMessage::JoinRoom {
            room_id: RoomId(999),
        }),
    };
    ws.send(encode_envelope(&join)).await.expect("send");

    let msg = ws.next().await.unwrap().expect("recv");
    let env = decode_envelope(msg);
    match env.payload {
        Payload::System(SystemMessage::Error { code, .. }) => {
            assert_eq!(code, 404);
        }
        other => panic!("expected Error 404, got {other:?}"),
    }
}

#[tokio::test]
async fn test_game_message_not_in_room() {
    let addr = start_server().await;
    let mut ws = connect(&addr).await;
    handshake(&mut ws, 1).await;

    // Send a game message without joining a room first.
    let game_data = serde_json::to_vec(&EchoMsg {
        text: "hello".into(),
    })
    .unwrap();
    let env = Envelope {
        seq: 1,
        timestamp: 0,
        channel: Channel::ReliableOrdered,
        payload: Payload::Game(game_data),
    };
    ws.send(encode_envelope(&env)).await.expect("send");

    let msg = ws.next().await.unwrap().expect("recv");
    let resp = decode_envelope(msg);
    match resp.payload {
        Payload::System(SystemMessage::Error { code, message }) => {
            assert_eq!(code, 400);
            assert!(message.contains("not in any room"));
        }
        other => panic!("expected Error 400, got {other:?}"),
    }
}

#[tokio::test]
async fn test_invalid_envelope_ignored() {
    let addr = start_server().await;
    let mut ws = connect(&addr).await;
    handshake(&mut ws, 1).await;

    // Send garbage data.
    ws.send(Message::Binary(b"not json".to_vec().into()))
        .await
        .expect("send");

    // Send a valid heartbeat — should still work (bad envelope was skipped).
    let hb = Envelope {
        seq: 1,
        timestamp: 0,
        channel: Channel::ReliableOrdered,
        payload: Payload::System(SystemMessage::Heartbeat {
            client_time: 999,
        }),
    };
    ws.send(encode_envelope(&hb)).await.expect("send");

    let msg = ws.next().await.unwrap().expect("recv");
    let env = decode_envelope(msg);
    assert!(matches!(
        env.payload,
        Payload::System(SystemMessage::HeartbeatAck { .. })
    ));
}

#[tokio::test]
async fn test_handshake_non_handshake_first_message() {
    let addr = start_server().await;
    let mut ws = connect(&addr).await;

    // Send a heartbeat as the first message (should be rejected).
    let hb = Envelope {
        seq: 0,
        timestamp: 0,
        channel: Channel::ReliableOrdered,
        payload: Payload::System(SystemMessage::Heartbeat {
            client_time: 0,
        }),
    };
    ws.send(encode_envelope(&hb)).await.expect("send");

    let msg = ws.next().await.unwrap().expect("recv");
    let env = decode_envelope(msg);
    match env.payload {
        Payload::System(SystemMessage::Error { code, .. }) => {
            assert_eq!(code, 400);
        }
        other => panic!("expected Error 400, got {other:?}"),
    }
}

#[tokio::test]
async fn test_multiple_connections_independent() {
    let addr = start_server().await;

    let mut ws1 = connect(&addr).await;
    let mut ws2 = connect(&addr).await;

    let ack1 = handshake(&mut ws1, 10).await;
    let ack2 = handshake(&mut ws2, 20).await;

    match (&ack1.payload, &ack2.payload) {
        (
            Payload::System(SystemMessage::HandshakeAck {
                player_id: p1, ..
            }),
            Payload::System(SystemMessage::HandshakeAck {
                player_id: p2, ..
            }),
        ) => {
            assert_eq!(*p1, PlayerId(10));
            assert_eq!(*p2, PlayerId(20));
        }
        _ => panic!("expected two HandshakeAcks"),
    }
}

#[tokio::test]
async fn test_list_rooms_empty_server() {
    let addr = start_server().await;
    let mut ws = connect(&addr).await;
    handshake(&mut ws, 1).await;

    let list_req = Envelope {
        seq: 1,
        timestamp: 0,
        channel: Channel::ReliableOrdered,
        payload: Payload::System(SystemMessage::ListRooms),
    };
    ws.send(encode_envelope(&list_req)).await.expect("send");

    let msg = ws.next().await.unwrap().expect("recv");
    let env = decode_envelope(msg);
    match env.payload {
        Payload::System(SystemMessage::RoomList { rooms }) => {
            assert!(rooms.is_empty());
        }
        other => panic!("expected RoomList, got {other:?}"),
    }
}

#[tokio::test]
async fn test_join_or_create_creates_room() {
    let addr = start_server().await;
    let mut ws = connect(&addr).await;
    handshake(&mut ws, 1).await;

    let joc = Envelope {
        seq: 1,
        timestamp: 0,
        channel: Channel::ReliableOrdered,
        payload: Payload::System(SystemMessage::JoinOrCreate {
            name: "test".into(),
            options: vec![],
        }),
    };
    ws.send(encode_envelope(&joc)).await.expect("send");

    let msg = ws.next().await.unwrap().expect("recv");
    let env = decode_envelope(msg);
    match env.payload {
        Payload::System(SystemMessage::RoomJoined { room_id, .. }) => {
            assert!(room_id.0 > 0);
        }
        other => panic!("expected RoomJoined, got {other:?}"),
    }
}

#[tokio::test]
async fn test_join_or_create_second_player_joins_existing() {
    let addr = start_server().await;

    // Player 1 creates a room via JoinOrCreate.
    let mut ws1 = connect(&addr).await;
    handshake(&mut ws1, 1).await;

    let joc = Envelope {
        seq: 1,
        timestamp: 0,
        channel: Channel::ReliableOrdered,
        payload: Payload::System(SystemMessage::JoinOrCreate {
            name: "test".into(),
            options: vec![],
        }),
    };
    ws1.send(encode_envelope(&joc)).await.expect("send");
    let msg1 = ws1.next().await.unwrap().expect("recv");
    let env1 = decode_envelope(msg1);
    let room_id_1 = match env1.payload {
        Payload::System(SystemMessage::RoomJoined { room_id, .. }) => {
            room_id
        }
        other => panic!("expected RoomJoined, got {other:?}"),
    };

    // Player 2 should join the same room.
    let mut ws2 = connect(&addr).await;
    handshake(&mut ws2, 2).await;
    ws2.send(encode_envelope(&joc)).await.expect("send");
    let msg2 = ws2.next().await.unwrap().expect("recv");
    let env2 = decode_envelope(msg2);
    match env2.payload {
        Payload::System(SystemMessage::RoomJoined { room_id, .. }) => {
            assert_eq!(room_id, room_id_1);
        }
        other => panic!("expected RoomJoined same room, got {other:?}"),
    }
}

#[tokio::test]
async fn test_list_rooms_after_join_or_create() {
    let addr = start_server().await;
    let mut ws = connect(&addr).await;
    handshake(&mut ws, 1).await;

    // Create a room first.
    let joc = Envelope {
        seq: 1,
        timestamp: 0,
        channel: Channel::ReliableOrdered,
        payload: Payload::System(SystemMessage::JoinOrCreate {
            name: "test".into(),
            options: vec![],
        }),
    };
    ws.send(encode_envelope(&joc)).await.expect("send");
    let _ = ws.next().await.unwrap().expect("recv RoomJoined");

    // Now list rooms from a second connection.
    let mut ws2 = connect(&addr).await;
    handshake(&mut ws2, 2).await;

    let list_req = Envelope {
        seq: 1,
        timestamp: 0,
        channel: Channel::ReliableOrdered,
        payload: Payload::System(SystemMessage::ListRooms),
    };
    ws2.send(encode_envelope(&list_req)).await.expect("send");

    let msg = ws2.next().await.unwrap().expect("recv");
    let env = decode_envelope(msg);
    match env.payload {
        Payload::System(SystemMessage::RoomList { rooms }) => {
            assert_eq!(rooms.len(), 1);
            assert_eq!(rooms[0].player_count, 1);
        }
        other => panic!("expected RoomList, got {other:?}"),
    }
}
