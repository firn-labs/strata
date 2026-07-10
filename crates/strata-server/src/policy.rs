//! Runtime administration of the status-permission policy.
//!
//! ACCESS-10 demands that permissions be *assignable* per status, and the
//! API-first rule demands that assignment happen over the API — so the
//! policy is server state, not configuration baked into the binary.
//!
//! Any authenticated caller may replace the policy for now; restricting this
//! to administrators is part of the named-access-management work (ACCESS-09)
//! that brings real roles.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use strata_common::StatusPolicy;

use crate::AppState;
use crate::identity::Principal;

/// `GET /policy/status` — the policy currently in force.
pub async fn show(
    State(state): State<Arc<AppState>>,
    Principal(_actor): Principal,
) -> Json<StatusPolicy> {
    Json(state.policy.read().expect("policy lock poisoned").clone())
}

/// `PUT /policy/status` — replace the policy; takes effect immediately.
pub async fn replace(
    State(state): State<Arc<AppState>>,
    Principal(actor): Principal,
    Json(policy): Json<StatusPolicy>,
) -> Json<StatusPolicy> {
    tracing::info!(by = %actor.user, "status policy replaced");
    *state.policy.write().expect("policy lock poisoned") = policy.clone();
    Json(policy)
}
