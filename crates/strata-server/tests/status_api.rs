//! End-to-end tests for the document status API (ACCESS-10).
//!
//! These drive the real router in-process: placeholder-auth headers in,
//! JSON out, permission and lifecycle rules enforced in between.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use strata_server::{AppState, app};
use tower::ServiceExt;

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

async fn create_document(server: &Router, user: &str, title: &str) -> Value {
    let (status, doc) = send(
        server,
        "POST",
        "/documents",
        Some((user, "")),
        Some(json!({ "title": title })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    doc
}

async fn transition(
    server: &Router,
    identity: (&str, &str),
    id: &str,
    to: &str,
) -> (StatusCode, Value) {
    send(
        server,
        "POST",
        &format!("/documents/{id}/status"),
        Some(identity),
        Some(json!({ "to": to })),
    )
    .await
}

#[tokio::test]
async fn documents_start_as_drafts_owned_by_their_creator() {
    let server = server();
    let doc = create_document(&server, "alice", "Q3 report").await;

    assert_eq!(doc["status"], "draft");
    assert_eq!(doc["owner"], "alice");
    assert_eq!(doc["title"], "Q3 report");
    assert_eq!(doc["history"], json!([]));
}

#[tokio::test]
async fn requests_without_identity_are_rejected() {
    let server = server();
    let (status, body) = send(&server, "GET", "/documents", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body["error"].as_str().unwrap().contains("x-strata-user"));
}

#[tokio::test]
async fn a_document_walks_the_full_lifecycle_and_keeps_its_history() {
    let server = server();
    let doc = create_document(&server, "alice", "contract").await;
    let id = doc["id"].as_str().unwrap().to_owned();

    for (to, step) in [("in_use", 1), ("archived", 2), ("deletable", 3)] {
        let (status, updated) = transition(&server, ("alice", ""), &id, to).await;
        assert_eq!(status, StatusCode::OK, "transition to {to}: {updated}");
        assert_eq!(updated["status"], to);
        assert_eq!(updated["history"].as_array().unwrap().len(), step);
    }

    let (_, doc) = send(
        &server,
        "GET",
        &format!("/documents/{id}"),
        Some(("alice", "")),
        None,
    )
    .await;
    let history = doc["history"].as_array().unwrap();
    assert_eq!(history[0]["from"], "draft");
    assert_eq!(history[2]["to"], "deletable");
    assert!(history.iter().all(|h| h["by"] == "alice"));
}

#[tokio::test]
async fn lifecycle_stages_cannot_be_skipped() {
    let server = server();
    let doc = create_document(&server, "alice", "memo").await;
    let id = doc["id"].as_str().unwrap();

    let (status, body) = transition(&server, ("alice", ""), id, "archived").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body["error"].as_str().unwrap().contains("draft"));
}

#[tokio::test]
async fn drafts_are_invisible_to_everyone_but_the_owner() {
    let server = server();
    let doc = create_document(&server, "alice", "secret draft").await;
    let id = doc["id"].as_str().unwrap();

    // Bob gets 404, not 403 — the API must not confirm the draft exists.
    let (status, _) = send(
        &server,
        "GET",
        &format!("/documents/{id}"),
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (_, list) = send(&server, "GET", "/documents", Some(("bob", "")), None).await;
    assert_eq!(list, json!([]));

    let (_, list) = send(&server, "GET", "/documents", Some(("alice", "")), None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn only_the_owner_may_move_a_document_between_statuses() {
    let server = server();
    let doc = create_document(&server, "alice", "shared doc").await;
    let id = doc["id"].as_str().unwrap();
    transition(&server, ("alice", ""), id, "in_use").await;

    // In use: bob may view it, but changing status stays with the owner.
    let (status, _) = send(
        &server,
        "GET",
        &format!("/documents/{id}"),
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = transition(&server, ("bob", ""), id, "archived").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn listing_filters_by_status() {
    let server = server();
    let draft = create_document(&server, "alice", "still writing").await;
    let active = create_document(&server, "alice", "in circulation").await;
    let active_id = active["id"].as_str().unwrap();
    transition(&server, ("alice", ""), active_id, "in_use").await;

    let (_, list) = send(
        &server,
        "GET",
        "/documents?status=draft",
        Some(("alice", "")),
        None,
    )
    .await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["id"], draft["id"]);

    let (_, list) = send(
        &server,
        "GET",
        "/documents?status=in_use",
        Some(("alice", "")),
        None,
    )
    .await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["id"], active["id"]);
}

#[tokio::test]
async fn permissions_are_reassignable_per_status_via_the_api() {
    let server = server();
    let doc = create_document(&server, "alice", "records file").await;
    let id = doc["id"].as_str().unwrap();
    transition(&server, ("alice", ""), id, "in_use").await;
    transition(&server, ("alice", ""), id, "archived").await;

    // Baseline: the records team may not release archived documents.
    let (status, _) = transition(&server, ("carol", "records"), id, "deletable").await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Fetch the live policy, hand `change_status` on archived documents to
    // the records group, and put it back.
    let (_, mut policy) = send(&server, "GET", "/policy/status", Some(("admin", "")), None).await;
    policy["archived"]["change_status"] = json!(["owner", { "group": "records" }]);
    let (status, _) = send(
        &server,
        "PUT",
        "/policy/status",
        Some(("admin", "")),
        Some(policy.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, current) = send(&server, "GET", "/policy/status", Some(("admin", "")), None).await;
    assert_eq!(current, policy);

    // The new rule takes effect immediately.
    let (status, updated) = transition(&server, ("carol", "records"), id, "deletable").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["status"], "deletable");
    assert_eq!(
        updated["history"].as_array().unwrap().last().unwrap()["by"],
        "carol"
    );
}

#[tokio::test]
async fn status_changes_land_on_the_event_feed_in_order() {
    let server = server();
    let doc = create_document(&server, "alice", "audited doc").await;
    let id = doc["id"].as_str().unwrap();
    transition(&server, ("alice", ""), id, "in_use").await;
    transition(&server, ("alice", ""), id, "archived").await;

    let (_, events) = send(
        &server,
        "GET",
        "/events/status",
        Some(("workflow", "")),
        None,
    )
    .await;
    let events = events.as_array().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["seq"], 1);
    assert_eq!(events[0]["from"], "draft");
    assert_eq!(events[1]["seq"], 2);
    assert_eq!(events[1]["to"], "archived");
    assert!(events.iter().all(|e| e["document"] == doc["id"]));

    // A consumer that has seen seq 1 only receives what came after.
    let (_, tail) = send(
        &server,
        "GET",
        "/events/status?after=1",
        Some(("workflow", "")),
        None,
    )
    .await;
    let tail = tail.as_array().unwrap();
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0]["seq"], 2);
}
