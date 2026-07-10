//! End-to-end tests for the dossier ("E-Akte") API (STORE-09, STORE-10,
//! ACCESS-09).
//!
//! These drive the real router in-process, exactly like the status API
//! tests: placeholder-auth headers in, JSON out.

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

async fn create_dossier(server: &Router, user: &str, name: &str) -> Value {
    let (status, dossier) = send(
        server,
        "POST",
        "/dossiers",
        Some((user, "")),
        Some(json!({ "name": name })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    dossier
}

/// Create a document owned by `user` and move it to `in_use`, so that under
/// the baseline policy anyone may view (and therefore file) it.
async fn create_active_document(server: &Router, user: &str, title: &str) -> String {
    let (status, doc) = send(
        server,
        "POST",
        "/documents",
        Some((user, "")),
        Some(json!({ "title": title })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = doc["id"].as_str().unwrap().to_owned();

    let (status, _) = send(
        server,
        "POST",
        &format!("/documents/{id}/status"),
        Some((user, "")),
        Some(json!({ "to": "in_use" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    id
}

async fn add_document_entry(
    server: &Router,
    identity: (&str, &str),
    dossier_id: &str,
    document_id: &str,
) -> (StatusCode, Value) {
    send(
        server,
        "POST",
        &format!("/dossiers/{dossier_id}/entries"),
        Some(identity),
        Some(json!({ "reference": { "document": document_id } })),
    )
    .await
}

#[tokio::test]
async fn dossiers_start_private_to_their_creator_with_their_metadata() {
    let server = server();
    let (status, dossier) = send(
        &server,
        "POST",
        "/dossiers",
        Some(("alice", "")),
        Some(json!({
            "name": "Client X litigation",
            "metadata": { "client": "X GmbH", "case_number": "4711" }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(dossier["owner"], "alice");
    assert_eq!(dossier["metadata"]["case_number"], "4711");
    assert_eq!(dossier["acl"]["view"], json!(["owner"]));
    assert_eq!(dossier["entries"], json!([]));

    // Private: bob neither sees it listed nor can fetch it by ID.
    let id = dossier["id"].as_str().unwrap();
    let (_, list) = send(&server, "GET", "/dossiers", Some(("bob", "")), None).await;
    assert_eq!(list, json!([]));
    let (status, _) = send(
        &server,
        "GET",
        &format!("/dossiers/{id}"),
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "existence must not leak");
}

#[tokio::test]
async fn one_document_appears_in_many_dossiers_by_reference() {
    let server = server();
    let doc = create_active_document(&server, "alice", "framework contract").await;
    let file_a = create_dossier(&server, "alice", "Client A").await;
    let file_b = create_dossier(&server, "alice", "Client B").await;

    for dossier in [&file_a, &file_b] {
        let id = dossier["id"].as_str().unwrap();
        let (status, updated) = add_document_entry(&server, ("alice", ""), id, &doc).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(updated["entries"][0]["reference"]["document"], doc);
    }

    // Same reference twice in the same dossier is rejected — it is a
    // reference, not a copy.
    let id = file_a["id"].as_str().unwrap();
    let (status, body) = add_document_entry(&server, ("alice", ""), id, &doc).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body["error"].as_str().unwrap().contains("already filed"));

    // Removing the reference from one dossier leaves the other intact.
    let (_, current) = send(
        &server,
        "GET",
        &format!("/dossiers/{id}"),
        Some(("alice", "")),
        None,
    )
    .await;
    let entry_id = current["entries"][0]["id"].as_str().unwrap();
    let (status, _) = send(
        &server,
        "DELETE",
        &format!("/dossiers/{id}/entries/{entry_id}"),
        Some(("alice", "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let other_id = file_b["id"].as_str().unwrap();
    let (_, other) = send(
        &server,
        "GET",
        &format!("/dossiers/{other_id}"),
        Some(("alice", "")),
        None,
    )
    .await;
    assert_eq!(other["entries"].as_array().unwrap().len(), 1);
    let (_, doc_still_there) = send(
        &server,
        "GET",
        &format!("/documents/{doc}"),
        Some(("alice", "")),
        None,
    )
    .await;
    assert_eq!(doc_still_there["id"], doc, "document itself is untouched");
}

#[tokio::test]
async fn dossiers_reference_external_and_physical_records() {
    let server = server();
    let dossier = create_dossier(&server, "alice", "Property purchase").await;
    let id = dossier["id"].as_str().unwrap();

    let (status, updated) = send(
        &server,
        "POST",
        &format!("/dossiers/{id}/entries"),
        Some(("alice", "")),
        Some(json!({
            "reference": {
                "external": { "label": "notarized deed (paper)", "location": "safe, drawer 2" }
            },
            "note": "original must stay physical"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let entry = &updated["entries"][0];
    assert_eq!(
        entry["reference"]["external"]["label"],
        "notarized deed (paper)"
    );
    assert_eq!(entry["note"], "original must stay physical");
    assert_eq!(entry["added_by"], "alice");
}

#[tokio::test]
async fn documents_the_caller_cannot_view_cannot_be_filed() {
    let server = server();
    // Bob's draft is invisible to alice under the baseline policy.
    let (_, draft) = send(
        &server,
        "POST",
        "/documents",
        Some(("bob", "")),
        Some(json!({ "title": "bob's secret draft" })),
    )
    .await;
    let draft_id = draft["id"].as_str().unwrap();

    let dossier = create_dossier(&server, "alice", "Alice's file").await;
    let dossier_id = dossier["id"].as_str().unwrap();

    let (status, _) = add_document_entry(&server, ("alice", ""), dossier_id, draft_id).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "invisible documents 404");

    // A made-up ID fails the same way.
    let (status, _) = add_document_entry(
        &server,
        ("alice", ""),
        dossier_id,
        "00000000-0000-0000-0000-000000000000",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn dossier_metadata_is_user_extendable() {
    let server = server();
    let dossier = create_dossier(&server, "alice", "HR file Miller").await;
    let id = dossier["id"].as_str().unwrap();

    // Fetch, extend with a team-defined field, send back (CAPTURE-10).
    let (status, updated) = send(
        &server,
        "PATCH",
        &format!("/dossiers/{id}"),
        Some(("alice", "")),
        Some(json!({
            "metadata": { "department": "HR", "retention_class": "personnel" }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["metadata"]["retention_class"], "personnel");
    assert_eq!(updated["name"], "HR file Miller", "name untouched");

    let (status, renamed) = send(
        &server,
        "PATCH",
        &format!("/dossiers/{id}"),
        Some(("alice", "")),
        Some(json!({ "name": "HR file Miller, Jane" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(renamed["name"], "HR file Miller, Jane");
    assert_eq!(
        renamed["metadata"]["department"], "HR",
        "metadata untouched when only renaming"
    );
}

#[tokio::test]
async fn teams_administer_their_dossiers_through_the_acl() {
    let server = server();
    let dossier = create_dossier(&server, "alice", "Team space").await;
    let id = dossier["id"].as_str().unwrap();

    // Grant the accounting group view+edit; managing stays with alice.
    let (status, _) = send(
        &server,
        "PUT",
        &format!("/dossiers/{id}/acl"),
        Some(("alice", "")),
        Some(json!({
            "view": ["owner", { "group": "accounting" }],
            "edit": ["owner", { "group": "accounting" }],
            "manage": ["owner"]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Bob (accounting) now sees the dossier and may file into it.
    let doc = create_active_document(&server, "bob", "invoice 2026-031").await;
    let (status, _) = add_document_entry(&server, ("bob", "accounting"), id, &doc).await;
    assert_eq!(status, StatusCode::CREATED);

    // But bob may not administer permissions.
    let (status, _) = send(
        &server,
        "PUT",
        &format!("/dossiers/{id}/acl"),
        Some(("bob", "accounting")),
        Some(json!({ "view": ["anyone"], "edit": ["anyone"], "manage": ["anyone"] })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Carol (not in accounting) still sees nothing.
    let (status, _) = send(
        &server,
        "GET",
        &format!("/dossiers/{id}"),
        Some(("carol", "sales")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Viewers without edit rights exist too: tighten edit to the owner and
    // bob's next filing attempt fails while he can still read.
    let (_, _) = send(
        &server,
        "PUT",
        &format!("/dossiers/{id}/acl"),
        Some(("alice", "")),
        Some(json!({
            "view": ["owner", { "group": "accounting" }],
            "edit": ["owner"],
            "manage": ["owner"]
        })),
    )
    .await;
    let doc2 = create_active_document(&server, "bob", "invoice 2026-032").await;
    let (status, _) = add_document_entry(&server, ("bob", "accounting"), id, &doc2).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = send(
        &server,
        "GET",
        &format!("/dossiers/{id}"),
        Some(("bob", "accounting")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn per_entry_access_narrows_visibility_inside_a_shared_dossier() {
    let server = server();
    let dossier = create_dossier(&server, "alice", "Project Aurora").await;
    let id = dossier["id"].as_str().unwrap();

    // The whole team sees the dossier.
    send(
        &server,
        "PUT",
        &format!("/dossiers/{id}/acl"),
        Some(("alice", "")),
        Some(json!({
            "view": ["owner", { "group": "aurora" }],
            "edit": ["owner"],
            "manage": ["owner"]
        })),
    )
    .await;

    // File one open document and one restricted to bob (ACCESS-09:
    // deliberately small circle for sensitive records).
    let open_doc = create_active_document(&server, "alice", "project plan").await;
    add_document_entry(&server, ("alice", ""), id, &open_doc).await;

    let salary_doc = create_active_document(&server, "alice", "salary bands").await;
    let (status, updated) = send(
        &server,
        "POST",
        &format!("/dossiers/{id}/entries"),
        Some(("alice", "")),
        Some(json!({
            "reference": { "document": salary_doc },
            "access": ["owner", { "user": "bob" }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(updated["entries"].as_array().unwrap().len(), 2);

    // Bob sees both entries; carol sees the open one and a count of what
    // was withheld — but nothing about it.
    let (_, for_bob) = send(
        &server,
        "GET",
        &format!("/dossiers/{id}"),
        Some(("bob", "aurora")),
        None,
    )
    .await;
    assert_eq!(for_bob["entries"].as_array().unwrap().len(), 2);
    assert_eq!(for_bob["hidden_entries"], 0);

    let (_, for_carol) = send(
        &server,
        "GET",
        &format!("/dossiers/{id}"),
        Some(("carol", "aurora")),
        None,
    )
    .await;
    let carol_entries = for_carol["entries"].as_array().unwrap();
    assert_eq!(carol_entries.len(), 1);
    assert_eq!(carol_entries[0]["reference"]["document"], open_doc);
    assert_eq!(for_carol["hidden_entries"], 1);

    // The owner lifts the restriction; carol now sees both.
    let entry_id = updated["entries"][1]["id"].as_str().unwrap();
    let (status, _) = send(
        &server,
        "PUT",
        &format!("/dossiers/{id}/entries/{entry_id}/access"),
        Some(("alice", "")),
        Some(json!({ "access": null })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, for_carol) = send(
        &server,
        "GET",
        &format!("/dossiers/{id}"),
        Some(("carol", "aurora")),
        None,
    )
    .await;
    assert_eq!(for_carol["entries"].as_array().unwrap().len(), 2);
    assert_eq!(for_carol["hidden_entries"], 0);
}

#[tokio::test]
async fn entry_access_administration_requires_manage_rights() {
    let server = server();
    let dossier = create_dossier(&server, "alice", "Managed file").await;
    let id = dossier["id"].as_str().unwrap();
    send(
        &server,
        "PUT",
        &format!("/dossiers/{id}/acl"),
        Some(("alice", "")),
        Some(json!({
            "view": ["owner", { "user": "bob" }],
            "edit": ["owner", { "user": "bob" }],
            "manage": ["owner"]
        })),
    )
    .await;

    let doc = create_active_document(&server, "alice", "spec").await;
    let (_, updated) = add_document_entry(&server, ("alice", ""), id, &doc).await;
    let entry_id = updated["entries"][0]["id"].as_str().unwrap();

    // Bob can edit (file/remove) but not administer per-entry access.
    let (status, _) = send(
        &server,
        "PUT",
        &format!("/dossiers/{id}/entries/{entry_id}/access"),
        Some(("bob", "")),
        Some(json!({ "access": [{ "user": "bob" }] })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
