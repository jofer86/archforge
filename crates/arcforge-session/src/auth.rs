//! Authentication hook for validating player identity.
//!
//! Arcforge doesn't implement authentication itself — that's your job
//! (or your auth provider's: Firebase, Auth0, Supabase, custom JWT, etc.).
//!
//! Instead, Arcforge defines the [`Authenticator`] trait: a single async
//! method that takes a token string and returns a `PlayerId` or an error.
//! You implement this trait with your auth logic, and the framework calls
//! it during the handshake.
//!
//! # Why a trait?
//!
//! A trait is like an interface in other languages — it defines WHAT
//! something can do without specifying HOW. This lets us:
//! - Use JWT validation in production
//! - Use a simple "accept everyone" authenticator in development
//! - Use a mock authenticator in tests
//!
//! All without changing any framework code.

use arcforge_protocol::PlayerId;

use crate::SessionError;

/// Validates a client's auth token and returns their identity.
///
/// # Trait bounds
///
/// - `Send + Sync` → the authenticator can be shared across async tasks
///   (Tokio may call it from different threads simultaneously).
/// - `'static` → it doesn't borrow temporary data. This is required
///   because the authenticator lives as long as the server.
///
/// # Example
///
/// ```rust
/// use arcforge_session::{Authenticator, SessionError};
/// use arcforge_protocol::PlayerId;
///
/// /// Accepts any token and uses it as the player ID.
/// /// Only for development — never use this in production!
/// struct DevAuthenticator;
///
/// impl Authenticator for DevAuthenticator {
///     async fn authenticate(
///         &self,
///         token: &str,
///     ) -> Result<PlayerId, SessionError> {
///         // Parse the token as a number to use as the player ID.
///         // In production, you'd validate a JWT, call an auth API, etc.
///         let id: u64 = token.parse().map_err(|_| {
///             SessionError::AuthFailed("token must be a number".into())
///         })?;
///         Ok(PlayerId(id))
///     }
/// }
/// ```
pub trait Authenticator: Send + Sync + 'static {
    /// Validates the given token and returns the player's identity.
    ///
    /// Called during the handshake when a client sends a
    /// [`SystemMessage::Handshake`](arcforge_protocol::SystemMessage::Handshake)
    /// with a token.
    ///
    /// # Arguments
    /// - `token` — the auth token sent by the client (JWT, API key, etc.)
    ///
    /// # Returns
    /// - `Ok(PlayerId)` — authentication succeeded, here's who they are
    /// - `Err(SessionError::AuthFailed)` — token is invalid/expired
    fn authenticate(
        &self,
        token: &str,
    ) -> impl std::future::Future<Output = Result<PlayerId, SessionError>> + Send;
}
