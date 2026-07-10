//! Run trigger and run-trace query API (WORKFLOW-05).
//!
//! `POST /flows/{id}/runs` executes a flow synchronously and returns its
//! full trace; listing endpoints return summaries so the frontend can show
//! a run history without hauling every step, and `GET /runs/{id}` returns
//! the complete step-by-step record for diagnosis or status reporting.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::AppState;
use crate::engine::{self, RunId, RunRecord, RunStatus};
use crate::error::ApiError;
use crate::flow::{FlowId, NodeKind};
use crate::identity::Principal;

#[derive(Debug, Deserialize)]
pub struct TriggerRequest {
    /// The trigger node to enter the flow through.
    pub trigger: String,
    /// Payload made available to the run (and its condition nodes).
    #[serde(default)]
    pub input: Value,
}

/// One line of a run history: everything but the steps.
#[derive(Debug, Serialize)]
pub struct RunSummary {
    pub id: RunId,
    pub flow: FlowId,
    pub triggered_by: String,
    pub trigger_node: String,
    pub status: RunStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub started_at: Timestamp,
    pub finished_at: Timestamp,
    pub steps: usize,
}

impl From<&RunRecord> for RunSummary {
    fn from(run: &RunRecord) -> Self {
        Self {
            id: run.id,
            flow: run.flow,
            triggered_by: run.triggered_by.clone(),
            trigger_node: run.trigger_node.clone(),
            status: run.status,
            error: run.error.clone(),
            started_at: run.started_at,
            finished_at: run.finished_at,
            steps: run.steps.len(),
        }
    }
}

/// `POST /flows/{id}/runs` — execute the flow and record the trace.
pub async fn trigger(
    State(state): State<Arc<AppState>>,
    Principal(actor): Principal,
    Path(flow_id): Path<FlowId>,
    Json(request): Json<TriggerRequest>,
) -> Result<(StatusCode, Json<RunRecord>), ApiError> {
    let flow = {
        let flows = state.flows.read().expect("flows lock poisoned");
        flows
            .get(&flow_id)
            .cloned()
            .ok_or(ApiError::FlowNotFound(flow_id))?
    };

    let node = flow
        .node(&request.trigger)
        .ok_or_else(|| ApiError::NotATrigger {
            node: request.trigger.clone(),
            reason: "the flow has no node with this id",
        })?;
    if node.kind != NodeKind::Trigger {
        return Err(ApiError::NotATrigger {
            node: request.trigger,
            reason: "only trigger nodes start a run",
        });
    }

    let run = engine::execute(&flow, &request.trigger, request.input, &actor.user);

    let mut runs = state.runs.write().expect("runs lock poisoned");
    runs.push(run.clone());
    Ok((StatusCode::CREATED, Json(run)))
}

#[derive(Debug, Deserialize)]
pub struct RunsQuery {
    /// Only runs with this status (`completed` / `failed`).
    #[serde(default)]
    pub status: Option<RunStatus>,
}

/// `GET /flows/{id}/runs` — this flow's run history, oldest first.
pub async fn list_for_flow(
    State(state): State<Arc<AppState>>,
    Principal(_actor): Principal,
    Path(flow_id): Path<FlowId>,
    Query(query): Query<RunsQuery>,
) -> Result<Json<Vec<RunSummary>>, ApiError> {
    {
        let flows = state.flows.read().expect("flows lock poisoned");
        if !flows.contains_key(&flow_id) {
            return Err(ApiError::FlowNotFound(flow_id));
        }
    }
    let runs = state.runs.read().expect("runs lock poisoned");
    let summaries = runs
        .iter()
        .filter(|run| run.flow == flow_id)
        .filter(|run| query.status.is_none_or(|status| run.status == status))
        .map(RunSummary::from)
        .collect();
    Ok(Json(summaries))
}

/// `GET /runs/{id}` — the complete step-by-step trace of one run.
pub async fn show(
    State(state): State<Arc<AppState>>,
    Principal(_actor): Principal,
    Path(id): Path<RunId>,
) -> Result<Json<RunRecord>, ApiError> {
    let runs = state.runs.read().expect("runs lock poisoned");
    runs.iter()
        .find(|run| run.id == id)
        .cloned()
        .map(Json)
        .ok_or(ApiError::RunNotFound(id))
}
