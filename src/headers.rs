//! The Granite header policy, in one place.
//!
//! Granite's auth layer reads a fixed set of request headers. The three
//! hand-rolled clients this crate replaces each re-spelled these string
//! literals inline; a typo (`x-user-can_write`, `x-granite-app`) is a
//! silent auth failure, not a compile error. Owning the constants here
//! makes the spelling a single source of truth.

/// The user the request acts on behalf of (the grant/approval owner uid).
/// Required on every internal-service-token call; for an app-credentialled
/// or end-user-bearer call it names the acting user when the caller speaks
/// for a user other than the bearer.
pub const USER_ID: &str = "x-user-id";

/// Whether the acting user may perform writes. `"true"` / `"false"`.
/// Granite's `require_write_access` gate reads this on every mutating
/// route (create/approve/deny/revoke/register-device).
pub const USER_CAN_WRITE: &str = "x-user-can-write";

/// The registered app id a caller authenticates as, alongside its bearer
/// app secret. When present, Granite binds `requester_app_id` to this
/// attested value rather than trusting the request body.
pub const GRANITE_APP_ID: &str = "x-granite-app-id";

/// The Drive-resource id of the agent acting under a Loom-hosted grant.
/// Carried so verification can match a grant's stored `subject_agent_id`.
pub const AGENT_ID: &str = "x-agent-id";

/// The canonical `"true"` / `"false"` rendering of the write flag, so
/// callers never hand-spell the boolean.
#[must_use]
pub fn can_write_value(can_write: bool) -> &'static str {
    if can_write {
        "true"
    } else {
        "false"
    }
}
