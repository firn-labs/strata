//! End-to-end tests for the retention and deletion engine
//! (PRESERVE-06, PRESERVE-07, PRESERVE-08).
//!
//! Time is controlled without a mock clock: a deadline in the past (or a
//! plan rule with `retain_for_days: 0`) is already expired, a deadline in
//! the distant future never expires within a test run.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use strata_server::{AppState, app};
use tower::ServiceExt;

const PAST: &str = "2000-01-01T00:00:00Z";
const FUTURE: &str = "2099-01-01T00:00:00Z";

fn server() -> Router {
    app(Arc::new(AppState::new()))
}

/// Send one request as `user` (with optional comma-separated groups) and
/// return status code plus parsed JSON body.
async fn send(
    server: &Router,
    method: &str,
    uri: &str,
    identity: Option<(&str, &str)>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder().method(method).uri(uri);
    if let Some((user, groups)) = identity {
        request = request.header("x-strata-user", user);
        if !groups.is_empty() {
            request = request.header("x-strata-groups", groups);
        }
    }
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

/// Create a document for `user` and return its id.
async fn create_document(server: &Router, user: &str, body: Value) -> String {
    let (status, doc) = send(server, "POST", "/documents", Some((user, "")), Some(body)).await;
    assert_eq!(status, StatusCode::CREATED);
    doc["id"].as_str().unwrap().to_owned()
}

/// Walk a document forward from wherever it currently is into `target`
/// status, as its owner.
async fn advance(server: &Router, user: &str, id: &str, target: &str) {
    const CHAIN: [&str; 4] = ["draft", "in_use", "archived", "deletable"];
    let (status, doc) = send(
        server,
        "GET",
        &format!("/documents/{id}"),
        Some((user, "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{doc}");

    let position = |s: &str| CHAIN.iter().position(|c| *c == s).unwrap();
    let current = position(doc["status"].as_str().unwrap());
    for to in &CHAIN[current + 1..=position(target)] {
        let (status, body) = send(
            server,
            "POST",
            &format!("/documents/{id}/status"),
            Some((user, "")),
            Some(json!({ "to": to })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "transition to {to}: {body}");
    }
}

async fn set_deadline(
    server: &Router,
    user: &str,
    id: &str,
    delete_after: &str,
) -> (StatusCode, Value) {
    send(
        server,
        "PUT",
        &format!("/documents/{id}/retention"),
        Some((user, "")),
        Some(json!({ "delete_after": delete_after })),
    )
    .await
}

#[tokio::test]
async fn deadlines_cannot_be_set_before_archive_time() {
    let server = server();
    let id = create_document(&server, "alice", json!({ "title": "contract" })).await;

    // Draft and in-use: too early — the retention clock starts at archiving.
    let (status, body) = set_deadline(&server, "alice", &id, FUTURE).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["error"].as_str().unwrap().contains("archive"));

    advance(&server, "alice", &id, "in_use").await;
    let (status, _) = set_deadline(&server, "alice", &id, FUTURE).await;
    assert_eq!(status, StatusCode::CONFLICT);

    // Archived: allowed, recorded as an explicit deadline.
    advance(&server, "alice", &id, "archived").await;
    let (status, doc) = set_deadline(&server, "alice", &id, FUTURE).await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    assert_eq!(doc["retention"]["source"], "explicit");
    assert_eq!(doc["retention"]["set_by"], "alice");
}

#[tokio::test]
async fn deletion_is_blocked_until_the_deadline_and_certified_after() {
    let server = server();
    let id = create_document(&server, "alice", json!({ "title": "tax records" })).await;
    advance(&server, "alice", &id, "archived").await;
    set_deadline(&server, "alice", &id, FUTURE).await;
    advance(&server, "alice", &id, "deletable").await;

    // Deletable status alone is not enough: the deadline still blocks.
    let (status, body) = send(
        &server,
        "DELETE",
        &format!("/documents/{id}"),
        Some(("alice", "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["error"].as_str().unwrap().contains("blocked"));

    // Once the deadline lies in the past, deletion goes through and returns
    // the certificate (PRESERVE-08).
    set_deadline(&server, "alice", &id, PAST).await;
    let (status, certificate) = send(
        &server,
        "DELETE",
        &format!("/documents/{id}"),
        Some(("alice", "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{certificate}");
    assert_eq!(certificate["document"], id);
    assert_eq!(certificate["deleted_by"], "alice");
    assert_eq!(certificate["trigger"], "manual");

    // The document is gone; the certificate lives on in the history.
    let (status, _) = send(
        &server,
        "GET",
        &format!("/documents/{id}"),
        Some(("alice", "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (_, history) = send(
        &server,
        "GET",
        "/retention/deletions",
        Some(("auditor", "")),
        None,
    )
    .await;
    assert_eq!(history.as_array().unwrap().len(), 1);
    assert_eq!(history[0]["id"], certificate["id"]);
}

#[tokio::test]
async fn archiving_applies_the_standard_deadline_from_the_retention_plan() {
    let server = server();

    // Standard deadlines per document type and team (PRESERVE-06): the
    // type+team rule is more specific than the type-only rule and wins.
    let (status, _) = send(
        &server,
        "PUT",
        "/retention/plan",
        Some(("admin", "")),
        Some(json!([
            {
                "doc_type": "invoice",
                "team": "accounting",
                "retain_for_days": 0,
                "on_expiry": "auto_delete",
            },
            { "doc_type": "invoice", "retain_for_days": 36500, "on_expiry": "notify_responsible" },
        ])),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let short = create_document(
        &server,
        "alice",
        json!({ "title": "Q1 invoice", "doc_type": "invoice", "team": "accounting" }),
    )
    .await;
    let long = create_document(
        &server,
        "alice",
        json!({ "title": "sales invoice", "doc_type": "invoice", "team": "sales" }),
    )
    .await;
    let unruled = create_document(&server, "alice", json!({ "title": "memo" })).await;

    advance(&server, "alice", &short, "archived").await;
    advance(&server, "alice", &long, "archived").await;
    advance(&server, "alice", &unruled, "archived").await;

    let doc = |id: &str| format!("/documents/{id}");
    let (_, short_doc) = send(&server, "GET", &doc(&short), Some(("alice", "")), None).await;
    let (_, long_doc) = send(&server, "GET", &doc(&long), Some(("alice", "")), None).await;
    let (_, unruled_doc) = send(&server, "GET", &doc(&unruled), Some(("alice", "")), None).await;

    // A zero-day retention period expires the moment it starts.
    assert_eq!(short_doc["retention"]["source"], "plan");
    assert_eq!(
        short_doc["retention"]["delete_after"],
        short_doc["retention"]["set_at"]
    );

    // The type-only rule matched the sales invoice: 100 years out.
    assert_eq!(long_doc["retention"]["source"], "plan");
    assert_ne!(
        long_doc["retention"]["delete_after"],
        long_doc["retention"]["set_at"]
    );

    // No matching rule → no deadline; nothing invented (and nothing to ever
    // block deletion of the memo).
    assert_eq!(unruled_doc["retention"], Value::Null);
}

#[tokio::test]
async fn explicit_deadlines_survive_reactivation_and_are_not_overwritten_by_the_plan() {
    let server = server();
    send(
        &server,
        "PUT",
        "/retention/plan",
        Some(("admin", "")),
        Some(json!([{ "retain_for_days": 30, "on_expiry": "notify_responsible" }])),
    )
    .await;

    let id = create_document(&server, "alice", json!({ "title": "agreement" })).await;
    advance(&server, "alice", &id, "archived").await;
    let (status, doc) = set_deadline(&server, "alice", &id, FUTURE).await;
    assert_eq!(status, StatusCode::OK, "{doc}");

    // Reactivate and archive again: the explicit deadline stays in force.
    for to in ["in_use", "archived"] {
        let (status, _) = send(
            &server,
            "POST",
            &format!("/documents/{id}/status"),
            Some(("alice", "")),
            Some(json!({ "to": to })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    let (_, doc) = send(
        &server,
        "GET",
        &format!("/documents/{id}"),
        Some(("alice", "")),
        None,
    )
    .await;
    assert_eq!(doc["retention"]["source"], "explicit");
    assert_eq!(
        doc["retention"]["delete_after"].as_str().unwrap(),
        "2099-01-01T00:00:00Z"
    );
}

#[tokio::test]
async fn the_sweep_deletes_or_notifies_per_document_class() {
    let server = server();
    send(
        &server,
        "PUT",
        "/retention/plan",
        Some(("admin", "")),
        Some(json!([
            { "doc_type": "log", "retain_for_days": 0, "on_expiry": "auto_delete" },
            { "doc_type": "contract", "retain_for_days": 0, "on_expiry": "notify_responsible" },
        ])),
    )
    .await;

    let log = create_document(
        &server,
        "alice",
        json!({ "title": "access log", "doc_type": "log" }),
    )
    .await;
    let contract = create_document(
        &server,
        "bob",
        json!({ "title": "supplier contract", "doc_type": "contract" }),
    )
    .await;
    advance(&server, "alice", &log, "archived").await;
    advance(&server, "bob", &contract, "archived").await;

    let (status, outcome) = send(
        &server,
        "POST",
        "/retention/sweep",
        Some(("retention-engine", "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{outcome}");

    // The log's class says auto-delete: gone, with a certificate.
    let deleted = outcome["deleted"].as_array().unwrap();
    assert_eq!(deleted.len(), 1);
    assert_eq!(deleted[0]["document"], log);
    assert_eq!(deleted[0]["trigger"], "retention_expiry");
    assert_eq!(deleted[0]["deleted_by"], "retention-engine");
    let (status, _) = send(
        &server,
        "GET",
        &format!("/documents/{log}"),
        Some(("alice", "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The contract's class says notify: the responsible person is told, the
    // document survives.
    let notified = outcome["notified"].as_array().unwrap();
    assert_eq!(notified.len(), 1);
    assert_eq!(notified[0]["document"], contract);
    assert_eq!(notified[0]["responsible"], "bob");
    let (status, _) = send(
        &server,
        "GET",
        &format!("/documents/{contract}"),
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, notifications) = send(
        &server,
        "GET",
        "/retention/notifications",
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(notifications.as_array().unwrap().len(), 1);

    // A second run finds nothing new: the deletion already happened, the
    // notification is not repeated.
    let (_, outcome) = send(
        &server,
        "POST",
        "/retention/sweep",
        Some(("retention-engine", "")),
        None,
    )
    .await;
    assert_eq!(outcome["deleted"], json!([]));
    assert_eq!(outcome["notified"], json!([]));

    // Both certified deletions (none) and the sweep's one deletion are in
    // the permanent history.
    let (_, history) = send(
        &server,
        "GET",
        "/retention/deletions",
        Some(("auditor", "")),
        None,
    )
    .await;
    assert_eq!(history.as_array().unwrap().len(), 1);
    assert_eq!(history[0]["document"], log);
}

#[tokio::test]
async fn expired_documents_without_a_plan_rule_are_notified_never_auto_deleted() {
    let server = server();
    let id = create_document(&server, "alice", json!({ "title": "orphan" })).await;
    advance(&server, "alice", &id, "archived").await;
    set_deadline(&server, "alice", &id, PAST).await;

    let (_, outcome) = send(
        &server,
        "POST",
        "/retention/sweep",
        Some(("retention-engine", "")),
        None,
    )
    .await;
    assert_eq!(outcome["deleted"], json!([]));
    assert_eq!(outcome["notified"][0]["document"], id.as_str());
}

#[tokio::test]
async fn reactivated_documents_are_left_alone_by_the_sweep() {
    let server = server();
    send(
        &server,
        "PUT",
        "/retention/plan",
        Some(("admin", "")),
        Some(json!([{ "retain_for_days": 0, "on_expiry": "auto_delete" }])),
    )
    .await;

    let id = create_document(&server, "alice", json!({ "title": "back in use" })).await;
    advance(&server, "alice", &id, "archived").await;
    let (status, _) = send(
        &server,
        "POST",
        &format!("/documents/{id}/status"),
        Some(("alice", "")),
        Some(json!({ "to": "in_use" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, outcome) = send(
        &server,
        "POST",
        "/retention/sweep",
        Some(("retention-engine", "")),
        None,
    )
    .await;
    assert_eq!(outcome["deleted"], json!([]));
    assert_eq!(outcome["notified"], json!([]));
}

#[tokio::test]
async fn deletion_permissions_follow_the_status_policy() {
    let server = server();
    let id = create_document(&server, "alice", json!({ "title": "guarded" })).await;
    advance(&server, "alice", &id, "archived").await;

    // Baseline grants `delete` in no status but deletable — not even the
    // owner deletes an archived document.
    let (status, _) = send(
        &server,
        "DELETE",
        &format!("/documents/{id}"),
        Some(("alice", "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Deletable documents are invisible to non-owners: bob gets 404, and
    // certainly cannot delete.
    advance(&server, "alice", &id, "deletable").await;
    let (status, _) = send(
        &server,
        "DELETE",
        &format!("/documents/{id}"),
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn deleting_a_document_purges_its_dossier_references() {
    let server = server();
    let id = create_document(&server, "alice", json!({ "title": "filed doc" })).await;
    advance(&server, "alice", &id, "in_use").await;

    let (_, dossier) = send(
        &server,
        "POST",
        "/dossiers",
        Some(("alice", "")),
        Some(json!({ "name": "case file" })),
    )
    .await;
    let dossier_id = dossier["id"].as_str().unwrap().to_owned();
    let (status, _) = send(
        &server,
        "POST",
        &format!("/dossiers/{dossier_id}/entries"),
        Some(("alice", "")),
        Some(json!({ "reference": { "document": id } })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    advance(&server, "alice", &id, "deletable").await;
    let (status, _) = send(
        &server,
        "DELETE",
        &format!("/documents/{id}"),
        Some(("alice", "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // The dossier no longer references the destroyed document.
    let (_, dossier) = send(
        &server,
        "GET",
        &format!("/dossiers/{dossier_id}"),
        Some(("alice", "")),
        None,
    )
    .await;
    assert_eq!(dossier["entries"], json!([]));
}
