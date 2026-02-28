//! Codec trait and implementations for serializing/deserializing messages.
//!
//! A "codec" (coder/decoder) converts between Rust types and raw bytes.
//! The protocol layer doesn't care HOW messages are serialized — it just
//! needs something that implements the [`Codec`] trait. This is the
//! "strategy pattern": we define an interface, and swap implementations.
//!
//! Currently we provide [`JsonCodec`] (human-readable, great for debugging).
//! Later we can add `BincodeCodec` (compact binary, better for production)
//! without changing any other code.

use serde::{de::DeserializeOwned, Serialize};

use crate::ProtocolError;

/// A codec that can encode Rust types to bytes and decode bytes back.
///
/// ## Trait bounds explained
///
/// - `Send + Sync` → safe to share between threads (required because
///   Tokio may run our code on any thread in its thread pool).
/// - `'static` → the codec doesn't borrow temporary data. It owns
///   everything it needs. This is required for types stored in
///   long-lived async tasks.
///
/// ## Generic methods
///
/// The `encode` and `decode` methods are *generic* — they work with ANY
/// type `T`, as long as `T` implements the right serde trait:
/// - `encode<T: Serialize>` → T can be turned into bytes
/// - `decode<T: DeserializeOwned>` → T can be created from bytes
///
/// `DeserializeOwned` (vs plain `Deserialize`) means the result doesn't
/// borrow from the input bytes — it owns all its data. This is important
/// because we often want to drop the input buffer after decoding.
pub trait Codec: Send + Sync + 'static {
    /// Serializes a value into bytes.
    ///
    /// # Errors
    /// Returns `ProtocolError::Encode` if serialization fails
    /// (e.g., the type contains values that can't be represented
    /// in this format).
    fn encode<T: Serialize>(
        &self,
        value: &T,
    ) -> Result<Vec<u8>, ProtocolError>;

    /// Deserializes bytes back into a value.
    ///
    /// # Errors
    /// Returns `ProtocolError::Decode` if the bytes are malformed,
    /// incomplete, or don't match the expected type.
    fn decode<T: DeserializeOwned>(
        &self,
        data: &[u8],
    ) -> Result<T, ProtocolError>;
}

// ---------------------------------------------------------------------------
// JsonCodec
// ---------------------------------------------------------------------------

/// A [`Codec`] that uses JSON (via `serde_json`).
///
/// JSON is human-readable, which makes it perfect for development:
/// you can inspect messages in browser DevTools, log them, and debug
/// issues easily. The tradeoff is size — JSON is larger than binary
/// formats. For production, you'd switch to a binary codec.
///
/// This is behind the `json` feature flag (enabled by default).
/// Feature flags let users opt out of dependencies they don't need.
///
/// ## Example
///
/// ```rust
/// use arcforge_protocol::{JsonCodec, Codec, Envelope, Payload, SystemMessage, Channel};
///
/// let codec = JsonCodec;
///
/// let envelope = Envelope {
///     seq: 1,
///     timestamp: 5000,
///     channel: Channel::ReliableOrdered,
///     payload: Payload::System(SystemMessage::Heartbeat { client_time: 5000 }),
/// };
///
/// // Encode to bytes (JSON)
/// let bytes = codec.encode(&envelope).unwrap();
///
/// // Decode back
/// let decoded: Envelope = codec.decode(&bytes).unwrap();
/// assert_eq!(envelope, decoded);
/// ```
#[cfg(feature = "json")]
#[derive(Debug, Clone, Copy, Default)]
pub struct JsonCodec;

#[cfg(feature = "json")]
impl Codec for JsonCodec {
    fn encode<T: Serialize>(
        &self,
        value: &T,
    ) -> Result<Vec<u8>, ProtocolError> {
        // `serde_json::to_vec` serializes directly to a `Vec<u8>`.
        // The `?` operator: if this returns an `Err`, convert it to
        // our `ProtocolError` type (via the `From` impl in error.rs)
        // and return early. If it's `Ok`, unwrap the value and continue.
        serde_json::to_vec(value).map_err(ProtocolError::Encode)
    }

    fn decode<T: DeserializeOwned>(
        &self,
        data: &[u8],
    ) -> Result<T, ProtocolError> {
        // `serde_json::from_slice` parses a `&[u8]` as JSON.
        // A "slice" (`&[u8]`) is a borrowed view into a byte array —
        // it doesn't copy the data, just points to it.
        serde_json::from_slice(data).map_err(ProtocolError::Decode)
    }
}
