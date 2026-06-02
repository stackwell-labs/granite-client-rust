//! The typed async Granite client.

use reqwest::RequestBuilder;
use uuid::Uuid;

use crate::error::GraniteError;
use crate::headers;
use crate::model::{
    ApprovalGrant, ApprovalRequest, CreateApprovalRequest, GrantVerification, VerifyGrantRequest,
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
