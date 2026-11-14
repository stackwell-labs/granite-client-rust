# granite-client

One typed async Rust client for the [Granite](https://granite) approval core —
the single source of truth for its wire contract, header policy, and
destructive-intent taxonomy. It replaces three divergent hand-rolled clients
(amber's, Drive's, and the compute-gateway's planned one).

## What it owns

- **Wire types** (`model`) matching what Granite actually serves:
  `CreateApprovalRequest`, `ApprovalRequest`, `VerifyGrantRequest`,
  `GrantVerification`, `ApprovalGrant`, and `SubjectKind` (an app, or a public
  ChirpAuth client whose grant an enforcer verifies with
  `VerifyGrantRequest::with_subject_client_id`, naming the person's token's
  `client_id`).
- **A strict status enum** — `ApprovalRequestStatus` has no default and no
  catch-all. An unknown status string is a **loud decode error**, never a
  silent "pending" (the bug the hand-rolled clients carried via `_ => Pending`).
- **Header policy** (`headers`): `x-user-id` and `x-granite-app-id`,
  spelled once.
- **Destructive-intent taxonomy** (`intent`): `PurgeKind::Row { rid }` /
  `PurgeKind::Table` carries its payload in the variant, so a table purge
  cannot carry a row id and a row purge cannot omit one. `DestructiveIntent::to_approval_request`
  is the single intent→wire mapping.
- **A typed client** (`client::GraniteClient`) over the real endpoints:
  - `POST /v1/approval-requests`
  - `GET /v1/approval-requests/{id}`
  - `POST /v1/grants/verify`
  - `POST /v1/approval-requests/{id}/withdraw`
  - `POST /v1/grants/{id}/revoke`

## Endpoints it does NOT invent

There is no `GET /v1/grants/{owner}/{app}` and no `capabilities` field. The
client models only what Granite serves.

## Usage

```rust
use granite_client::{ActingUser, Auth, DestructiveIntent, GraniteClient, VerifyGrantRequest};

# async fn example() -> Result<(), granite_client::GraniteError> {
// App-credentialled caller (Granite binds requester_app_id).
let client = GraniteClient::new(
    "https://granite.example",
    Auth::AppCredential { secret: "gas_…".into(), app_id: "amber".into() },
);

// File a high-risk purge for a human to approve.
let intent = DestructiveIntent::table("amber", "ledger").with_reason("agent cleanup");
let req = client
    .create_approval_request(&ActingUser::new("alice"), &intent.to_approval_request())
    .await?;

// Later: verify a standing grant before acting on it.
let v = client
    .verify_grant(
        &ActingUser::new("alice"),
        &VerifyGrantRequest::for_grant(some_grant_id).with_scopes(["drive.read".into()]),
    )
    .await?;
assert!(v.approved);
# let _ = req; Ok(())
# }
```

## Auth model

- `Auth::ChirpBearer(token)` — a chirp identity token as the bearer (a
  machine token, or a person's). Granite binds `requester_app_id` for a
  machine token's bound app or client id; a person's bearer sets it.
- `Auth::AppCredential { secret, app_id }` — sends `x-granite-app-id`; Granite
  binds `requester_app_id` to the attested app, so leave it `None`.

The acting user (`x-user-id`) is passed per call via `ActingUser::new(uid)`,
since one service acts for many users.

## License

MIT OR Apache-2.0.
