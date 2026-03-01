use arcforge::prelude::*;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Game types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Cell { Empty, X, O }

#[derive(Clone, Serialize, Deserialize)]
pub struct State {
    board: [[Cell; 3]; 3],
    players: [PlayerId; 2],
    turn: usize, // index into players: 0 = X, 1 = O
    winner: Option<PlayerId>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Move { pub row: usize, pub col: usize }

#[derive(Clone, Serialize, Deserialize)]
pub enum Event {
    MoveMade { player: u64, row: usize, col: usize, mark: char },
    GameOver { winner: Option<u64>, reason: String },
}

// ---------------------------------------------------------------------------
// Game logic
// ---------------------------------------------------------------------------

struct TicTacToe;

impl GameLogic for TicTacToe {
    type Config = ();
    type State = State;
    type ClientMessage = Move;
    type ServerMessage = Event;

    fn init(_: &(), players: &[PlayerId]) -> State {
        State {
            board: [[Cell::Empty; 3]; 3],
            players: [players[0], players[1]],
            turn: 0,
            winner: None,
        }
    }

    fn validate_message(state: &State, sender: PlayerId, msg: &Move) -> Result<(), String> {
        if state.winner.is_some() {
            return Err("game is over".into());
        }
        if state.players[state.turn] != sender {
            return Err("not your turn".into());
        }
        if msg.row >= 3 || msg.col >= 3 {
            return Err("row and col must be 0-2".into());
        }
        if state.board[msg.row][msg.col] != Cell::Empty {
            return Err("cell is occupied".into());
        }
        Ok(())
    }

    fn handle_message(state: &mut State, sender: PlayerId, msg: Move) -> Vec<(Recipient, Event)> {
        let mark = if state.turn == 0 { Cell::X } else { Cell::O };
        state.board[msg.row][msg.col] = mark;

        let mark_char = if mark == Cell::X { 'X' } else { 'O' };
        let mut out = vec![(
            Recipient::All,
            Event::MoveMade { player: sender.0, row: msg.row, col: msg.col, mark: mark_char },
        )];

        if check_winner(&state.board, mark) {
            state.winner = Some(sender);
            out.push((Recipient::All, Event::GameOver {
                winner: Some(sender.0),
                reason: format!("{mark_char} wins!"),
            }));
        } else if board_full(&state.board) {
            state.winner = Some(PlayerId(0)); // sentinel for draw
            out.push((Recipient::All, Event::GameOver {
                winner: None,
                reason: "draw".into(),
            }));
        } else {
            state.turn = 1 - state.turn;
        }

        out
    }

    fn is_finished(state: &State) -> bool {
        state.winner.is_some()
    }

    fn room_config() -> RoomConfig {
        RoomConfig { min_players: 2, max_players: 2, ..RoomConfig::default() }
    }
}

fn check_winner(b: &[[Cell; 3]; 3], m: Cell) -> bool {
    (0..3).any(|i| (0..3).all(|j| b[i][j] == m))           // rows
    || (0..3).any(|j| (0..3).all(|i| b[i][j] == m))        // cols
    || (0..3).all(|i| b[i][i] == m)                         // diagonal
    || (0..3).all(|i| b[i][2 - i] == m)                     // anti-diagonal
}

fn board_full(b: &[[Cell; 3]; 3]) -> bool {
    b.iter().all(|row| row.iter().all(|c| *c != Cell::Empty))
}

// ---------------------------------------------------------------------------
// Server bootstrap
// ---------------------------------------------------------------------------

struct TokenAuth;

impl Authenticator for TokenAuth {
    async fn authenticate(&self, token: &str) -> Result<PlayerId, SessionError> {
        let id: u64 = token.parse()
            .map_err(|_| SessionError::AuthFailed("token must be a number".into()))?;
        Ok(PlayerId(id))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("starting tic-tac-toe server on 0.0.0.0:8080");

    let server = ArcforgeServerBuilder::new()
        .bind("0.0.0.0:8080")
        .build::<TicTacToe>(TokenAuth)
        .await?;

    server.run().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use std::time::Duration;
    use tokio_tungstenite::tungstenite::Message;

    type Ws = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    async fn start() -> String {
        let server = ArcforgeServerBuilder::new()
            .bind("127.0.0.1:0")
            .build::<TicTacToe>(TokenAuth)
            .await
            .unwrap();
        let addr = server.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            let _ = server.run().await;
        });
        tokio::time::sleep(Duration::from_millis(10)).await;
        addr
    }

    async fn ws(addr: &str) -> Ws {
        let (ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
            .await
            .unwrap();
        ws
    }

    fn enc(env: &Envelope) -> Message {
        Message::Binary(serde_json::to_vec(env).unwrap().into())
    }

    fn dec(msg: Message) -> Envelope {
        serde_json::from_slice(&msg.into_data()).unwrap()
    }

    async fn do_handshake(ws: &mut Ws, id: u64) {
        let env = Envelope {
            seq: 0, timestamp: 0, channel: Channel::ReliableOrdered,
            payload: Payload::System(SystemMessage::Handshake {
                version: PROTOCOL_VERSION, token: Some(id.to_string()),
            }),
        };
        ws.send(enc(&env)).await.unwrap();
        let _ = ws.next().await.unwrap().unwrap(); // HandshakeAck
    }

    async fn join(ws: &mut Ws) {
        let env = Envelope {
            seq: 1, timestamp: 0, channel: Channel::ReliableOrdered,
            payload: Payload::System(SystemMessage::JoinOrCreate {
                name: "ttt".into(), options: vec![],
            }),
        };
        ws.send(enc(&env)).await.unwrap();
        let _ = ws.next().await.unwrap().unwrap(); // RoomJoined
    }

    async fn send_move(ws: &mut Ws, row: usize, col: usize) {
        let data = serde_json::to_vec(&Move { row, col }).unwrap();
        let env = Envelope {
            seq: 0, timestamp: 0, channel: Channel::ReliableOrdered,
            payload: Payload::Game(data),
        };
        ws.send(enc(&env)).await.unwrap();
    }

    async fn recv(ws: &mut Ws) -> Envelope {
        let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await.expect("timeout").unwrap().unwrap();
        dec(msg)
    }

    fn game_payload(env: &Envelope) -> Event {
        match &env.payload {
            Payload::Game(data) => serde_json::from_slice(data).unwrap(),
            other => panic!("expected Game, got {other:?}"),
        }
    }

    /// Setup: 2 players connected, joined, RoomState drained.
    async fn setup_game(addr: &str) -> (Ws, Ws) {
        let mut p1 = ws(addr).await;
        let mut p2 = ws(addr).await;
        do_handshake(&mut p1, 1).await;
        do_handshake(&mut p2, 2).await;
        join(&mut p1).await;
        join(&mut p2).await;
        let _ = recv(&mut p1).await; // RoomState
        let _ = recv(&mut p2).await; // RoomState
        (p1, p2)
    }

    /// Send a move and drain the MoveMade broadcast from both players.
    /// Returns the Event received by the sender.
    async fn play(p1: &mut Ws, p2: &mut Ws, who: u8, row: usize, col: usize) -> Event {
        let (sender, other) = if who == 1 { (p1 as &mut Ws, p2 as &mut Ws) } else { (p2 as &mut Ws, p1 as &mut Ws) };
        send_move(sender, row, col).await;
        let e = game_payload(&recv(sender).await);
        let _ = recv(other).await; // other player gets same broadcast
        e
    }

    // Minimal test: one move, verify both players receive it.
    #[tokio::test]
    async fn test_single_move() {
        let addr = start().await;
        let (mut p1, mut p2) = setup_game(&addr).await;

        send_move(&mut p1, 0, 0).await;
        let e = game_payload(&recv(&mut p1).await);
        assert!(matches!(e, Event::MoveMade { mark: 'X', row: 0, col: 0, .. }));
        let _ = recv(&mut p2).await;
    }

    // ---------------------------------------------------------------
    // Full game: X wins with top row
    //  X | X | X
    //  O | O | .
    //  . | . | .
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_x_wins_top_row() {
        let addr = start().await;
        let (mut p1, mut p2) = setup_game(&addr).await;

        let e = play(&mut p1, &mut p2, 1, 0, 0).await;
        assert!(matches!(e, Event::MoveMade { mark: 'X', row: 0, col: 0, .. }));

        play(&mut p1, &mut p2, 2, 1, 0).await;
        play(&mut p1, &mut p2, 1, 0, 1).await;
        play(&mut p1, &mut p2, 2, 1, 1).await;

        // X plays (0,2) — winning move. Produces MoveMade + GameOver.
        send_move(&mut p1, 0, 2).await;
        let e1 = game_payload(&recv(&mut p1).await);
        assert!(matches!(e1, Event::MoveMade { mark: 'X', row: 0, col: 2, .. }));
        let e2 = game_payload(&recv(&mut p1).await);
        assert!(matches!(e2, Event::GameOver { winner: Some(1), .. }));

        // p2 gets both
        let _ = recv(&mut p2).await; // MoveMade
        let e3 = game_payload(&recv(&mut p2).await);
        assert!(matches!(e3, Event::GameOver { winner: Some(1), .. }));
    }

    // ---------------------------------------------------------------
    // Diagonal win
    //  X | O | .
    //  O | X | .
    //  . | . | X
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_diagonal_win() {
        let addr = start().await;
        let (mut p1, mut p2) = setup_game(&addr).await;

        play(&mut p1, &mut p2, 1, 0, 0).await; // X
        play(&mut p1, &mut p2, 2, 0, 1).await; // O
        play(&mut p1, &mut p2, 1, 1, 1).await; // X
        play(&mut p1, &mut p2, 2, 1, 0).await; // O

        // X plays (2,2) — wins on diagonal
        send_move(&mut p1, 2, 2).await;
        let _ = recv(&mut p1).await; // MoveMade
        let e = game_payload(&recv(&mut p1).await);
        assert!(matches!(e, Event::GameOver { winner: Some(1), .. }));
        let _ = recv(&mut p2).await;
        let _ = recv(&mut p2).await;
    }

    // ---------------------------------------------------------------
    // Draw game
    //  X | O | X
    //  X | O | X
    //  O | X | O
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_draw() {
        let addr = start().await;
        let (mut p1, mut p2) = setup_game(&addr).await;

        play(&mut p1, &mut p2, 1, 0, 0).await; // X
        play(&mut p1, &mut p2, 2, 0, 1).await; // O
        play(&mut p1, &mut p2, 1, 0, 2).await; // X
        play(&mut p1, &mut p2, 2, 1, 1).await; // O
        play(&mut p1, &mut p2, 1, 1, 0).await; // X
        play(&mut p1, &mut p2, 2, 2, 0).await; // O
        play(&mut p1, &mut p2, 1, 1, 2).await; // X
        play(&mut p1, &mut p2, 2, 2, 2).await; // O

        // X plays (2,1) — board full, draw. Produces MoveMade + GameOver.
        send_move(&mut p1, 2, 1).await;
        let _ = recv(&mut p1).await; // MoveMade
        let e = game_payload(&recv(&mut p1).await);
        assert!(matches!(e, Event::GameOver { winner: None, .. }));
        let _ = recv(&mut p2).await;
        let e2 = game_payload(&recv(&mut p2).await);
        assert!(matches!(e2, Event::GameOver { winner: None, .. }));
    }

    // ---------------------------------------------------------------
    // Wrong turn: O tries to go first, then X succeeds
    // (Room actor silently drops invalid moves — no error sent back.
    //  We verify by confirming X can still play after O's invalid attempt.)
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_wrong_turn_ignored() {
        let addr = start().await;
        let (mut p1, mut p2) = setup_game(&addr).await;

        // O tries to go first — silently dropped by room actor
        send_move(&mut p2, 0, 0).await;

        // X goes — should succeed (proving O's move was ignored)
        send_move(&mut p1, 0, 0).await;
        let e = game_payload(&recv(&mut p1).await);
        assert!(matches!(e, Event::MoveMade { mark: 'X', row: 0, col: 0, .. }));
        let _ = recv(&mut p2).await;
    }

    // ---------------------------------------------------------------
    // Unit tests for validate_message — deterministic, no network.
    // Tests occupied cell, out of bounds, game over, and wrong turn.
    // ---------------------------------------------------------------
    #[test]
    fn test_validate_rejects_out_of_bounds() {
        let state = TicTacToe::init(&(), &[PlayerId(1), PlayerId(2)]);
        let r = TicTacToe::validate_message(&state, PlayerId(1), &Move { row: 3, col: 0 });
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("0-2"));
    }

    #[test]
    fn test_validate_rejects_occupied_cell() {
        let mut state = TicTacToe::init(&(), &[PlayerId(1), PlayerId(2)]);
        TicTacToe::handle_message(&mut state, PlayerId(1), Move { row: 0, col: 0 });
        // Now it's O's turn, cell (0,0) is taken
        let r = TicTacToe::validate_message(&state, PlayerId(2), &Move { row: 0, col: 0 });
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("occupied"));
    }

    #[test]
    fn test_validate_rejects_wrong_turn() {
        let state = TicTacToe::init(&(), &[PlayerId(1), PlayerId(2)]);
        let r = TicTacToe::validate_message(&state, PlayerId(2), &Move { row: 0, col: 0 });
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("not your turn"));
    }

    #[test]
    fn test_validate_rejects_after_game_over() {
        let mut state = TicTacToe::init(&(), &[PlayerId(1), PlayerId(2)]);
        state.winner = Some(PlayerId(1));
        let r = TicTacToe::validate_message(&state, PlayerId(2), &Move { row: 1, col: 1 });
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("game is over"));
    }

    #[test]
    fn test_win_detection_all_lines() {
        // Rows
        for row in 0..3 {
            let mut b = [[Cell::Empty; 3]; 3];
            for col in 0..3 { b[row][col] = Cell::X; }
            assert!(check_winner(&b, Cell::X), "row {row}");
        }
        // Columns
        for col in 0..3 {
            let mut b = [[Cell::Empty; 3]; 3];
            for row in 0..3 { b[row][col] = Cell::O; }
            assert!(check_winner(&b, Cell::O), "col {col}");
        }
        // Diagonals
        let mut b = [[Cell::Empty; 3]; 3];
        for i in 0..3 { b[i][i] = Cell::X; }
        assert!(check_winner(&b, Cell::X), "main diagonal");

        let mut b = [[Cell::Empty; 3]; 3];
        for i in 0..3 { b[i][2-i] = Cell::O; }
        assert!(check_winner(&b, Cell::O), "anti-diagonal");
    }
}
