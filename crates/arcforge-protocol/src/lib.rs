//! Wire protocol for Arcforge.
//!
//! This crate defines the "language" that clients and servers speak:
//!
//! - **Types** ([`Envelope`], [`SystemMessage`], [`Channel`], etc.) —
//!   the message structures that travel on the wire.
//! - **Codec** ([`Codec`] trait, [`JsonCodec`]) — how those messages
//!   are converted to/from bytes.
//! - **Errors** ([`ProtocolError`]) — what can go wrong during
//!   encoding/decoding.
//!
//! # Architecture
//!
//! The protocol layer sits between transport (raw bytes) and session
//! (player identity). It doesn't know about connections or rooms —
//! it only knows how to serialize and deserialize messages.
//!
//! ```text
//! Transport (bytes) → Protocol (Envelope) → Session (player context)
//! ```

// ---------------------------------------------------------------------------
// Module declarations
// ---------------------------------------------------------------------------

// `mod` declares a submodule. Rust looks for the code in either:
//   - `src/types.rs` (file), or
//   - `src/types/mod.rs` (directory with mod.rs)
// We use the file approach since each module is a single file.

mod codec;
mod error;
mod types;

// ---------------------------------------------------------------------------
// Re-exports
// ---------------------------------------------------------------------------

// `pub use` makes items from submodules available at the crate root.
// Users can write `use arcforge_protocol::Envelope` instead of
// `use arcforge_protocol::types::Envelope`. This is a cleaner public API.

pub use codec::Codec;
#[cfg(feature = "json")]
pub use codec::JsonCodec;
pub use error::ProtocolError;
pub use types::{
    Channel, Envelope, Payload, PlayerId, Recipient, RoomId, RoomListEntry,
    SystemMessage,
};
