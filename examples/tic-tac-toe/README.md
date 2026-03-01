# Tic-Tac-Toe — Arcforge Demo

A two-player tic-tac-toe game demonstrating the Arcforge framework: WebSocket transport, session auth, room management, and turn-based game logic.

## Running the Demo

### 1. Start the server

```bash
cargo run -p tic-tac-toe
```

The server listens on `ws://localhost:8080`.

### 2. Open the client

Open `demo.html` in two browser tabs (or two different browsers).

- In the first tab, enter token `1` and click **Connect**.
- In the second tab, enter token `2` and click **Connect**.

The game starts automatically once both players join. Player 1 is **X**, Player 2 is **O**.

### 3. Play

Click a cell on your turn. The board updates in real time for both players.

## How It Works

The server implements the `GameLogic` trait in ~60 lines:

- `init` — creates an empty 3×3 board and assigns X/O
- `validate_message` — rejects invalid moves (wrong turn, occupied cell, out of bounds)
- `handle_message` — places the mark, checks for win/draw, broadcasts events
- `is_finished` — returns true when someone wins or the board is full

Authentication uses a simple numeric token (the player ID). In a real game you'd swap in your own `Authenticator` implementation.

## Project Structure

```
examples/tic-tac-toe/
├── src/main.rs   # Server: game logic + bootstrap
├── demo.html     # Browser client (vanilla JS + WebSocket)
├── Cargo.toml
└── README.md
```
