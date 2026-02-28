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

## Quick Start

```rust
use arcforge::prelude::*;

struct MyGame;

impl GameLogic for MyGame {
    type Config = ();
    type State = MyState;
    type ClientMessage = ClientMsg;
    type ServerMessage = ServerMsg;

    fn init(_config: &(), players: &[PlayerId]) -> MyState { todo!() }
    fn handle_message(state: &mut MyState, sender: PlayerId, msg: ClientMsg)
        -> Vec<(Recipient, ServerMsg)> { todo!() }
    fn is_finished(state: &MyState) -> bool { false }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = ArcforgeServer::builder()
        .bind("0.0.0.0:8080")
        .game::<MyGame>()
        .build()
        .await?;
    server.run().await
}
```

## Crates

| Crate | Purpose |
|---|---|
| `arcforge` | Meta-crate: server builder, prelude, re-exports |
| `arcforge-transport` | Transport abstraction + WebSocket/WebTransport |
| `arcforge-protocol` | Wire format, message envelopes, codecs |
| `arcforge-session` | Player identity, auth hooks, session management |
| `arcforge-room` | Room lifecycle, player slots, state machine |
| `arcforge-tick` | Fixed-timestep tick scheduler |

## Client SDK

```typescript
import { ArcforgeClient } from "@arcforge/client";

const client = new ArcforgeClient("ws://localhost:8080");
const room = await client.joinOrCreate<GameState, ClientMsg, ServerMsg>("my-game");

room.onMessage("MarkerPlaced", (msg) => render(msg));
room.send({ kind: "PlaceMarker", row: 1, col: 1 });
```

## Status

🚧 **Phase 1: MVP (Turn-Based Foundation)** — In Progress

See [roadmap](.kiro/steering/roadmap.md) for details.

## License

MIT OR Apache-2.0