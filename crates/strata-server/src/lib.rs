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

mod documents;
mod error;
mod events;
mod identity;
mod policy;

pub use error::ApiError;

use std::collections::HashMap;
use std::sync::RwLock;

use axum::{Json, Router, routing::get, routing::post};
use strata_common::{DocumentId, Health, HealthStatus, StatusChangedEvent, StatusPolicy};

use documents::DocumentRecord;

pub const SERVICE: &str = "strata-server";

/// Shared mutable server state.
///
/// Documents and events are held in memory for now — durable metadata
/// storage is a separate concern and will replace these maps behind the same
/// handlers. The status policy starts from [`StatusPolicy::baseline`] and is
/// administered via `PUT /policy/status`.
pub struct AppState {
    documents: RwLock<HashMap<DocumentId, DocumentRecord>>,
    policy: RwLock<StatusPolicy>,
    events: RwLock<Vec<StatusChangedEvent>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            documents: RwLock::new(HashMap::new()),
            policy: RwLock::new(StatusPolicy::baseline()),
            events: RwLock::new(Vec::new()),
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
        .route("/documents/{id}", get(documents::show))
        .route("/documents/{id}/status", post(documents::change_status))
        .route("/policy/status", get(policy::show).put(policy::replace))
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
