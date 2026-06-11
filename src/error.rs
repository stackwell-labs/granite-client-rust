//! The client's error type.

use reqwest::StatusCode;

/// Anything that can go wrong talking to Granite.
///
/// `Status` keeps the HTTP code and body so a caller can distinguish a 403
/// (auth) from a 404 (no such request) from a 400 (bad body) without string
/// matching; `Decode` carries the serde message so a contract drift —
/// notably an unknown `ApprovalRequestStatus` — surfaces as a real error
/// rather than a silent default.
#[derive(Debug, thiserror::Error)]
pub enum GraniteError {
    /// The HTTP request itself failed (connect/timeout/TLS).
    #[error("granite http error: {0}")]
    Http(#[from] reqwest::Error),
    /// Granite returned a non-2xx status.
    #[error("granite returned {status}: {body}")]
    Status {
        /// The HTTP status code.
        status: StatusCode,
        /// The response body (often a JSON error message).
        body: String,
    },
    /// The response body did not match the expected wire type.
    #[error("granite response decode error: {0}")]
    Decode(String),
    /// `await_decision` gave up before the request reached a terminal state.
    /// The request is unaffected — still pending, and can be awaited again by
    /// id (state lives in the record, not the caller).
    #[error("timed out after {0:?} waiting for an approval decision")]
    Timeout(std::time::Duration),
    /// An approval was minted in a different trust environment than the caller
    /// is running in — e.g. a `Test` approval presented to authorize a `Prod`
    /// destructive action. Refused: a test authority must never launder into
    /// prod (or vice versa). Raised by [`crate::ApprovalRequest::assert_environment`].
    #[error("approval environment mismatch: expected {expected}, found {found}")]
    EnvironmentMismatch {
        /// The environment the caller is running in.
        expected: String,
        /// The environment the approval was actually minted in.
        found: String,
    },
}
