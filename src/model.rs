//! The Granite wire contract, owned as public types.
//!
//! These mirror the request/response bodies Granite's `model.rs` serves.
//! Only the fields a client needs to send or read are modelled; Granite's
//! responses carry more (full `ApprovalRequest`/`ApprovalGrant` records),
//! and `#[serde(default)]` plus permissive structs keep this crate
//! forward-compatible with fields it does not yet care about.
//!
//! The one deliberately strict spot is [`ApprovalRequestStatus`]: it has no
//! `default` and no catch-all variant, so an unrecognized status is a loud
//! decode error rather than a silent "pending". A divergent server that
//! starts returning a new status must be noticed, not absorbed.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// The shape of approval Granite is being asked to grant. Mirrors
/// Granite's `ApprovalRequestType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalRequestType {
    /// A single action approved once, creating no standing grant.
    #[default]
    OneTimeAction,
    /// A standing capability grant (e.g. a tool capability).
    CapabilityGrant,
    /// A standing storage grant over a `storage:{provider}:{resource}` ref.
    StorageGrant,
    /// A delegation grant (e.g. an external agent acting on a thread).
    DelegationGrant,
}

/// The lifecycle state of an approval request.
///
/// Deliberately strict: no `#[serde(other)]`, no `Default`. An unknown
/// status string fails to deserialize — surfacing a server/client contract
/// drift loudly instead of silently treating it as [`Self::Pending`]. This
/// is the bug the amber and drive hand-rolled clients each carried (their
/// `_ => Pending` arms turned a denied/unknown decision into "keep
/// polling").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalRequestStatus {
    /// Awaiting a human decision.
    Pending,
    /// The user approved it.
    Approved,
    /// The user denied it.
    Denied,
}

impl ApprovalRequestStatus {
    /// Whether the request has reached a terminal (decided) state.
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Approved | Self::Denied)
    }
}

/// Caller-facing risk hint surfaced on the user's consent page / push.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalRiskLevel {
    /// Low risk.
    Low,
    /// Medium risk (Granite's default).
    #[default]
    Medium,
    /// High risk.
    High,
}

/// Standing-grant lifecycle state, as returned on grant records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalGrantStatus {
    /// The grant is live.
    Active,
    /// The grant was revoked.
    Revoked,
}

/// Body for `POST /v1/approval-requests` (and the project-scoped variant).
///
/// `requester_app_id` is `Option`: an app-credentialled caller MUST omit it
/// (Granite binds it from the attested app id), while an
/// internal-service-token caller MUST supply it. Encoding that as `Option`
/// — and skipping it when `None` — keeps the wire body matching whichever
/// auth path the caller is on.
#[derive(Debug, Clone, Serialize, Default)]
pub struct CreateApprovalRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requester_app_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requester_agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requester_chirp_sub: Option<String>,
    pub request_type: ApprovalRequestType,
    pub title: String,
    pub summary: String,
    pub risk_level: ApprovalRiskLevel,
    pub requested_action: String,
    pub requested_resource: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub requested_scopes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposed_limits: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callback_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callback_secret: Option<String>,
}

/// The approval-request record Granite returns from create and status reads.
///
/// Models the fields a client reads; unknown fields are ignored. Status is
/// the strict [`ApprovalRequestStatus`].
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ApprovalRequest {
    pub id: Uuid,
    pub owner_uid: String,
    pub requester_app_id: String,
    #[serde(default)]
    pub requester_agent_id: Option<String>,
    #[serde(default)]
    pub requester_chirp_sub: Option<String>,
    #[serde(default)]
    pub request_type: ApprovalRequestType,
    pub title: String,
    pub summary: String,
    #[serde(default)]
    pub risk_level: ApprovalRiskLevel,
    pub requested_action: String,
    pub requested_resource: String,
    #[serde(default)]
    pub requested_scopes: Vec<String>,
    pub status: ApprovalRequestStatus,
    #[serde(default)]
    pub grant_id: Option<Uuid>,
    #[serde(default)]
    pub trace_id: Option<String>,
}

/// A standing grant record, as embedded in a verification response.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ApprovalGrant {
    pub id: Uuid,
    pub owner_uid: String,
    pub request_id: Uuid,
    pub subject_app_id: String,
    #[serde(default)]
    pub subject_agent_id: Option<String>,
    #[serde(default)]
    pub subject_chirp_sub: Option<String>,
    pub service_id: String,
    pub resource: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub limits: Option<Value>,
    pub status: ApprovalGrantStatus,
}

/// Body for `POST /v1/grants/verify` (and the project-scoped variant).
///
/// At least one of `request_id` / `grant_id` is supplied to name the grant;
/// `resource` and `scopes` narrow what is being checked.
#[derive(Debug, Clone, Serialize, Default)]
pub struct VerifyGrantRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_chirp_sub: Option<String>,
}

impl VerifyGrantRequest {
    /// A verification keyed on a grant id, the common case.
    #[must_use]
    pub fn for_grant(grant_id: Uuid) -> Self {
        Self {
            grant_id: Some(grant_id),
            ..Self::default()
        }
    }

    /// A verification keyed on an approval-request id.
    #[must_use]
    pub fn for_request(request_id: Uuid) -> Self {
        Self {
            request_id: Some(request_id),
            ..Self::default()
        }
    }

    /// Narrow the check to a specific resource ref.
    #[must_use]
    pub fn with_resource(mut self, resource: impl Into<String>) -> Self {
        self.resource = Some(resource.into());
        self
    }

    /// Narrow the check to specific scope actions.
    #[must_use]
    pub fn with_scopes(mut self, scopes: impl IntoIterator<Item = String>) -> Self {
        self.scopes = scopes.into_iter().collect();
        self
    }

    /// Require the grant to be bound to this ChirpAuth machine subject.
    #[must_use]
    pub fn with_subject_chirp_sub(mut self, sub: impl Into<String>) -> Self {
        self.subject_chirp_sub = Some(sub.into());
        self
    }
}

/// Response from `POST /v1/grants/verify`. `approved` is the load-bearing
/// boolean; `reason` explains a `false`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GrantVerification {
    pub approved: bool,
    #[serde(default)]
    pub request: Option<ApprovalRequest>,
    #[serde(default)]
    pub grant: Option<ApprovalGrant>,
    #[serde(default)]
    pub reason: Option<String>,
}
