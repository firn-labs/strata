//! Strata workflow engine — library part.
//!
//! The middle layer of Strata: departments build flows in the visual editor
//! (frontend), the editor saves them here as JSON graph definitions, and
//! this service executes them. The binary in `main.rs` only wires this
//! router to a TCP listener, so tests drive the exact same app in-process.
//!
//! Implemented so far:
//! - Flow registration with structural validation (WORKFLOW-06 groundwork):
//!   `POST /flows`, `GET /flows`, `GET /flows/{id}`.
//! - Execution with a per-run step trace (WORKFLOW-05): every run records
//!   trigger, inputs, condition decisions, outcomes, and timestamps.
//!   `POST /flows/{id}/runs` executes and returns the trace,
//!   `GET /flows/{id}/runs?status=` lists a flow's run history,
//!   `GET /runs/{id}` returns one full trace.
//!
//! Step nodes do not yet call the core server — that action executor lands
//! with WORKFLOW-08 behind the same trace format.

mod engine;
mod error;
mod flow;
mod flows;
mod identity;
mod runs;

pub use engine::{RunId, RunRecord, RunStatus, StepOutcome, StepRecord};
pub use error::ApiError;
pub use flow::{FlowDefinition, FlowEdge, FlowId, FlowNode, NodeKind};

use std::collections::HashMap;
use std::sync::RwLock;

use axum::{
    Json, Router,
    routing::{get, post},
};
use strata_common::{Health, HealthStatus};

pub const SERVICE: &str = "strata-workflow";

/// Shared mutable service state.
///
/// Flows and run records are held in memory for now, mirroring the core
/// server's approach — durable storage replaces these behind the same
/// handlers. Runs are kept in execution order, so history listings are
/// chronological for free.
#[derive(Default)]
pub struct AppState {
    flows: RwLock<HashMap<FlowId, FlowDefinition>>,
    /// Every executed run with its full step trace (WORKFLOW-05).
    runs: RwLock<Vec<RunRecord>>,
}

impl AppState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Build the full API router.
pub fn app(state: std::sync::Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/flows", post(flows::create).get(flows::list))
        .route("/flows/{id}", get(flows::show))
        .route(
            "/flows/{id}/runs",
            post(runs::trigger).get(runs::list_for_flow),
        )
        .route("/runs/{id}", get(runs::show))
        .with_state(state)
}

async fn healthz() -> Json<Health> {
    Json(Health {
        service: SERVICE,
        version: env!("CARGO_PKG_VERSION"),
        status: HealthStatus::Ok,
    })
}
