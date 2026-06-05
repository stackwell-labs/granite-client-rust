//! The typed async Granite client.

use reqwest::RequestBuilder;
use uuid::Uuid;

use crate::error::GraniteError;
use crate::headers;
use crate::model::{
    ApprovalGrant, ApprovalRequest, ApprovalRequestStatus, CreateApprovalRequest, GrantVerification,
    VerifyGrantRequest,
};

/// How the client authenticates to Granite.
///
/// The two paths Granite supports map to two variants here, so a caller
/// cannot half-configure (e.g. set an app id but forget to flag itself as
/// app-credentialled, which silently sent the wrong headers in the
/// hand-rolled clients).
#[derive(Debug, Clone)]
pub enum Auth {
    /// The master internal-service-token (operator). The caller must supply
    /// `requester_app_id` in the body of created requests.
    InternalServiceToken(String),
    /// A registered app credential: the bearer secret plus the attested app
    /// id sent as `x-granite-app-id`. Granite binds `requester_app_id` to
    /// `app_id`, so created requests may omit it.
    AppCredential {
        /// The `gas_`-prefixed app secret.
        secret: String,
        /// The registered app id.
        app_id: String,
    },
}

/// A typed async client for the Granite approval core.
///
/// One client per `(base_url, auth)`; the acting-user uid and write flag are
/// passed per call, since a single service often acts on behalf of many
/// users.
#[derive(Debug, Clone)]
pub struct GraniteClient {
    base_url: String,
    auth: Auth,
    http: reqwest::Client,
}

impl GraniteClient {
    /// Build a client. `base_url` may include or omit a trailing slash.
    #[must_use]
    pub fn new(base_url: impl Into<String>, auth: Auth) -> Self {
        Self::with_http(base_url, auth, reqwest::Client::new())
    }

    /// Build a client over a caller-provided [`reqwest::Client`] (to share a
    /// connection pool / configure timeouts).
    #[must_use]
    pub fn with_http(base_url: impl Into<String>, auth: Auth, http: reqwest::Client) -> Self {
        let base_url = base_url.into().trim_end_matches('/').to_owned();
        Self {
            base_url,
            auth,
            http,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// Apply the bearer + identity headers for an acting user. This is the
    /// single place the header policy is applied for outbound calls.
    fn authed(&self, builder: RequestBuilder, acting_user: &ActingUser) -> RequestBuilder {
        let (token, app_id) = match &self.auth {
            Auth::InternalServiceToken(token) => (token.as_str(), None),
            Auth::AppCredential { secret, app_id } => (secret.as_str(), Some(app_id.as_str())),
        };
        let mut b = builder
            .bearer_auth(token)
            .header(headers::USER_ID, &acting_user.uid)
            .header(headers::USER_CAN_WRITE, headers::can_write_value(acting_user.can_write));
        if let Some(app_id) = app_id {
            b = b.header(headers::GRANITE_APP_ID, app_id);
        }
        if let Some(agent_id) = &acting_user.agent_id {
            b = b.header(headers::AGENT_ID, agent_id);
        }
        b
    }

    async fn send_json<T: serde::de::DeserializeOwned>(
        &self,
        builder: RequestBuilder,
    ) -> Result<T, GraniteError> {
        let resp = builder.send().await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(GraniteError::Status { status, body });
        }
        serde_json::from_str(&body).map_err(|e| GraniteError::Decode(e.to_string()))
    }

    /// `POST /v1/approval-requests`: file a new approval request.
    pub async fn create_approval_request(
        &self,
        acting_user: &ActingUser,
        request: &CreateApprovalRequest,
    ) -> Result<ApprovalRequest, GraniteError> {
        let builder = self
            .authed(self.http.post(self.url("/v1/approval-requests")), acting_user)
            .json(request);
        self.send_json(builder).await
    }

    /// `GET /v1/approval-requests/{id}`: read a request's current status.
    pub async fn get_approval_request(
        &self,
        acting_user: &ActingUser,
        request_id: Uuid,
    ) -> Result<ApprovalRequest, GraniteError> {
        let builder = self.authed(
            self.http
                .get(self.url(&format!("/v1/approval-requests/{request_id}"))),
            acting_user,
        );
        self.send_json(builder).await
    }

    /// `POST /v1/grants/verify`: verify a grant is active and in scope.
    pub async fn verify_grant(
        &self,
        acting_user: &ActingUser,
        request: &VerifyGrantRequest,
    ) -> Result<GrantVerification, GraniteError> {
        let builder = self
            .authed(self.http.post(self.url("/v1/grants/verify")), acting_user)
            .json(request);
        self.send_json(builder).await
    }

    /// `POST /v1/projects/{project_id}/grants/verify`: project-scoped verify.
    /// Operator-only at Granite; the acting user names the owner via headers.
    pub async fn verify_project_grant(
        &self,
        acting_user: &ActingUser,
        project_id: Uuid,
        request: &VerifyGrantRequest,
    ) -> Result<GrantVerification, GraniteError> {
        let builder = self
            .authed(
                self.http
                    .post(self.url(&format!("/v1/projects/{project_id}/grants/verify"))),
                acting_user,
            )
            .json(request);
        self.send_json(builder).await
    }

    /// `POST /v1/grants/{id}/revoke`: revoke a standing grant.
    pub async fn revoke_grant(
        &self,
        acting_user: &ActingUser,
        grant_id: Uuid,
    ) -> Result<ApprovalGrant, GraniteError> {
        let builder = self.authed(
            self.http
                .post(self.url(&format!("/v1/grants/{grant_id}/revoke"))),
            acting_user,
        );
        self.send_json(builder).await
    }

    /// Single non-blocking check: the terminal [`Decision`] if the request has
    /// been decided, or `None` while still pending.
    ///
    /// This is the runtime-agnostic core of in-flow approval: any caller — an
    /// AI agent, a CLI, a CI job, a web handler — drives its own loop around
    /// this, and can **resume** later from just the `request_id` (the state
    /// lives in the Granite record, not in the caller).
    pub async fn poll_decision(
        &self,
        acting_user: &ActingUser,
        request_id: Uuid,
    ) -> Result<Option<Decision>, GraniteError> {
        let request = self.get_approval_request(acting_user, request_id).await?;
        Ok(decision_from_request(request))
    }

    /// Block until the request reaches a terminal state, polling every
    /// `config.interval`. Resumable: pass a known id to await a request created
    /// in an earlier process. Returns [`GraniteError::Timeout`] if
    /// `config.timeout` elapses first — the request stays pending and can be
    /// awaited again.
    pub async fn await_decision(
        &self,
        acting_user: &ActingUser,
        request_id: Uuid,
        config: AwaitConfig,
    ) -> Result<Decision, GraniteError> {
        let started = std::time::Instant::now();
        loop {
            if let Some(decision) = self.poll_decision(acting_user, request_id).await? {
                return Ok(decision);
            }
            if let Some(timeout) = config.timeout {
                if started.elapsed() >= timeout {
                    return Err(GraniteError::Timeout(timeout));
                }
            }
            tokio::time::sleep(config.interval).await;
        }
    }

    /// Create a request and await its decision — the common in-flow case.
    /// Equivalent to [`create_approval_request`](Self::create_approval_request)
    /// followed by [`await_decision`](Self::await_decision).
    pub async fn request_and_await(
        &self,
        acting_user: &ActingUser,
        request: &CreateApprovalRequest,
        config: AwaitConfig,
    ) -> Result<Decision, GraniteError> {
        let created = self.create_approval_request(acting_user, request).await?;
        self.await_decision(acting_user, created.id, config).await
    }
}

/// The terminal outcome of an approval request, for callers driving the
/// request → decide → react loop. Carries the full request so a caller can read
/// `grant_id` (to verify/redeem the resulting grant) or the denial reason.
#[derive(Debug, Clone)]
pub enum Decision {
    /// The owner approved the request.
    Approved(ApprovalRequest),
    /// The owner denied it; see [`Decision::reason`].
    Denied(ApprovalRequest),
}

impl Decision {
    /// The underlying request record (approved or denied).
    #[must_use]
    pub fn request(&self) -> &ApprovalRequest {
        match self {
            Decision::Approved(request) | Decision::Denied(request) => request,
        }
    }

    /// Whether the request was approved.
    #[must_use]
    pub fn is_approved(&self) -> bool {
        matches!(self, Decision::Approved(_))
    }

    /// The decision reason, if one was recorded (typically on denial). Lets a
    /// caller surface "denied: <why>" — an agent to pivot on, a CLI to print.
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        self.request().decision_reason.as_deref()
    }
}

/// Pacing for [`GraniteClient::await_decision`]. [`Default`] polls every 3s
/// with no overall deadline (wait indefinitely).
#[derive(Debug, Clone)]
pub struct AwaitConfig {
    /// Delay between status polls.
    pub interval: std::time::Duration,
    /// Optional overall deadline; `None` waits indefinitely.
    pub timeout: Option<std::time::Duration>,
}

impl Default for AwaitConfig {
    fn default() -> Self {
        Self {
            interval: std::time::Duration::from_secs(3),
            timeout: None,
        }
    }
}

/// Pure map from a request to its terminal [`Decision`], or `None` while
/// pending. The network-free core of the poll/await loop.
fn decision_from_request(request: ApprovalRequest) -> Option<Decision> {
    match request.status {
        ApprovalRequestStatus::Pending => None,
        ApprovalRequestStatus::Approved => Some(Decision::Approved(request)),
        ApprovalRequestStatus::Denied => Some(Decision::Denied(request)),
    }
}

/// The user a request acts on behalf of, plus their write flag and optional
/// acting agent id. Passed per call rather than baked into the client.
#[derive(Debug, Clone)]
pub struct ActingUser {
    /// The owner uid (`x-user-id`).
    pub uid: String,
    /// Whether the user may write (`x-user-can-write`).
    pub can_write: bool,
    /// Optional Loom agent id (`x-agent-id`).
    pub agent_id: Option<String>,
}

impl ActingUser {
    /// A read-only acting user (`can_write = false`, no agent id). The right
    /// default for verification calls.
    #[must_use]
    pub fn read_only(uid: impl Into<String>) -> Self {
        Self {
            uid: uid.into(),
            can_write: false,
            agent_id: None,
        }
    }

    /// A writing acting user (`can_write = true`). Required for create /
    /// approve / deny / revoke.
    #[must_use]
    pub fn writer(uid: impl Into<String>) -> Self {
        Self {
            uid: uid.into(),
            can_write: true,
            agent_id: None,
        }
    }

    /// Attach a Loom agent id.
    #[must_use]
    pub fn with_agent_id(mut self, agent_id: impl Into<String>) -> Self {
        self.agent_id = Some(agent_id.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Build an ApprovalRequest from minimal JSON (lenient deserialize) so we can
    // exercise the pure decision mapping without a server.
    fn request_with_status(status: &str) -> ApprovalRequest {
        serde_json::from_value(serde_json::json!({
            "id": "00000000-0000-0000-0000-000000000001",
            "owner_uid": "owner1",
            "requester_app_id": "drive",
            "title": "delete files",
            "summary": "remove three files",
            "requested_action": "delete",
            "requested_resource": "storage:drive:/sites/x",
            "status": status,
            "decision_reason": "those files are still needed"
        }))
        .expect("minimal approval request should deserialize")
    }

    #[test]
    fn pending_request_yields_no_decision() {
        assert!(decision_from_request(request_with_status("pending")).is_none());
    }

    #[test]
    fn approved_maps_to_approved_decision() {
        let decision = decision_from_request(request_with_status("approved"))
            .expect("approved is terminal");
        assert!(decision.is_approved());
    }

    #[test]
    fn denied_maps_to_denied_decision_and_surfaces_reason() {
        let decision =
            decision_from_request(request_with_status("denied")).expect("denied is terminal");
        assert!(!decision.is_approved());
        assert_eq!(decision.reason(), Some("those files are still needed"));
    }
}
