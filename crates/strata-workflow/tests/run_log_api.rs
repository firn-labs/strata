//! End-to-end tests for the workflow run log (WORKFLOW-05).
//!
//! These drive the real router in-process: register a flow, trigger runs,
//! and read the step-by-step trace back through the query API.

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

async fn trigger_run(
    server: &Router,
    user: &str,
    flow_id: &str,
    input: Value,
) -> (StatusCode, Value) {
    send(
        server,
        "POST",
        &format!("/flows/{flow_id}/runs"),
        Some((user, "")),
        Some(json!({ "trigger": "upload", "input": input })),
    )
    .await
}

#[tokio::test]
async fn requests_without_identity_are_rejected() {
    let server = server();
    let (status, body) = send(&server, "GET", "/flows", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body["error"].as_str().unwrap().contains("x-strata-user"));
}

#[tokio::test]
async fn registered_flows_can_be_listed_and_fetched() {
    let server = server();
    let flow = register_flow(&server, invoice_flow()).await;
    let id = flow["id"].as_str().unwrap();

    let (status, all) = send(&server, "GET", "/flows", Some(("bob", "")), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(all.as_array().unwrap().len(), 1);

    let (status, fetched) = send(
        &server,
        "GET",
        &format!("/flows/{id}"),
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched["name"], "Incoming invoices");
    assert_eq!(fetched["nodes"].as_array().unwrap().len(), 4);
}

#[tokio::test]
async fn structurally_broken_flows_are_rejected_at_registration() {
    let server = server();
    let (status, body) = send(
        &server,
        "POST",
        "/flows",
        Some(("alice", "")),
        Some(json!({
            "name": "broken",
            "nodes": [{ "id": "lonely", "kind": "step" }],
            "edges": [{ "from": "lonely", "to": "nowhere" }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["error"].as_str().unwrap().contains("trigger"));
}

#[tokio::test]
async fn a_run_records_trigger_inputs_decisions_outcomes_and_timestamps() {
    let server = server();
    let flow = register_flow(&server, invoice_flow()).await;
    let flow_id = flow["id"].as_str().unwrap();

    let (status, run) = trigger_run(&server, "carol", flow_id, json!({"amount": 5000})).await;
    assert_eq!(status, StatusCode::CREATED, "{run}");

    assert_eq!(run["status"], "completed");
    assert_eq!(run["triggered_by"], "carol");
    assert_eq!(run["trigger_node"], "upload");
    assert_eq!(run["input"], json!({"amount": 5000}));

    let steps = run["steps"].as_array().unwrap();
    let visited: Vec<_> = steps.iter().map(|s| s["node"].as_str().unwrap()).collect();
    assert_eq!(visited, ["upload", "big?", "approve"]);

    assert_eq!(steps[0]["outcome"]["result"], "triggered");
    assert_eq!(steps[1]["outcome"]["result"], "decided");
    assert_eq!(steps[1]["outcome"]["branch"], "true");
    assert!(
        steps[1]["outcome"]["reason"]
            .as_str()
            .unwrap()
            .contains("amount")
    );
    assert_eq!(steps[2]["outcome"]["result"], "executed");

    for step in steps {
        assert!(step["started_at"].is_string());
        assert!(step["finished_at"].is_string());
    }
    assert!(run["started_at"].is_string());
    assert!(run["finished_at"].is_string());
}

#[tokio::test]
async fn the_small_invoice_takes_the_other_branch() {
    let server = server();
    let flow = register_flow(&server, invoice_flow()).await;
    let flow_id = flow["id"].as_str().unwrap();

    let (_, run) = trigger_run(&server, "carol", flow_id, json!({"amount": 20})).await;
    let visited: Vec<_> = run["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["node"].as_str().unwrap())
        .collect();
    assert_eq!(visited, ["upload", "big?", "file"]);
}

#[tokio::test]
async fn a_failing_run_is_traced_up_to_the_failing_step() {
    let server = server();
    let flow = register_flow(&server, invoice_flow()).await;
    let flow_id = flow["id"].as_str().unwrap();

    // No `amount` in the input: the condition cannot decide.
    let (status, run) = trigger_run(&server, "carol", flow_id, json!({"vendor": "acme"})).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(run["status"], "failed");
    assert!(run["error"].as_str().unwrap().contains("big?"));

    let steps = run["steps"].as_array().unwrap();
    let last = steps.last().unwrap();
    assert_eq!(last["node"], "big?");
    assert_eq!(last["outcome"]["result"], "failed");
    assert!(
        last["outcome"]["error"]
            .as_str()
            .unwrap()
            .contains("amount")
    );
}

#[tokio::test]
async fn run_history_is_queryable_per_flow_and_filterable_by_status() {
    let server = server();
    let flow = register_flow(&server, invoice_flow()).await;
    let flow_id = flow["id"].as_str().unwrap();

    trigger_run(&server, "carol", flow_id, json!({"amount": 5000})).await;
    trigger_run(&server, "dave", flow_id, json!({"amount": 20})).await;
    trigger_run(&server, "erin", flow_id, json!({})).await; // fails

    let (status, all) = send(
        &server,
        "GET",
        &format!("/flows/{flow_id}/runs"),
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let all = all.as_array().unwrap();
    assert_eq!(all.len(), 3);
    // Chronological order, summaries carry who/when/how many steps.
    assert_eq!(all[0]["triggered_by"], "carol");
    assert_eq!(all[2]["status"], "failed");
    // Summaries carry a step count, not the trace itself.
    assert_eq!(all[0]["steps"], 3);

    let (_, failed) = send(
        &server,
        "GET",
        &format!("/flows/{flow_id}/runs?status=failed"),
        Some(("bob", "")),
        None,
    )
    .await;
    let failed = failed.as_array().unwrap();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0]["triggered_by"], "erin");
}

#[tokio::test]
async fn a_single_run_is_retrievable_with_its_full_trace() {
    let server = server();
    let flow = register_flow(&server, invoice_flow()).await;
    let flow_id = flow["id"].as_str().unwrap();

    let (_, run) = trigger_run(&server, "carol", flow_id, json!({"amount": 5000})).await;
    let run_id = run["id"].as_str().unwrap();

    let (status, fetched) = send(
        &server,
        "GET",
        &format!("/runs/{run_id}"),
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched["id"], run["id"]);
    assert_eq!(fetched["steps"].as_array().unwrap().len(), 3);

    let (status, body) = send(
        &server,
        "GET",
        &format!("/runs/{}", uuid::Uuid::nil()),
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body["error"].as_str().unwrap().contains("not found"));
}

#[tokio::test]
async fn only_trigger_nodes_start_runs() {
    let server = server();
    let flow = register_flow(&server, invoice_flow()).await;
    let flow_id = flow["id"].as_str().unwrap();

    let (status, body) = send(
        &server,
        "POST",
        &format!("/flows/{flow_id}/runs"),
        Some(("carol", "")),
        Some(json!({ "trigger": "file", "input": {} })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("only trigger nodes")
    );

    let (status, body) = send(
        &server,
        "POST",
        &format!("/flows/{flow_id}/runs"),
        Some(("carol", "")),
        Some(json!({ "trigger": "ghost", "input": {} })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["error"].as_str().unwrap().contains("no node"));
}

#[tokio::test]
async fn runs_against_unknown_flows_are_not_found() {
    let server = server();
    let (status, _) = send(
        &server,
        "POST",
        &format!("/flows/{}/runs", uuid::Uuid::nil()),
        Some(("carol", "")),
        Some(json!({ "trigger": "upload" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
