# Arcforge

Low-latency game backend framework for web games, built in Rust.

Game developers implement a single `GameLogic` trait — Arcforge handles transport, sessions, rooms, and state synchronization.

## Architecture

```
┌─────────────────────────────────────────┐
│  Game Logic Layer  (your code)          │  Implement GameLogic trait
├─────────────────────────────────────────┤
│  Room Layer                             │  Isolated game instances
├─────────────────────────────────────────┤
│  Session Layer                          │  Auth, identity, reconnection
├─────────────────────────────────────────┤
│  Protocol Layer                         │  Wire format, codecs
├─────────────────────────────────────────┤
│  Transport Layer                        │  WebSocket, WebTransport
└─────────────────────────────────────────┘
```

## Try It — Tic-Tac-Toe Demo

A working two-player game in ~60 lines of game logic.

### Prerequisites

- [Rust](https://rustup.rs/) 1.85+ (stable)

### Run

```bash
# Start the server
cargo run -p tic-tac-toe
```

Then open [`examples/tic-tac-toe/demo.html`](examples/tic-tac-toe/demo.html) in two browser tabs:

1. First tab — enter token `1`, click **Connect**
2. Second tab — enter token `2`, click **Connect**

The game starts when both players join. Click cells to play. See the [example README](examples/tic-tac-toe/README.md) for details.

## How It Works

Implement the `GameLogic` trait and Arcforge runs your game:

```rust
use arcforge::prelude::*;

struct TicTacToe;

impl GameLogic for TicTacToe {
    type Config = ();
    type State = State;
    type ClientMessage = Move;
    type ServerMessage = Event;

    fn init(_: &(), players: &[PlayerId]) -> State { /* set up board */ }
    fn handle_message(state: &mut State, sender: PlayerId, msg: Move)
        -> Vec<(Recipient, Event)> { /* place mark, check win */ }
    fn is_finished(state: &State) -> bool { state.winner.is_some() }
}
```

The framework provides:

- **Transport** — WebSocket connections with binary framing
- **Protocol** — JSON-encoded message envelopes with sequencing
- **Sessions** — Pluggable authentication, reconnection tokens
- **Rooms** — Isolated game instances with player slot management
- **Tick scheduler** — Fixed-timestep loop (1–128 Hz) for real-time games

## Crates

| Crate | Purpose |
|---|---|
| `arcforge` | Meta-crate: server builder, prelude, re-exports |
| `arcforge-transport` | Transport abstraction + WebSocket implementation |
| `arcforge-protocol` | Wire format, message envelopes, codecs |
| `arcforge-session` | Player identity, auth hooks, session management |
| `arcforge-room` | Room lifecycle, player slots, state machine |
| `arcforge-tick` | Fixed-timestep tick scheduler |

## Status

🚧 **Phase 1: MVP (Turn-Based Foundation)** — In Progress

Core framework is functional with a working demo. Next up: reconnection handling, WebTransport, and client SDK.

## License

MIT OR Apache-2.0
