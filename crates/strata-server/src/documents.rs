//! Document records and the lifecycle-status API (ACCESS-10).
//!
//! Records are metadata-only for now: content upload belongs to the capture
//! pipeline and goes through `strata-storage` separately. What matters here
//! is that every document carries a status, that transitions follow the
//! lifecycle rules from `strata-common`, and that every applied transition
//! is appended to the event feed the workflow engine will consume.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use strata_common::{
    Actor, DocumentAction, DocumentId, DocumentStatus, StatusChange, StatusChangedEvent,
};

use crate::AppState;
use crate::error::ApiError;
use crate::identity::Principal;

/// A managed document, as stored and as returned by the API.
#[derive(Debug, Clone, Serialize)]
pub struct DocumentRecord {
    pub id: DocumentId,
    pub title: String,
    /// User who registered the document; `Trustee::Owner` rules match them.
    pub owner: String,
    pub status: DocumentStatus,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    /// Every applied status transition, oldest first (audit trail).
    pub history: Vec<StatusChange>,
}

/// Check `action` against the current policy, treating a denied `View` as
/// "not found" so the API never confirms the existence of documents the
/// caller may not see.
fn authorize(
    state: &AppState,
    record: &DocumentRecord,
    action: DocumentAction,
    actor: &Actor,
) -> Result<(), ApiError> {
    let policy = state.policy.read().expect("policy lock poisoned");
    let is_owner = record.owner == actor.user;

    if !policy.allows(record.status, DocumentAction::View, actor, is_owner) {
        return Err(ApiError::DocumentNotFound(record.id));
    }
    if !policy.allows(record.status, action, actor, is_owner) {
        return Err(ApiError::Forbidden {
            action,
            status: record.status,
        });
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct CreateDocument {
    pub title: String,
}

/// `POST /documents` — register a document; it starts life as a draft.
pub async fn create(
    State(state): State<Arc<AppState>>,
    Principal(actor): Principal,
    Json(body): Json<CreateDocument>,
) -> (StatusCode, Json<DocumentRecord>) {
    let now = Timestamp::now();
    let record = DocumentRecord {
        id: DocumentId::new(),
        title: body.title,
        owner: actor.user,
        status: DocumentStatus::Draft,
        created_at: now,
        updated_at: now,
        history: Vec::new(),
    };

    state
        .documents
        .write()
        .expect("documents lock poisoned")
        .insert(record.id, record.clone());

    (StatusCode::CREATED, Json(record))
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    /// Only return documents currently in this status.
    pub status: Option<DocumentStatus>,
}

/// `GET /documents[?status=...]` — list the documents the caller may view.
///
/// The status filter is what lets the archive stay one central, queryable
/// place (PRESERVE-05) instead of a separate silo.
pub async fn list(
    State(state): State<Arc<AppState>>,
    Principal(actor): Principal,
    Query(query): Query<ListQuery>,
) -> Json<Vec<DocumentRecord>> {
    let documents = state.documents.read().expect("documents lock poisoned");
    let policy = state.policy.read().expect("policy lock poisoned");

    let mut visible: Vec<DocumentRecord> = documents
        .values()
        .filter(|record| query.status.is_none_or(|s| record.status == s))
        .filter(|record| {
            let is_owner = record.owner == actor.user;
            policy.allows(record.status, DocumentAction::View, &actor, is_owner)
        })
        .cloned()
        .collect();
    visible.sort_by_key(|record| (record.created_at, record.id.0));

    Json(visible)
}

/// `GET /documents/{id}` — one record, including its transition history.
pub async fn show(
    State(state): State<Arc<AppState>>,
    Principal(actor): Principal,
    Path(id): Path<DocumentId>,
) -> Result<Json<DocumentRecord>, ApiError> {
    let documents = state.documents.read().expect("documents lock poisoned");
    let record = documents.get(&id).ok_or(ApiError::DocumentNotFound(id))?;
    authorize(&state, record, DocumentAction::View, &actor)?;
    Ok(Json(record.clone()))
}

#[derive(Debug, Deserialize)]
pub struct TransitionRequest {
    pub to: DocumentStatus,
    #[serde(default)]
    pub comment: Option<String>,
}

/// `POST /documents/{id}/status` — move a document through its lifecycle.
///
/// The caller needs `change_status` permission *at the document's current
/// status*, and the transition must be one the lifecycle allows. Every
/// applied transition lands in the document's history and on the event feed
/// (WORKFLOW-08: status changes act as workflow triggers).
pub async fn change_status(
    State(state): State<Arc<AppState>>,
    Principal(actor): Principal,
    Path(id): Path<DocumentId>,
    Json(body): Json<TransitionRequest>,
) -> Result<Json<DocumentRecord>, ApiError> {
    let mut documents = state.documents.write().expect("documents lock poisoned");
    let record = documents
        .get_mut(&id)
        .ok_or(ApiError::DocumentNotFound(id))?;

    authorize(&state, record, DocumentAction::ChangeStatus, &actor)?;

    if !record.status.can_transition_to(body.to) {
        return Err(ApiError::InvalidTransition {
            from: record.status,
            to: body.to,
        });
    }

    let now = Timestamp::now();
    let change = StatusChange {
        from: record.status,
        to: body.to,
        by: actor.user.clone(),
        at: now,
        comment: body.comment,
    };

    record.status = body.to;
    record.updated_at = now;
    record.history.push(change.clone());

    let mut events = state.events.write().expect("events lock poisoned");
    let seq = events.len() as u64 + 1;
    events.push(StatusChangedEvent {
        seq,
        document: record.id,
        from: change.from,
        to: change.to,
        by: change.by,
        at: change.at,
    });

    Ok(Json(record.clone()))
}
