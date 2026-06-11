//! `granite-client` — the one client for talking to the Granite approval core.
//!
//! This crate replaces three divergent hand-rolled Granite clients (amber's,
//! drive's, and the compute-gateway's planned one) with a single source of
//! truth for:
//!
//! - **the wire contract** ([`model`]) — the create/verify/status request and
//!   response bodies, matching what Granite actually serves;
//! - **the header policy** ([`headers`]) — `x-user-id`, `x-user-can-write`,
//!   `x-granite-app-id`, `x-agent-id`, spelled once;
//! - **the destructive-intent taxonomy** ([`intent`]) — `PurgeKind` /
//!   `DestructiveIntent`, replacing the stringly-typed `"row"`/`"table"`
//!   pair, plus the single intent→approval-request mapping;
//! - **a typed async client** ([`client`]) over the real Granite endpoints:
//!   `POST /v1/grants/verify`, `POST /v1/grants/{id}/revoke`,
//!   `POST /v1/approval-requests`, `GET /v1/approval-requests/{id}`, and
//!   `POST /v1/projects/{project_id}/grants/verify`.
//!
//! Notably, [`model::ApprovalRequestStatus`] deserializes strictly: an
//! unknown status is a loud decode error, never a silent "pending".

pub mod client;
pub mod error;
pub mod headers;
pub mod intent;
pub mod model;

pub use client::{ActingUser, Auth, AwaitConfig, Decision, GraniteClient};
/// The shared trust-environment primitive, re-exported so consumers speak one
/// type: `granite_client::Environment`. An approval is minted in exactly one of
/// these (derived by Granite from the caller's keyset — never requester-set), and
/// a destructive action must only act on an approval whose environment matches the
/// caller's own. See [`ApprovalRequest::assert_environment`].
pub use capability::Environment;
pub use error::GraniteError;
pub use intent::{DestructiveIntent, PurgeKind};
pub use model::{
    ApprovalGrant, ApprovalGrantStatus, ApprovalRequest, ApprovalRequestStatus,
    ApprovalRequestType, ApprovalRiskLevel, CreateApprovalRequest, GrantVerification,
    VerifyGrantRequest,
};

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn approval_request_status_decodes_known_values() {
        for (s, want) in [
            ("\"pending\"", ApprovalRequestStatus::Pending),
            ("\"approved\"", ApprovalRequestStatus::Approved),
            ("\"denied\"", ApprovalRequestStatus::Denied),
        ] {
            let got: ApprovalRequestStatus = serde_json::from_str(s).unwrap();
            assert_eq!(got, want);
        }
    }

    #[test]
    fn unknown_approval_status_is_a_loud_decode_error() {
        // This is the whole point of the strict enum: a server that starts
        // returning a new status must surface an error, not silently decode
        // to a default. The hand-rolled clients did `_ => Pending`.
        let err = serde_json::from_str::<ApprovalRequestStatus>("\"rejected\"");
        assert!(err.is_err(), "unknown status must not deserialize");
    }

    #[test]
    fn approval_request_decodes_a_real_granite_body() {
        // Shape taken from Granite's ApprovalRequest serialization, with
        // extra server-only fields present to prove we ignore them.
        let body = json!({
            "id": "11111111-1111-1111-1111-111111111111",
            "owner_uid": "alice",
            "requester_app_id": "amber",
            "request_type": "one_time_action",
            "title": "Permanently drop table foo",
            "summary": "cleanup",
            "risk_level": "high",
            "requested_action": "drop_table",
            "requested_resource": "amber:table:foo",
            "requested_scopes": [],
            "status": "pending",
            "created_at": "2026-06-02T00:00:00Z",
            "updated_at": "2026-06-02T00:00:00Z"
        });
        let req: ApprovalRequest = serde_json::from_value(body).unwrap();
        assert_eq!(req.status, ApprovalRequestStatus::Pending);
        assert_eq!(req.requester_app_id, "amber");
        assert!(!req.status.is_terminal());
        // A body without `environment` (a pre-split Granite) reads as Prod.
        assert_eq!(req.environment, Environment::Prod);
    }

    #[test]
    fn environment_defaults_to_prod_and_gates_cross_env_isolation() {
        let base = json!({
            "id": "11111111-1111-1111-1111-111111111111",
            "owner_uid": "alice",
            "requester_app_id": "drive",
            "title": "Permanently erase owner",
            "summary": "crypto-shred",
            "requested_action": "crypto_shred",
            "requested_resource": "drive:owner:alice",
            "status": "approved"
        });

        // Untagged approval → strictest reading, Prod.
        let untagged: ApprovalRequest = serde_json::from_value(base.clone()).unwrap();
        assert_eq!(untagged.environment, Environment::Prod);

        // A Test-tagged approval round-trips to Test { tenant } (using the SHARED
        // type's own serde, so client and Granite can't disagree on the wire form).
        let test_env = Environment::Test { tenant: "acme".into() };
        let mut tagged = base.clone();
        tagged["environment"] = serde_json::to_value(&test_env).unwrap();
        let req: ApprovalRequest = serde_json::from_value(tagged).unwrap();
        assert_eq!(req.environment, test_env);

        // Isolation: an approval only authorizes an action in its OWN environment.
        assert!(req.assert_environment(&test_env).is_ok());
        assert!(
            matches!(
                req.assert_environment(&Environment::Prod),
                Err(GraniteError::EnvironmentMismatch { .. })
            ),
            "a Test approval must NOT authorize a Prod action"
        );
        assert!(
            untagged.assert_environment(&test_env).is_err(),
            "a Prod approval must NOT be settled by a Test caller"
        );
        assert!(untagged.assert_environment(&Environment::Prod).is_ok());
    }

    #[test]
    fn grant_verification_decodes() {
        let body = json!({
            "approved": false,
            "reason": "grant revoked"
        });
        let v: GrantVerification = serde_json::from_value(body).unwrap();
        assert!(!v.approved);
        assert_eq!(v.reason.as_deref(), Some("grant revoked"));
        assert!(v.grant.is_none());
    }

    #[test]
    fn create_request_omits_none_requester_app_id() {
        // App-credentialled path: the body must NOT carry requester_app_id
        // (Granite binds it), so it must be absent from the JSON.
        let req = CreateApprovalRequest {
            request_type: ApprovalRequestType::OneTimeAction,
            title: "t".into(),
            summary: "s".into(),
            requested_action: "a".into(),
            requested_resource: "r".into(),
            ..Default::default()
        };
        let v = serde_json::to_value(&req).unwrap();
        assert!(v.get("requester_app_id").is_none());
        assert_eq!(v["request_type"], "one_time_action");
        assert_eq!(v["risk_level"], "medium");
    }

    #[test]
    fn row_purge_carries_its_rid_inside_the_variant() {
        let intent = DestructiveIntent::row("amber", "ledger", 42)
            .with_reason("agent cleanup")
            .with_agent_id("agent-1");
        assert_eq!(intent.kind, PurgeKind::Row { rid: 42 });
        assert_eq!(intent.resource_ref(), "amber:row:ledger#42");
        assert_eq!(intent.kind.action(), "purge_row");

        let req = intent.to_approval_request();
        assert_eq!(req.request_type, ApprovalRequestType::OneTimeAction);
        assert_eq!(req.risk_level, ApprovalRiskLevel::High);
        assert_eq!(req.requested_action, "purge_row");
        assert_eq!(req.requested_resource, "amber:row:ledger#42");
        assert_eq!(req.requester_agent_id.as_deref(), Some("agent-1"));
        assert_eq!(req.summary, "agent cleanup");
        // App-credentialled callers let Granite bind the app id.
        assert!(req.requester_app_id.is_none());
    }

    #[test]
    fn table_purge_has_no_rid_and_maps_to_drop_table() {
        let intent = DestructiveIntent::table("amber", "ledger");
        assert_eq!(intent.kind, PurgeKind::Table);
        assert_eq!(intent.resource_ref(), "amber:table:ledger");
        let req = intent.to_approval_request();
        assert_eq!(req.requested_action, "drop_table");
        assert_eq!(req.requested_resource, "amber:table:ledger");
        assert_eq!(
            req.summary,
            "Permanent removal requested by an agent.",
            "default summary when no reason given"
        );
    }

    #[test]
    fn verify_grant_request_builder_serializes_minimally() {
        let id = uuid::Uuid::nil();
        let req = VerifyGrantRequest::for_grant(id)
            .with_resource("drive:files")
            .with_scopes(["drive.read".to_owned()]);
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["grant_id"], id.to_string());
        assert_eq!(v["resource"], "drive:files");
        assert_eq!(v["scopes"], json!(["drive.read"]));
        // request_id / subject fields are None -> absent.
        assert!(v.get("request_id").is_none());
        assert!(v.get("subject_chirp_sub").is_none());
    }

    #[test]
    fn header_constants_match_granite_policy() {
        assert_eq!(headers::USER_ID, "x-user-id");
        assert_eq!(headers::USER_CAN_WRITE, "x-user-can-write");
        assert_eq!(headers::GRANITE_APP_ID, "x-granite-app-id");
        assert_eq!(headers::AGENT_ID, "x-agent-id");
        assert_eq!(headers::can_write_value(true), "true");
        assert_eq!(headers::can_write_value(false), "false");
    }
}
