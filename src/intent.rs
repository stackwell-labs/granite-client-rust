//! Destructive-intent taxonomy as a type, not a string.
//!
//! amber's hand-rolled client carried a `kind: String` that was "row" or
//! "table", with the row id smuggled alongside in a separate `Option<i64>`
//! that was only meaningful for the row case. That is an illegal-state
//! factory: a `"table"` with a stray `rid`, a `"row"` with `rid: None`.
//!
//! [`PurgeKind`] makes the row id live *inside* the row variant, so a table
//! purge cannot carry one and a row purge cannot omit one. [`DestructiveIntent`]
//! wraps it with the context Granite needs to render a consent page, and
//! [`DestructiveIntent::to_approval_request`] is the single mapping from
//! intent to wire body.

use crate::model::{ApprovalRequestType, ApprovalRiskLevel, CreateApprovalRequest};

/// What is being permanently removed. The payload lives in the variant, so
/// the "which fields apply" question is answered by construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PurgeKind {
    /// A single row, identified by its row id.
    Row { rid: i64 },
    /// An entire table.
    Table,
}

impl PurgeKind {
    /// The Granite `requested_action` verb for this purge.
    #[must_use]
    pub fn action(&self) -> &'static str {
        match self {
            PurgeKind::Row { .. } => "purge_row",
            PurgeKind::Table => "drop_table",
        }
    }
}

/// A destructive operation an agent wants a human to approve before it runs.
///
/// `target` is the logical object the purge applies to (e.g. an amber table
/// name); the `kind` carries any per-shape payload. `resource_prefix`
/// namespaces the Granite `requested_resource` so two products' "table foo"
/// refs never collide (e.g. `"amber"` -> `amber:table:foo`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestructiveIntent {
    /// Resource namespace for the requesting product (e.g. `"amber"`).
    pub resource_prefix: String,
    /// The object being purged (e.g. a table name).
    pub target: String,
    /// The shape of the purge, carrying its own payload.
    pub kind: PurgeKind,
    /// Optional human-readable reason shown on the consent page.
    pub reason: Option<String>,
    /// Optional id of the agent that requested the purge.
    pub agent_id: Option<String>,
}

impl DestructiveIntent {
    /// Construct a row-purge intent.
    #[must_use]
    pub fn row(
        resource_prefix: impl Into<String>,
        target: impl Into<String>,
        rid: i64,
    ) -> Self {
        Self {
            resource_prefix: resource_prefix.into(),
            target: target.into(),
            kind: PurgeKind::Row { rid },
            reason: None,
            agent_id: None,
        }
    }

    /// Construct a table-drop intent.
    #[must_use]
    pub fn table(resource_prefix: impl Into<String>, target: impl Into<String>) -> Self {
        Self {
            resource_prefix: resource_prefix.into(),
            target: target.into(),
            kind: PurgeKind::Table,
            reason: None,
            agent_id: None,
        }
    }

    /// Attach a human-readable reason.
    #[must_use]
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    /// Attach the requesting agent's id.
    #[must_use]
    pub fn with_agent_id(mut self, agent_id: impl Into<String>) -> Self {
        self.agent_id = Some(agent_id.into());
        self
    }

    /// The Granite `requested_resource` ref for this intent.
    #[must_use]
    pub fn resource_ref(&self) -> String {
        match &self.kind {
            PurgeKind::Row { rid } => {
                format!("{}:row:{}#{}", self.resource_prefix, self.target, rid)
            }
            PurgeKind::Table => format!("{}:table:{}", self.resource_prefix, self.target),
        }
    }

    /// The default consent-page title.
    #[must_use]
    pub fn title(&self) -> String {
        match &self.kind {
            PurgeKind::Row { rid } => {
                format!("Permanently delete {}#{}", self.target, rid)
            }
            PurgeKind::Table => format!("Permanently drop table {}", self.target),
        }
    }

    /// The single mapping from a destructive intent to a Granite
    /// approval-request body. A purge is always a high-risk
    /// `one_time_action` (it creates no standing grant). `requester_app_id`
    /// is left `None` so an app-credentialled caller can let Granite bind
    /// it; an internal-service-token caller sets it before sending.
    #[must_use]
    pub fn to_approval_request(&self) -> CreateApprovalRequest {
        let summary = self
            .reason
            .clone()
            .unwrap_or_else(|| "Permanent removal requested by an agent.".to_owned());
        CreateApprovalRequest {
            requester_app_id: None,
            requester_agent_id: self.agent_id.clone(),
            request_type: ApprovalRequestType::OneTimeAction,
            title: self.title(),
            summary,
            risk_level: ApprovalRiskLevel::High,
            requested_action: self.kind.action().to_owned(),
            requested_resource: self.resource_ref(),
            ..CreateApprovalRequest::default()
        }
    }
}
