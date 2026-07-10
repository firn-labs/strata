//! Flow registration API.
//!
//! The visual editor saves its JSON graph here; the engine executes exactly
//! what is stored (WORKFLOW-06). Definitions are validated structurally at
//! registration so runs never meet dangling edges. Storage is in-memory for
//! now, same as the core server's metadata — durability is a separate
//! concern behind the same handlers.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Deserialize;

use crate::AppState;
use crate::error::ApiError;
use crate::flow::{FlowDefinition, FlowEdge, FlowId, FlowNode};
use crate::identity::Principal;

/// A definition as submitted by the editor: the engine assigns the id.
#[derive(Debug, Deserialize)]
pub struct NewFlow {
    pub name: String,
    /// Department or team that owns the flow; defaults to the caller.
    #[serde(default)]
    pub owner: Option<String>,
    pub nodes: Vec<FlowNode>,
    #[serde(default)]
    pub edges: Vec<FlowEdge>,
}

/// `POST /flows` — register a flow definition.
pub async fn create(
    State(state): State<Arc<AppState>>,
    Principal(actor): Principal,
    Json(new): Json<NewFlow>,
) -> Result<(StatusCode, Json<FlowDefinition>), ApiError> {
    let flow = FlowDefinition {
        id: FlowId::new(),
        name: new.name,
        owner: new.owner.unwrap_or(actor.user),
        nodes: new.nodes,
        edges: new.edges,
    };
    flow.validate().map_err(ApiError::InvalidFlow)?;

    let mut flows = state.flows.write().expect("flows lock poisoned");
    flows.insert(flow.id, flow.clone());
    Ok((StatusCode::CREATED, Json(flow)))
}

/// `GET /flows` — all registered definitions.
pub async fn list(
    State(state): State<Arc<AppState>>,
    Principal(_actor): Principal,
) -> Json<Vec<FlowDefinition>> {
    let flows = state.flows.read().expect("flows lock poisoned");
    let mut all: Vec<_> = flows.values().cloned().collect();
    all.sort_by(|a, b| a.name.cmp(&b.name));
    Json(all)
}

/// `GET /flows/{id}` — one definition.
pub async fn show(
    State(state): State<Arc<AppState>>,
    Principal(_actor): Principal,
    Path(id): Path<FlowId>,
) -> Result<Json<FlowDefinition>, ApiError> {
    let flows = state.flows.read().expect("flows lock poisoned");
    flows
        .get(&id)
        .cloned()
        .map(Json)
        .ok_or(ApiError::FlowNotFound(id))
}
