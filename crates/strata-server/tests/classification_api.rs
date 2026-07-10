//! End-to-end tests for classification-driven storage placement and
//! encryption (STORE-04 × CAPTURE-10).
//!
//! The test server attaches two in-memory backends and keeps direct handles
//! to them, so assertions can inspect the *raw stored bytes* — the only way
//! to prove encryption at rest actually happened rather than trusting the
//! metadata. The external backend is deliberately listed first: placement
//! must be driven by policy, not by configuration order alone.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use strata_common::{BackendLocation, DocumentId};
use strata_server::{AppState, OperatorKey, StorageBackend, app};
use strata_storage::{MemoryProvider, StorageProvider};
use tower::ServiceExt;

const CONTENT: &[u8] = b"quarterly numbers, not yet public";

struct Backends {
    internal: Arc<MemoryProvider>,
    external: Arc<MemoryProvider>,
}

/// A server with an external backend listed *before* an internal one, plus
/// handles to both for raw-byte inspection.
fn server() -> (Router, Backends) {
    let internal = Arc::new(MemoryProvider::new());
    let external = Arc::new(MemoryProvider::new());
    let backends = vec![
        StorageBackend {
            name: "cloud".into(),
            location: BackendLocation::External,
            provider: external.clone(),
        },
        StorageBackend {
            name: "vault".into(),
            location: BackendLocation::Internal,
            provider: internal.clone(),
        },
    ];
    let state = AppState::with_storage(backends, OperatorKey::generate());
    (app(Arc::new(state)), Backends { internal, external })
}

/// A server whose only backend is external — the configuration in which
/// strictly confidential content has nowhere to go.
fn external_only_server() -> Router {
    let backends = vec![StorageBackend {
        name: "cloud".into(),
        location: BackendLocation::External,
        provider: Arc::new(MemoryProvider::new()),
    }];
    app(Arc::new(AppState::with_storage(
        backends,
        OperatorKey::generate(),
    )))
}

/// Send one JSON request as `user` and return status code plus parsed body.
async fn send(
    server: &Router,
    method: &str,
    uri: &str,
    user: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("x-strata-user", user);
    let request = match body {
        Some(json) => request
            .header("content-type", "application/json")
            .body(Body::from(json.to_string())),
        None => request.body(Body::empty()),
    }
    .unwrap();

    let response = server.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, value)
}

async fn create_document(server: &Router, user: &str, body: Value) -> String {
    let (status, doc) = send(server, "POST", "/documents", user, Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{doc}");
    doc["id"].as_str().unwrap().to_owned()
}

/// `PUT /documents/{id}/content` with a raw body.
async fn upload(server: &Router, user: &str, id: &str, content: &[u8]) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("PUT")
        .uri(format!("/documents/{id}/content"))
        .header("x-strata-user", user)
        .body(Body::from(content.to_vec()))
        .unwrap();
    let response = server.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, value)
}

/// `GET /documents/{id}/content`, returning the raw response body.
async fn download(server: &Router, user: &str, id: &str) -> (StatusCode, Vec<u8>) {
    let request = Request::builder()
        .method("GET")
        .uri(format!("/documents/{id}/content"))
        .header("x-strata-user", user)
        .body(Body::empty())
        .unwrap();
    let response = server.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, bytes.to_vec())
}

/// The raw bytes a backend holds for a document, straight from the provider.
async fn raw_blob(provider: &MemoryProvider, id: &str) -> Option<Vec<u8>> {
    provider.get(DocumentId(id.parse().unwrap())).await.ok()
}

#[tokio::test]
async fn documents_default_to_internal_and_accept_an_explicit_tier() {
    let (server, _) = server();

    let (status, doc) = send(
        &server,
        "POST",
        "/documents",
        "alice",
        Some(json!({ "title": "memo" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(doc["classification"], "internal");
    assert_eq!(doc["content"], Value::Null);
    assert_eq!(doc["classification_history"], json!([]));

    let (status, doc) = send(
        &server,
        "POST",
        "/documents",
        "alice",
        Some(json!({ "title": "press release", "classification": "public" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(doc["classification"], "public");
}

#[tokio::test]
async fn content_reaching_external_infrastructure_is_encrypted_at_rest() {
    let (server, backends) = server();
    let id = create_document(&server, "alice", json!({ "title": "report" })).await;

    let (status, doc) = upload(&server, "alice", &id, CONTENT).await;
    assert_eq!(status, StatusCode::OK, "{doc}");

    // Baseline policy allows `internal` documents on external media, and the
    // external backend is listed first — so that is where the blob went…
    assert_eq!(doc["content"]["backend"], "cloud");
    assert_eq!(doc["content"]["location"], "external");
    assert_eq!(doc["content"]["encrypted"], true);
    assert_eq!(doc["content"]["size"], CONTENT.len());
    assert_eq!(doc["content"]["stored_by"], "alice");

    // …but what the backend holds is ciphertext, not the document.
    let raw = raw_blob(&backends.external, &id).await.unwrap();
    assert_ne!(raw, CONTENT);
    assert!(!raw.windows(CONTENT.len()).any(|window| window == CONTENT));

    // Callers still read plaintext; encryption at rest is not their concern.
    let (status, body) = download(&server, "alice", &id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, CONTENT);
}

#[tokio::test]
async fn strictly_confidential_content_never_reaches_external_backends() {
    let (server, backends) = server();
    let id = create_document(
        &server,
        "alice",
        json!({ "title": "board minutes", "classification": "strictly_confidential" }),
    )
    .await;

    let (status, doc) = upload(&server, "alice", &id, CONTENT).await;
    assert_eq!(status, StatusCode::OK, "{doc}");

    // The external backend comes first in configuration order, but the
    // policy forbids it — placement must skip to the internal one.
    assert_eq!(doc["content"]["backend"], "vault");
    assert_eq!(doc["content"]["location"], "internal");
    assert_eq!(doc["content"]["encrypted"], true);

    assert!(raw_blob(&backends.external, &id).await.is_none());
    let raw = raw_blob(&backends.internal, &id).await.unwrap();
    assert_ne!(raw, CONTENT, "strictly confidential blobs are encrypted");

    let (status, body) = download(&server, "alice", &id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, CONTENT);
}

#[tokio::test]
async fn public_content_may_rest_in_plaintext_on_internal_media() {
    let (server, backends) = server();
    let id = create_document(
        &server,
        "alice",
        json!({ "title": "flyer", "classification": "public" }),
    )
    .await;

    // Restrict public documents to internal media so the plaintext rule is
    // observable (on external media encryption is invariant).
    let (status, _) = send(
        &server,
        "PUT",
        "/policy/placement",
        "admin",
        Some(json!({
            "public": { "allow_external": false, "encrypt_internal": false }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, doc) = upload(&server, "alice", &id, CONTENT).await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    assert_eq!(doc["content"]["backend"], "vault");
    assert_eq!(doc["content"]["encrypted"], false);

    let raw = raw_blob(&backends.internal, &id).await.unwrap();
    assert_eq!(raw, CONTENT, "public tier stores plaintext internally");
}

#[tokio::test]
async fn upload_is_refused_when_no_backend_may_hold_the_tier() {
    let server = external_only_server();
    let id = create_document(
        &server,
        "alice",
        json!({ "title": "secrets", "classification": "strictly_confidential" }),
    )
    .await;

    let (status, body) = upload(&server, "alice", &id, CONTENT).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("strictly_confidential")
    );

    // The document is untouched — no half-registered content.
    let (_, doc) = send(&server, "GET", &format!("/documents/{id}"), "alice", None).await;
    assert_eq!(doc["content"], Value::Null);
}

#[tokio::test]
async fn reclassification_moves_a_blob_the_new_tier_forbids() {
    let (server, backends) = server();
    let id = create_document(&server, "alice", json!({ "title": "draft deal" })).await;

    let (status, doc) = upload(&server, "alice", &id, CONTENT).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(doc["content"]["backend"], "cloud", "starts on external");

    let (status, doc) = send(
        &server,
        "PUT",
        &format!("/documents/{id}/classification"),
        "alice",
        Some(json!({ "to": "strictly_confidential", "comment": "deal went hostile" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    assert_eq!(doc["classification"], "strictly_confidential");

    // The blob moved within the same request — the placement invariant
    // never holds only eventually.
    assert_eq!(doc["content"]["backend"], "vault");
    assert_eq!(doc["content"]["location"], "internal");
    assert_eq!(doc["content"]["encrypted"], true);
    assert!(raw_blob(&backends.external, &id).await.is_none());
    assert!(raw_blob(&backends.internal, &id).await.is_some());

    // The change is on the record, attributed and explained.
    let history = doc["classification_history"].as_array().unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["from"], "internal");
    assert_eq!(history[0]["to"], "strictly_confidential");
    assert_eq!(history[0]["by"], "alice");
    assert_eq!(history[0]["comment"], "deal went hostile");

    let (status, body) = download(&server, "alice", &id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, CONTENT, "content survives the move intact");
}

#[tokio::test]
async fn reclassification_is_refused_entirely_when_content_cannot_move() {
    let server = external_only_server();
    let id = create_document(&server, "alice", json!({ "title": "notes" })).await;

    let (status, _) = upload(&server, "alice", &id, CONTENT).await;
    assert_eq!(status, StatusCode::OK);

    // Strictly confidential needs an internal backend; there is none. The
    // reclassification must fail as a whole — not change the tier and leave
    // the blob somewhere the new tier forbids.
    let (status, body) = send(
        &server,
        "PUT",
        &format!("/documents/{id}/classification"),
        "alice",
        Some(json!({ "to": "strictly_confidential" })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    let (_, doc) = send(&server, "GET", &format!("/documents/{id}"), "alice", None).await;
    assert_eq!(doc["classification"], "internal");
    assert_eq!(doc["classification_history"], json!([]));
    assert_eq!(doc["content"]["backend"], "cloud");
}

#[tokio::test]
async fn only_permitted_actors_reclassify() {
    let (server, _) = server();
    let id = create_document(&server, "alice", json!({ "title": "memo" })).await;

    // In use: anyone may view, only the owner classifies (baseline policy).
    let (status, _) = send(
        &server,
        "POST",
        &format!("/documents/{id}/status"),
        "alice",
        Some(json!({ "to": "in_use" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = send(
        &server,
        "PUT",
        &format!("/documents/{id}/classification"),
        "bob",
        Some(json!({ "to": "public" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    let (status, doc) = send(
        &server,
        "PUT",
        &format!("/documents/{id}/classification"),
        "alice",
        Some(json!({ "to": "public" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    assert_eq!(doc["classification"], "public");
}

#[tokio::test]
async fn reclassifying_to_the_same_tier_changes_nothing() {
    let (server, _) = server();
    let id = create_document(&server, "alice", json!({ "title": "memo" })).await;

    let (status, doc) = send(
        &server,
        "PUT",
        &format!("/documents/{id}/classification"),
        "alice",
        Some(json!({ "to": "internal" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(doc["classification_history"], json!([]), "no no-op noise");
}

#[tokio::test]
async fn placement_policy_is_administered_via_the_api() {
    let (server, _) = server();

    let (status, policy) = send(&server, "GET", "/policy/placement", "admin", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(policy["strictly_confidential"]["allow_external"], false);
    assert_eq!(policy["internal"]["allow_external"], true);

    // Tighten the policy: nothing may leave operator infrastructure.
    let mut tightened = policy.clone();
    for (_, rule) in tightened.as_object_mut().unwrap() {
        rule["allow_external"] = json!(false);
    }
    let (status, _) = send(
        &server,
        "PUT",
        "/policy/placement",
        "admin",
        Some(tightened),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Uploads now skip the (first-listed) external backend for every tier.
    let id = create_document(&server, "alice", json!({ "title": "anything" })).await;
    let (status, doc) = upload(&server, "alice", &id, CONTENT).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(doc["content"]["backend"], "vault");
}

#[tokio::test]
async fn destroying_a_document_destroys_its_blob() {
    let (server, backends) = server();
    let id = create_document(&server, "alice", json!({ "title": "old contract" })).await;

    let (status, _) = upload(&server, "alice", &id, CONTENT).await;
    assert_eq!(status, StatusCode::OK);
    assert!(raw_blob(&backends.external, &id).await.is_some());

    for to in ["in_use", "archived", "deletable"] {
        let (status, body) = send(
            &server,
            "POST",
            &format!("/documents/{id}/status"),
            "alice",
            Some(json!({ "to": to })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "transition to {to}: {body}");
    }

    let (status, certificate) = send(
        &server,
        "DELETE",
        &format!("/documents/{id}"),
        "alice",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{certificate}");
    assert_eq!(certificate["document"], id);

    // The certificate's claim is physically true: the bytes are gone.
    assert!(raw_blob(&backends.external, &id).await.is_none());
    assert!(raw_blob(&backends.internal, &id).await.is_none());
}

#[tokio::test]
async fn retention_sweep_destroys_blobs_of_auto_deleted_documents() {
    let (server, backends) = server();

    // Invoices auto-delete immediately on expiry.
    let (status, _) = send(
        &server,
        "PUT",
        "/retention/plan",
        "admin",
        Some(json!([
            { "doc_type": "invoice", "retain_for_days": 0, "on_expiry": "auto_delete" }
        ])),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let id = create_document(
        &server,
        "alice",
        json!({ "title": "invoice 17", "doc_type": "invoice" }),
    )
    .await;
    let (status, _) = upload(&server, "alice", &id, CONTENT).await;
    assert_eq!(status, StatusCode::OK);

    for to in ["in_use", "archived"] {
        let (status, _) = send(
            &server,
            "POST",
            &format!("/documents/{id}/status"),
            "alice",
            Some(json!({ "to": to })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    let (status, outcome) = send(&server, "POST", "/retention/sweep", "engine", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(outcome["deleted"].as_array().unwrap().len(), 1, "{outcome}");

    assert!(raw_blob(&backends.external, &id).await.is_none());
    assert!(raw_blob(&backends.internal, &id).await.is_none());
}
