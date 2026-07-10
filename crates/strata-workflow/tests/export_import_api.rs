//! End-to-end tests for flow export and import (WORKFLOW-07).
//!
//! The round-trip guarantee is contractual: a flow exported from one engine
//! and imported into a second must export identically there and execute
//! with the same step trace. These tests drive two real routers in-process
//! to prove exactly that.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use strata_workflow::{AppState, app};
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

/// An invoice-routing flow: trigger → amount check → approval or filing.
fn invoice_flow() -> Value {
    json!({
        "name": "Incoming invoices",
        "owner": "accounting",
        "nodes": [
            { "id": "upload", "kind": "trigger", "config": { "source": "capture" } },
            { "id": "big?", "kind": "condition",
              "config": { "input": "amount", "operator": "greater_than", "value": 1000 } },
            { "id": "approve", "kind": "step", "config": { "action": "request_approval" } },
            { "id": "file", "kind": "step", "config": { "action": "move", "target": "invoices/" } }
        ],
        "edges": [
            { "from": "upload", "to": "big?" },
            { "from": "big?", "to": "approve", "branch": "true" },
            { "from": "big?", "to": "file", "branch": "false" }
        ]
    })
}

/// A second flow so multi-flow exports have something to order.
fn archive_flow() -> Value {
    json!({
        "name": "Archive sweep",
        "owner": "records",
        "nodes": [
            { "id": "due", "kind": "trigger", "config": { "source": "schedule" } },
            { "id": "archive", "kind": "step", "config": { "action": "archive" } }
        ],
        "edges": [ { "from": "due", "to": "archive" } ]
    })
}

async fn register_flow(server: &Router, definition: Value) -> Value {
    let (status, flow) = send(
        server,
        "POST",
        "/flows",
        Some(("alice", "")),
        Some(definition),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{flow}");
    flow
}

async fn export_all(server: &Router) -> Value {
    let (status, envelope) = send(server, "GET", "/flows/export", Some(("bob", "")), None).await;
    assert_eq!(status, StatusCode::OK, "{envelope}");
    envelope
}

async fn import(server: &Router, uri: &str, envelope: Value) -> (StatusCode, Value) {
    send(server, "POST", uri, Some(("bob", "")), Some(envelope)).await
}

async fn trigger_run(server: &Router, flow_id: &str, trigger: &str, input: Value) -> Value {
    let (status, run) = send(
        server,
        "POST",
        &format!("/flows/{flow_id}/runs"),
        Some(("carol", "")),
        Some(json!({ "trigger": trigger, "input": input })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{run}");
    run
}

/// The behavioral fingerprint of a run: everything the round-trip must
/// preserve (ids and timestamps legitimately differ between engines).
fn trace_fingerprint(run: &Value) -> Vec<Value> {
    run["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| json!([s["node"], s["kind"], s["config"], s["outcome"]]))
        .collect()
}

#[tokio::test]
async fn exports_wrap_flows_in_the_versioned_envelope() {
    let server = server();
    register_flow(&server, invoice_flow()).await;
    let flow = register_flow(&server, archive_flow()).await;

    let envelope = export_all(&server).await;
    assert_eq!(envelope["format"], "strata-flows");
    assert_eq!(envelope["version"], 1);
    let flows = envelope["flows"].as_array().unwrap();
    assert_eq!(flows.len(), 2);
    // Deterministic order: by name.
    assert_eq!(flows[0]["name"], "Archive sweep");
    assert_eq!(flows[1]["name"], "Incoming invoices");

    // A single-flow export uses the same envelope shape.
    let id = flow["id"].as_str().unwrap();
    let (status, single) = send(
        &server,
        "GET",
        &format!("/flows/{id}/export"),
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(single["format"], "strata-flows");
    assert_eq!(single["version"], 1);
    assert_eq!(single["flows"].as_array().unwrap().len(), 1);
    assert_eq!(single["flows"][0], flow);
}

#[tokio::test]
async fn a_round_trip_preserves_definitions_and_behavior() {
    // Engine A: the system being replaced.
    let source = server();
    let flow = register_flow(&source, invoice_flow()).await;
    let flow_id = flow["id"].as_str().unwrap().to_owned();
    register_flow(&source, archive_flow()).await;
    let exported = export_all(&source).await;

    // Engine B: a fresh installation.
    let target = server();
    let (status, report) = import(&target, "/flows/import", exported.clone()).await;
    assert_eq!(status, StatusCode::OK, "{report}");
    assert_eq!(report["imported"].as_array().unwrap().len(), 2);
    assert!(report["imported"].as_array().unwrap().contains(&flow["id"]));

    // Ids are preserved: the flow is addressable under its original id,
    // and re-exporting yields the identical document.
    let (status, fetched) = send(
        &target,
        "GET",
        &format!("/flows/{flow_id}"),
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched, flow);
    assert_eq!(export_all(&target).await, exported);

    // Identical behavior: same trigger, same input, same trace on both
    // engines — including the condition's decision on each branch.
    for input in [json!({"amount": 5000}), json!({"amount": 20})] {
        let original = trigger_run(&source, &flow_id, "upload", input.clone()).await;
        let reimported = trigger_run(&target, &flow_id, "upload", input).await;
        assert_eq!(original["status"], reimported["status"]);
        assert_eq!(trace_fingerprint(&original), trace_fingerprint(&reimported));
    }
}

#[tokio::test]
async fn imports_reject_foreign_formats_and_unknown_versions() {
    let server = server();

    let (status, body) = import(
        &server,
        "/flows/import",
        json!({ "format": "acme-flows", "version": 1, "flows": [] }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["error"].as_str().unwrap().contains("acme-flows"));

    let (status, body) = import(
        &server,
        "/flows/import",
        json!({ "format": "strata-flows", "version": 99, "flows": [] }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["error"].as_str().unwrap().contains("version"));
}

#[tokio::test]
async fn a_broken_flow_rejects_the_whole_import() {
    let source = server();
    register_flow(&source, invoice_flow()).await;
    let mut envelope = export_all(&source).await;

    // Corrupt the payload: a second flow with a dangling edge.
    envelope["flows"].as_array_mut().unwrap().push(json!({
        "id": uuid::Uuid::nil(),
        "name": "broken",
        "owner": "qa",
        "nodes": [ { "id": "start", "kind": "trigger" } ],
        "edges": [ { "from": "start", "to": "nowhere" } ]
    }));

    let target = server();
    let (status, body) = import(&target, "/flows/import", envelope).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["error"].as_str().unwrap().contains("nowhere"));

    // All-or-nothing: the valid flow was not imported either.
    let (_, all) = send(&target, "GET", "/flows", Some(("bob", "")), None).await;
    assert_eq!(all.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn duplicate_ids_within_one_envelope_are_rejected() {
    let source = server();
    register_flow(&source, invoice_flow()).await;
    let mut envelope = export_all(&source).await;
    let twin = envelope["flows"][0].clone();
    envelope["flows"].as_array_mut().unwrap().push(twin);

    let (status, body) = import(&server(), "/flows/import", envelope).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["error"].as_str().unwrap().contains("more than once"));
}

#[tokio::test]
async fn conflicting_imports_fail_replace_or_skip_explicitly() {
    let server = server();
    register_flow(&server, invoice_flow()).await;
    let mut envelope = export_all(&server).await;

    // Default: a conflicting id rejects the import and changes nothing.
    let (status, body) = import(&server, "/flows/import", envelope.clone()).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body["error"].as_str().unwrap().contains("on_conflict"));

    // Skip: the existing definition wins.
    envelope["flows"][0]["name"] = json!("Incoming invoices v2");
    let (status, report) =
        import(&server, "/flows/import?on_conflict=skip", envelope.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(report["skipped"].as_array().unwrap().len(), 1);
    assert_eq!(report["imported"].as_array().unwrap().len(), 0);
    let (_, all) = send(&server, "GET", "/flows", Some(("bob", "")), None).await;
    assert_eq!(all[0]["name"], "Incoming invoices");

    // Replace: the imported definition wins.
    let (status, report) = import(&server, "/flows/import?on_conflict=replace", envelope).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(report["replaced"].as_array().unwrap().len(), 1);
    let (_, all) = send(&server, "GET", "/flows", Some(("bob", "")), None).await;
    assert_eq!(all[0]["name"], "Incoming invoices v2");
}

#[tokio::test]
async fn export_and_import_require_identity() {
    let server = server();
    for (method, uri) in [
        ("GET", "/flows/export"),
        ("POST", "/flows/import"),
        ("GET", &*format!("/flows/{}/export", uuid::Uuid::nil())),
    ] {
        let (status, _) = send(&server, method, uri, None, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {uri}");
    }
}
