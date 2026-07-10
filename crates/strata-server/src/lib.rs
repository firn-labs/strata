//! Strata core API server — library part.
//!
//! The binary in `main.rs` only wires this router to a TCP listener; all
//! routes, state, and behavior live here so tests can drive the exact same
//! app in-process.
//!
//! Implemented so far:
//! - Document records with lifecycle status and transition history
//!   (ACCESS-10), `POST /documents`, `GET /documents`, `GET /documents/{id}`,
//!   `POST /documents/{id}/status`.
//! - Status-based permissions, configurable at runtime (ACCESS-10 ×
//!   ACCESS-09): `GET`/`PUT /policy/status`.
//! - Status-change event feed for workflow triggers (WORKFLOW-08):
//!   `GET /events/status?after=<seq>`.
//! - Dossiers ("E-Akte") that group documents by reference, with own
//!   metadata, external references, per-dossier ACLs, and per-entry access
//!   lists (STORE-09, STORE-10, ACCESS-09): `POST`/`GET /dossiers`,
//!   `GET`/`PATCH /dossiers/{id}`, `PUT /dossiers/{id}/acl`,
//!   `POST /dossiers/{id}/entries`,
//!   `DELETE /dossiers/{id}/entries/{entry_id}`,
//!   `PUT /dossiers/{id}/entries/{entry_id}/access`.
//! - Retention and deletion engine (PRESERVE-06/07/08): deletion deadlines
//!   with a per-type/per-team retention plan, deadline-blocked deletion with
//!   certificates and a deletion history, and an expiry sweep that deletes or
//!   notifies per document class: `PUT /documents/{id}/retention`,
//!   `DELETE /documents/{id}`, `GET`/`PUT /retention/plan`,
//!   `POST /retention/sweep`, `GET /retention/deletions`,
//!   `GET /retention/notifications`.
//! - Classification-driven storage placement and encryption (STORE-04 ×
//!   CAPTURE-10): every document carries a confidentiality tier; content
//!   upload derives backend and at-rest encryption from it, and
//!   reclassification re-places non-compliant blobs:
//!   `PUT`/`GET /documents/{id}/content`,
//!   `PUT /documents/{id}/classification`, `GET`/`PUT /policy/placement`.
//! - Search facade (SEARCH-01…05): documents carry keywords, free metadata,
//!   and a filing-structure folder (`PATCH /documents/{id}`), plus extracted
//!   full text supplied by the capture pipeline (CAPTURE-07,
//!   `PUT`/`GET /documents/{id}/text`). One permission-filtered query core
//!   powers full-text + boolean-filter + folder + time-range search
//!   (`GET /search`), folder-tree navigation (`GET /search/folders`),
//!   timeline histograms (`GET /search/timeline`), and stable
//!   `strata:doc:<uuid>` reference resolution (`GET /refs/{reference}`).

mod crypto;
mod documents;
mod dossiers;
mod error;
mod events;
mod identity;
mod placement;
mod policy;
mod retention;
mod search;

pub use crypto::OperatorKey;
pub use error::ApiError;
pub use placement::StorageBackend;

use std::collections::HashMap;
use std::sync::RwLock;

use axum::{
    Json, Router,
    routing::{delete, get, post, put},
};
use strata_common::{
    DeletionCertificate, DocumentId, DossierId, Health, HealthStatus, PlacementPolicy,
    RetentionNotification, RetentionPlan, StatusChangedEvent, StatusPolicy,
};

use documents::{DocumentRecord, ExtractedText};
use dossiers::DossierRecord;

pub const SERVICE: &str = "strata-server";

/// Shared mutable server state.
///
/// Documents and events are held in memory for now — durable metadata
/// storage is a separate concern and will replace these maps behind the same
/// handlers. The status policy starts from [`StatusPolicy::baseline`] and is
/// administered via `PUT /policy/status`.
pub struct AppState {
    documents: RwLock<HashMap<DocumentId, DocumentRecord>>,
    /// Extracted full text per document (CAPTURE-07) — the corpus behind
    /// full-text search (SEARCH-01). Kept beside the records so document
    /// responses stay small.
    texts: RwLock<HashMap<DocumentId, ExtractedText>>,
    dossiers: RwLock<HashMap<DossierId, DossierRecord>>,
    policy: RwLock<StatusPolicy>,
    events: RwLock<Vec<StatusChangedEvent>>,
    /// Standard deletion deadlines per document type/team (PRESERVE-06).
    /// Starts empty: standard deadlines are a deployment's legal decision,
    /// not something to ship defaults for.
    retention_plan: RwLock<RetentionPlan>,
    /// Deletion history: one certificate per performed deletion, in order
    /// (PRESERVE-08).
    deletions: RwLock<Vec<DeletionCertificate>>,
    /// Expiry notifications issued to responsible persons (PRESERVE-07).
    notifications: RwLock<Vec<RetentionNotification>>,
    /// Placement requirements per confidentiality tier (STORE-04),
    /// administered via `PUT /policy/placement`.
    placement: RwLock<PlacementPolicy>,
    /// Attached storage media, in placement-preference order. Fixed at
    /// startup — attaching backends at runtime is a later concern.
    backends: Vec<StorageBackend>,
    /// Key for all at-rest encryption (STORE-04); owned by the operating
    /// organization, configured at startup.
    operator_key: OperatorKey,
}

impl AppState {
    /// State without any storage backend: metadata operations all work,
    /// content upload reports that no backend can take the blob.
    pub fn new() -> Self {
        Self::with_storage(Vec::new(), OperatorKey::generate())
    }

    pub fn with_storage(backends: Vec<StorageBackend>, operator_key: OperatorKey) -> Self {
        Self {
            documents: RwLock::new(HashMap::new()),
            texts: RwLock::new(HashMap::new()),
            dossiers: RwLock::new(HashMap::new()),
            policy: RwLock::new(StatusPolicy::baseline()),
            events: RwLock::new(Vec::new()),
            retention_plan: RwLock::new(RetentionPlan::default()),
            deletions: RwLock::new(Vec::new()),
            notifications: RwLock::new(Vec::new()),
            placement: RwLock::new(PlacementPolicy::baseline()),
            backends,
            operator_key,
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Build the full API router.
pub fn app(state: std::sync::Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/documents", post(documents::create).get(documents::list))
        .route(
            "/documents/{id}",
            get(documents::show)
                .patch(documents::update)
                .delete(retention::delete_document),
        )
        .route(
            "/documents/{id}/text",
            put(documents::set_text).get(documents::get_text),
        )
        .route("/documents/{id}/status", post(documents::change_status))
        .route(
            "/documents/{id}/content",
            put(placement::upload).get(placement::download),
        )
        .route("/documents/{id}/classification", put(placement::reclassify))
        .route("/documents/{id}/retention", put(retention::set_deadline))
        .route("/dossiers", post(dossiers::create).get(dossiers::list))
        .route(
            "/dossiers/{id}",
            get(dossiers::show).patch(dossiers::update),
        )
        .route("/dossiers/{id}/acl", put(dossiers::replace_acl))
        .route("/dossiers/{id}/entries", post(dossiers::add_entry))
        .route(
            "/dossiers/{id}/entries/{entry_id}",
            delete(dossiers::remove_entry),
        )
        .route(
            "/dossiers/{id}/entries/{entry_id}/access",
            put(dossiers::set_entry_access),
        )
        .route("/policy/status", get(policy::show).put(policy::replace))
        .route(
            "/policy/placement",
            get(placement::policy_show).put(placement::policy_replace),
        )
        .route(
            "/retention/plan",
            get(retention::plan_show).put(retention::plan_replace),
        )
        .route("/retention/sweep", post(retention::sweep))
        .route("/retention/deletions", get(retention::deletions_list))
        .route(
            "/retention/notifications",
            get(retention::notifications_list),
        )
        .route("/events/status", get(events::list))
        .route("/search", get(search::search))
        .route("/search/folders", get(search::folders))
        .route("/search/timeline", get(search::timeline))
        .route("/refs/{reference}", get(search::resolve))
        .with_state(state)
}

async fn healthz() -> Json<Health> {
    Json(Health {
        service: SERVICE,
        version: env!("CARGO_PKG_VERSION"),
        status: HealthStatus::Ok,
    })
}
