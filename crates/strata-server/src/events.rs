//! Status-change event feed (WORKFLOW-08).
//!
//! Every applied status transition is appended here with a strictly
//! increasing sequence number. The workflow engine polls
//! `GET /events/status?after=<last seen seq>` to pick up new events exactly
//! once and match them against its trigger nodes; other consumers (audit
//! tooling, the frontend) can read the same feed.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;
use strata_common::StatusChangedEvent;

use crate::AppState;
use crate::identity::Principal;

#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    /// Only return events with a sequence number greater than this.
    #[serde(default)]
    pub after: u64,
}

/// `GET /events/status?after=<seq>` — status-change events, oldest first.
pub async fn list(
    State(state): State<Arc<AppState>>,
    Principal(_actor): Principal,
    Query(query): Query<EventsQuery>,
) -> Json<Vec<StatusChangedEvent>> {
    let events = state.events.read().expect("events lock poisoned");
    let fresh = events
        .iter()
        .filter(|event| event.seq > query.after)
        .cloned()
        .collect();
    Json(fresh)
}
