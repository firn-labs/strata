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

mod documents;
mod dossiers;
mod error;
mod events;
mod identity;
mod policy;
mod retention;

pub use error::ApiError;

use std::collections::HashMap;
use std::sync::RwLock;

use axum::{
    Json, Router,
    routing::{delete, get, post, put},
};
use strata_common::{
    DeletionCertificate, DocumentId, DossierId, Health, HealthStatus, RetentionNotification,
    RetentionPlan, StatusChangedEvent, StatusPolicy,
};

use documents::DocumentRecord;
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
}

impl AppState {
    pub fn new() -> Self {
        Self {
            documents: RwLock::new(HashMap::new()),
            dossiers: RwLock::new(HashMap::new()),
            policy: RwLock::new(StatusPolicy::baseline()),
            events: RwLock::new(Vec::new()),
            retention_plan: RwLock::new(RetentionPlan::default()),
            deletions: RwLock::new(Vec::new()),
            notifications: RwLock::new(Vec::new()),
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
            get(documents::show).delete(retention::delete_document),
        )
        .route("/documents/{id}/status", post(documents::change_status))
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
        .with_state(state)
}

async fn healthz() -> Json<Health> {
    Json(Health {
        service: SERVICE,
        version: env!("CARGO_PKG_VERSION"),
        status: HealthStatus::Ok,
    })
}
