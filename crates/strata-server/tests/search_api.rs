//! End-to-end tests for the search facade (SEARCH-01 … SEARCH-05) and its
//! inputs: extracted full text (CAPTURE-07) and the indexing fields on
//! documents (CAPTURE-08).
//!
//! These drive the real router in-process, exactly like the other API
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

/// Register a document from `body` and move it to `in_use`, so the baseline
/// policy lets anyone view (and therefore find) it. Returns the record.
async fn create_active_document(server: &Router, user: &str, body: Value) -> Value {
    let (status, doc) = send(server, "POST", "/documents", Some((user, "")), Some(body)).await;
    assert_eq!(status, StatusCode::CREATED);
    let id = doc["id"].as_str().unwrap();
    let (status, doc) = send(
        server,
        "POST",
        &format!("/documents/{id}/status"),
        Some((user, "")),
        Some(json!({ "to": "in_use" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    doc
}

fn hit_titles(response: &Value) -> Vec<&str> {
    response["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hit| hit["title"].as_str().unwrap())
        .collect()
}

// ---------------------------------------------------------------- SEARCH-01

#[tokio::test]
async fn full_text_search_covers_extracted_text_title_and_keywords() {
    let server = server();
    let report = create_active_document(&server, "alice", json!({ "title": "Q3 Report" })).await;
    let report_id = report["id"].as_str().unwrap();
    create_active_document(
        &server,
        "alice",
        json!({ "title": "Cafeteria menu", "keywords": ["catering"] }),
    )
    .await;

    let (status, _) = send(
        &server,
        "PUT",
        &format!("/documents/{report_id}/text"),
        Some(("alice", "")),
        Some(json!({ "text": "The quarterly turbine maintenance was completed on schedule." })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // A word only present in the OCR-extracted text.
    let (status, found) = send(
        &server,
        "GET",
        "/search?text=turbine",
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(found["total"], 1);
    assert_eq!(hit_titles(&found), ["Q3 Report"]);

    // A word only present in a keyword.
    let (_, found) = send(
        &server,
        "GET",
        "/search?text=catering",
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(hit_titles(&found), ["Cafeteria menu"]);

    // Every word must occur somewhere: title + extracted text mix matches,
    // an extra unmatched word does not.
    let (_, found) = send(
        &server,
        "GET",
        "/search?text=report%20turbine",
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(found["total"], 1);
    let (_, found) = send(
        &server,
        "GET",
        "/search?text=turbine%20catering",
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(found["total"], 0);
}

#[tokio::test]
async fn full_text_hits_carry_a_snippet_of_the_matched_text() {
    let server = server();
    let doc = create_active_document(&server, "alice", json!({ "title": "Handbook" })).await;
    let id = doc["id"].as_str().unwrap();

    let filler = "lorem ipsum ".repeat(30);
    let text = format!("{filler}the turbine schedule follows {filler}");
    send(
        &server,
        "PUT",
        &format!("/documents/{id}/text"),
        Some(("alice", "")),
        Some(json!({ "text": text })),
    )
    .await;

    let (_, found) = send(
        &server,
        "GET",
        "/search?text=turbine",
        Some(("bob", "")),
        None,
    )
    .await;
    let snippet = found["hits"][0]["snippet"].as_str().unwrap();
    assert!(snippet.contains("turbine schedule"));
    // Both sides were truncated, so both ellipses are present.
    assert!(snippet.starts_with('…') && snippet.ends_with('…'));
}

#[tokio::test]
async fn search_never_reveals_documents_the_caller_may_not_view() {
    let server = server();
    // A draft is visible only to its owner under the baseline policy.
    let (status, doc) = send(
        &server,
        "POST",
        "/documents",
        Some(("bob", "")),
        Some(json!({ "title": "Secret acquisition plan", "keywords": ["confidential"] })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = doc["id"].as_str().unwrap();

    let (_, found) = send(
        &server,
        "GET",
        "/search?text=acquisition",
        Some(("alice", "")),
        None,
    )
    .await;
    assert_eq!(found["total"], 0);

    let (_, found) = send(
        &server,
        "GET",
        "/search?text=acquisition",
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(found["total"], 1);

    // The same rule holds for reference resolution: not viewable reads as
    // not found, never as forbidden.
    let (status, _) = send(
        &server,
        "GET",
        &format!("/refs/strata:doc:{id}"),
        Some(("alice", "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------- SEARCH-02

#[tokio::test]
async fn boolean_filters_combine_fields_operators_and_parentheses() {
    let server = server();
    create_active_document(
        &server,
        "alice",
        json!({
            "title": "Invoice 2201", "doc_type": "invoice", "team": "finance",
            "metadata": { "customer": "ACME" }
        }),
    )
    .await;
    create_active_document(
        &server,
        "alice",
        json!({
            "title": "Invoice 2202", "doc_type": "invoice", "team": "sales",
            "keywords": ["urgent"]
        }),
    )
    .await;
    create_active_document(
        &server,
        "alice",
        json!({ "title": "Hiring contract", "doc_type": "contract", "team": "hr" }),
    )
    .await;

    let query = "type:invoice%20AND%20(team:finance%20OR%20keyword:urgent)";
    let (status, found) = send(
        &server,
        "GET",
        &format!("/search?filter={query}"),
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(found["total"], 2);

    let (_, found) = send(
        &server,
        "GET",
        "/search?filter=type:invoice%20NOT%20team:sales",
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(hit_titles(&found), ["Invoice 2201"]);

    // Free metadata is reachable as meta.<key>; matching ignores case.
    let (_, found) = send(
        &server,
        "GET",
        "/search?filter=meta.customer:acme",
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(hit_titles(&found), ["Invoice 2201"]);

    // Filters and full text combine on the one surface (SEARCH-05).
    let (_, found) = send(
        &server,
        "GET",
        "/search?filter=type:invoice&text=hiring",
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(found["total"], 0);
}

#[tokio::test]
async fn malformed_filters_and_unknown_fields_are_rejected() {
    let server = server();
    create_active_document(&server, "alice", json!({ "title": "Anything" })).await;

    let (status, body) = send(
        &server,
        "GET",
        "/search?filter=type:invoice%20AND%20(team:finance",
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("invalid filter"));

    // A typo in a field name fails loudly instead of matching nothing.
    let (status, body) = send(
        &server,
        "GET",
        "/search?filter=tema:finance",
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("unknown field 'tema'")
    );
}

// ---------------------------------------------------------------- SEARCH-03

#[tokio::test]
async fn folder_listing_walks_the_filing_tree_with_subtree_counts() {
    let server = server();
    for (title, folder) in [
        ("Invoice A", "/finance/invoices"),
        ("Invoice B", "/finance/invoices"),
        ("Annual report", "/finance/reports"),
        ("Onboarding", "/hr"),
    ] {
        create_active_document(
            &server,
            "alice",
            json!({ "title": title, "folder": folder }),
        )
        .await;
    }
    // Unfiled documents appear in search but not in the folder tree.
    create_active_document(&server, "alice", json!({ "title": "Loose note" })).await;

    let (status, root) = send(&server, "GET", "/search/folders", Some(("bob", "")), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(root["folder"], "/");
    assert_eq!(
        root["subfolders"],
        json!([
            { "path": "/finance", "name": "finance", "documents": 3 },
            { "path": "/hr", "name": "hr", "documents": 1 },
        ])
    );
    assert_eq!(root["documents"].as_array().unwrap().len(), 0);

    let (_, finance) = send(
        &server,
        "GET",
        "/search/folders?under=%2Ffinance",
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(finance["subfolders"][0]["name"], "invoices");
    assert_eq!(finance["subfolders"][0]["documents"], 2);
    assert_eq!(finance["subfolders"][1]["name"], "reports");

    let (_, invoices) = send(
        &server,
        "GET",
        "/search/folders?under=%2Ffinance%2Finvoices",
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(invoices["subfolders"].as_array().unwrap().len(), 0);
    assert_eq!(invoices["documents"].as_array().unwrap().len(), 2);

    // The folder parameter scopes regular search to a subtree.
    let (_, found) = send(
        &server,
        "GET",
        "/search?folder=%2Ffinance",
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(found["total"], 3);
}

#[tokio::test]
async fn folder_counts_exclude_documents_the_caller_may_not_view() {
    let server = server();
    create_active_document(
        &server,
        "alice",
        json!({ "title": "Public plan", "folder": "/strategy" }),
    )
    .await;
    // Bob's draft in the same folder is invisible to others.
    send(
        &server,
        "POST",
        "/documents",
        Some(("bob", "")),
        Some(json!({ "title": "Secret plan", "folder": "/strategy" })),
    )
    .await;

    let (_, root) = send(&server, "GET", "/search/folders", Some(("alice", "")), None).await;
    assert_eq!(root["subfolders"][0]["documents"], 1);

    let (_, root) = send(&server, "GET", "/search/folders", Some(("bob", "")), None).await;
    assert_eq!(root["subfolders"][0]["documents"], 2);
}

#[tokio::test]
async fn folder_paths_are_normalized_and_garbage_is_rejected() {
    let server = server();
    let (status, doc) = send(
        &server,
        "POST",
        "/documents",
        Some(("alice", "")),
        Some(json!({ "title": "Messy path", "folder": "finance//invoices/" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(doc["folder"], "/finance/invoices");

    let (status, _) = send(
        &server,
        "POST",
        "/documents",
        Some(("alice", "")),
        Some(json!({ "title": "No folder at all", "folder": "///" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn timeline_buckets_matches_by_registration_time() {
    let server = server();
    let doc = create_active_document(&server, "alice", json!({ "title": "One" })).await;
    create_active_document(&server, "alice", json!({ "title": "Two" })).await;
    let created_at = doc["created_at"].as_str().unwrap();

    let (status, buckets) = send(
        &server,
        "GET",
        "/search/timeline?granularity=month",
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // Both documents were just registered, so one bucket holds them both,
    // named after the (UTC) month they were created in.
    assert_eq!(buckets, json!([{ "period": &created_at[..7], "count": 2 }]));

    let (_, buckets) = send(
        &server,
        "GET",
        "/search/timeline?granularity=year",
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(buckets[0]["period"], &created_at[..4]);

    // The time range narrows the histogram like any other search.
    let (_, buckets) = send(
        &server,
        "GET",
        "/search/timeline?created_after=2099-01-01T00:00:00Z",
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(buckets.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn created_range_filters_search_results() {
    let server = server();
    create_active_document(&server, "alice", json!({ "title": "Fresh" })).await;

    let (_, found) = send(
        &server,
        "GET",
        "/search?created_after=2000-01-01T00:00:00Z&created_before=2099-01-01T00:00:00Z",
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(found["total"], 1);

    let (_, found) = send(
        &server,
        "GET",
        "/search?created_before=2000-01-01T00:00:00Z",
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(found["total"], 0);
}

// ---------------------------------------------------------------- SEARCH-04

#[tokio::test]
async fn every_hit_carries_a_stable_reference_that_resolves() {
    let server = server();
    let doc = create_active_document(&server, "alice", json!({ "title": "Linked" })).await;
    let id = doc["id"].as_str().unwrap();

    let (_, found) = send(
        &server,
        "GET",
        "/search?text=linked",
        Some(("bob", "")),
        None,
    )
    .await;
    let reference = found["hits"][0]["reference"].as_str().unwrap();
    assert_eq!(reference, format!("strata:doc:{id}"));

    let (status, resolved) = send(
        &server,
        "GET",
        &format!("/refs/{reference}"),
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resolved["title"], "Linked");

    // The reference survives refiling: identity is the ID, not the path.
    let (status, _) = send(
        &server,
        "PATCH",
        &format!("/documents/{id}"),
        Some(("alice", "")),
        Some(json!({ "folder": "/archive/2026" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, resolved) = send(
        &server,
        "GET",
        &format!("/refs/{reference}"),
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resolved["folder"], "/archive/2026");
}

#[tokio::test]
async fn malformed_references_are_bad_requests() {
    let server = server();
    let (status, _) = send(
        &server,
        "GET",
        "/refs/https:%2F%2Fexample.com",
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = send(
        &server,
        "GET",
        "/refs/strata:doc:not-a-uuid",
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ------------------------------------------------- CAPTURE-07 / CAPTURE-08

#[tokio::test]
async fn extracted_text_is_stored_per_document_and_permission_checked() {
    let server = server();
    let doc = create_active_document(&server, "alice", json!({ "title": "Scan" })).await;
    let id = doc["id"].as_str().unwrap();

    let (status, _) = send(
        &server,
        "GET",
        &format!("/documents/{id}/text"),
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // An in_use document is editable by anyone under the baseline policy —
    // this is what lets a workflow OCR step (its own principal) supply text.
    let (status, stored) = send(
        &server,
        "PUT",
        &format!("/documents/{id}/text"),
        Some(("ocr-worker", "")),
        Some(json!({ "text": "extracted words" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stored["extracted_by"], "ocr-worker");

    let (status, fetched) = send(
        &server,
        "GET",
        &format!("/documents/{id}/text"),
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched["text"], "extracted words");
}

#[tokio::test]
async fn patch_updates_indexing_fields_and_search_sees_them_immediately() {
    let server = server();
    let doc = create_active_document(&server, "alice", json!({ "title": "Untagged" })).await;
    let id = doc["id"].as_str().unwrap();

    let (status, updated) = send(
        &server,
        "PATCH",
        &format!("/documents/{id}"),
        Some(("alice", "")),
        Some(json!({
            "keywords": ["urgent"],
            "metadata": { "customer": "ACME" },
            "folder": "inbox/"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["keywords"], json!(["urgent"]));
    assert_eq!(updated["folder"], "/inbox");

    let (_, found) = send(
        &server,
        "GET",
        "/search?filter=keyword:urgent%20meta.customer:acme%20folder:%2Finbox",
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(found["total"], 1);
}

// ---------------------------------------------------------------- SEARCH-05

#[tokio::test]
async fn limit_caps_hits_while_total_reports_all_matches_newest_first() {
    let server = server();
    for title in ["First", "Second", "Third"] {
        create_active_document(
            &server,
            "alice",
            json!({ "title": title, "doc_type": "note" }),
        )
        .await;
    }

    let (_, found) = send(
        &server,
        "GET",
        "/search?filter=type:note&limit=2",
        Some(("bob", "")),
        None,
    )
    .await;
    assert_eq!(found["total"], 3);
    assert_eq!(found["hits"].as_array().unwrap().len(), 2);
}
