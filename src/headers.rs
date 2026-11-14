//! The Granite header policy, in one place.
//!
//! Granite's auth layer reads a fixed set of request headers. The three
//! hand-rolled clients this crate replaces each re-spelled these string
//! literals inline; a typo (`x-user_id`) is a
//! silent auth failure, not a compile error. Owning the constants here
//! makes the spelling a single source of truth.

/// The user the request acts on behalf of (the grant/approval owner uid).
/// For a machine or end-user-bearer call it names the acting user when the
/// caller speaks for a user other than the bearer.
pub const USER_ID: &str = "x-user-id";
