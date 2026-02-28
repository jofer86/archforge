//! Error types for the protocol layer.
//!
//! Each crate in Arcforge defines its own error enum. This keeps errors
//! specific and meaningful — when you see a `ProtocolError`, you know
//! the problem is in serialization/deserialization, not in networking
//! or room management.

/// Errors that can occur in the protocol layer.
///
/// `#[derive(thiserror::Error)]` auto-generates the `std::error::Error`
/// trait implementation. The `#[error("...")]` attributes define the
/// human-readable message for each variant — what you see when you
/// print the error or it shows up in logs.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    /// Serialization failed (turning a Rust type into bytes).
    ///
    /// `#[error("encode failed: {0}")]` means printing this error
    /// will show something like: "encode failed: key must be a string".
    ///
    /// The inner `serde_json::Error` is the original error from serde_json.
    /// We wrap it so callers deal with `ProtocolError` uniformly,
    /// regardless of which codec produced the error.
    #[cfg(feature = "json")]
    #[error("encode failed: {0}")]
    Encode(serde_json::Error),

    /// Deserialization failed (turning bytes into a Rust type).
    ///
    /// Common causes: malformed JSON, missing required fields,
    /// wrong data types, or truncated messages.
    #[cfg(feature = "json")]
    #[error("decode failed: {0}")]
    Decode(serde_json::Error),

    /// The message is invalid at the protocol level.
    ///
    /// This is for logical errors that pass deserialization but
    /// violate protocol rules — e.g., a handshake with version 0,
    /// or an error code outside the valid range.
    #[error("invalid message: {0}")]
    InvalidMessage(String),
}
