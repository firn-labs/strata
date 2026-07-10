//! Document records and the lifecycle-status API (ACCESS-10).
//!
//! Records are metadata-only for now: content upload belongs to the capture
//! pipeline and goes through `strata-storage` separately. What matters here
//! is that every document carries a status, that transitions follow the
//! lifecycle rules from `strata-common`, and that every applied transition
//! is appended to the event feed the workflow engine will consume.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use strata_common::{
    Actor, BlobPlacement, ClassificationChange, Confidentiality, DocumentAction, DocumentId,
    DocumentStatus, RetentionDeadline, RetentionSource, StatusChange, StatusChangedEvent,
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
    /// Document type ("invoice", "contract", …) — what retention-plan rules
    /// match on (PRESERVE-06). Free-form until a managed type catalog lands.
    pub doc_type: Option<String>,
    /// Team the document belongs to — the second retention-plan dimension.
    pub team: Option<String>,
    /// Indexing keywords/tags (CAPTURE-08) — filterable via `keyword:` in
    /// search strings (SEARCH-02). Set by automated indexing or by hand.
    pub keywords: Vec<String>,
    /// Free key-value metadata — filterable via `meta.<key>:` (SEARCH-02).
    pub metadata: BTreeMap<String, String>,
    /// Position in the filing structure, as a normalized `/a/b/c` path
    /// (SEARCH-03). Purely a navigation aid: identity stays the ID, and
    /// refiling never invalidates references (SEARCH-04).
    pub folder: Option<String>,
    pub status: DocumentStatus,
    /// Confidentiality tier (STORE-04 × CAPTURE-10) — what storage placement
    /// and at-rest encryption are derived from.
    pub classification: Confidentiality,
    /// Where the document's content currently lives; `None` until content
    /// is uploaded.
    pub content: Option<BlobPlacement>,
    /// Deletion deadline (PRESERVE-06): while set and in the future, the
    /// document cannot be deleted.
    pub retention: Option<RetentionDeadline>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    /// Every applied status transition, oldest first (audit trail).
    pub history: Vec<StatusChange>,
    /// Every applied classification change, oldest first (audit trail).
    pub classification_history: Vec<ClassificationChange>,
}

/// Check `action` against the current policy, treating a denied `View` as
/// "not found" so the API never confirms the existence of documents the
/// caller may not see.
pub(crate) fn authorize(
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
    #[serde(default)]
    pub doc_type: Option<String>,
    #[serde(default)]
    pub team: Option<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    /// Filing-structure position; normalized to a `/a/b/c` path.
    #[serde(default)]
    pub folder: Option<String>,
    /// Confidentiality tier the document starts with (STORE-04). Documents
    /// unclassified at capture time default to `internal`: never published
    /// by accident, but not locked to internal-only storage either.
    #[serde(default)]
    pub classification: Option<Confidentiality>,
}

/// Normalize a filing path to `/segment/segment` form: leading slash, no
/// trailing slash, no empty segments. Rejects paths with no segments at all.
pub(crate) fn normalize_folder(raw: &str) -> Result<String, ApiError> {
    let segments: Vec<&str> = raw
        .split('/')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if segments.is_empty() {
        return Err(ApiError::InvalidFolder(raw.to_string()));
    }
    Ok(format!("/{}", segments.join("/")))
}

/// `POST /documents` — register a document; it starts life as a draft.
pub async fn create(
    State(state): State<Arc<AppState>>,
    Principal(actor): Principal,
    Json(body): Json<CreateDocument>,
) -> Result<(StatusCode, Json<DocumentRecord>), ApiError> {
    let folder = body.folder.as_deref().map(normalize_folder).transpose()?;

    let now = Timestamp::now();
    let record = DocumentRecord {
        id: DocumentId::new(),
        title: body.title,
        owner: actor.user,
        doc_type: body.doc_type,
        team: body.team,
        keywords: body.keywords,
        metadata: body.metadata,
        folder,
        status: DocumentStatus::Draft,
        classification: body.classification.unwrap_or(Confidentiality::Internal),
        content: None,
        retention: None,
        created_at: now,
        updated_at: now,
        history: Vec::new(),
        classification_history: Vec::new(),
    };

    state
        .documents
        .write()
        .expect("documents lock poisoned")
        .insert(record.id, record.clone());

    Ok((StatusCode::CREATED, Json(record)))
}

#[derive(Debug, Deserialize)]
pub struct UpdateDocument {
    pub title: Option<String>,
    pub doc_type: Option<String>,
    pub team: Option<String>,
    pub keywords: Option<Vec<String>>,
    pub metadata: Option<BTreeMap<String, String>>,
    /// New filing position; refiling never changes the document's ID or
    /// reference (SEARCH-04). Clearing a folder is not supported yet.
    pub folder: Option<String>,
}

/// `PATCH /documents/{id}` — update descriptive and indexing fields.
///
/// This is the human-override half of CAPTURE-08: automated indexing (a
/// workflow step) and users go through the same endpoint, so corrected
/// keywords, metadata, and filing are immediately searchable.
pub async fn update(
    State(state): State<Arc<AppState>>,
    Principal(actor): Principal,
    Path(id): Path<DocumentId>,
    Json(body): Json<UpdateDocument>,
) -> Result<Json<DocumentRecord>, ApiError> {
    let folder = body.folder.as_deref().map(normalize_folder).transpose()?;

    let mut documents = state.documents.write().expect("documents lock poisoned");
    let record = documents
        .get_mut(&id)
        .ok_or(ApiError::DocumentNotFound(id))?;
    authorize(&state, record, DocumentAction::Edit, &actor)?;

    if let Some(title) = body.title {
        record.title = title;
    }
    if let Some(doc_type) = body.doc_type {
        record.doc_type = Some(doc_type);
    }
    if let Some(team) = body.team {
        record.team = Some(team);
    }
    if let Some(keywords) = body.keywords {
        record.keywords = keywords;
    }
    if let Some(metadata) = body.metadata {
        record.metadata = metadata;
    }
    if let Some(folder) = folder {
        record.folder = Some(folder);
    }
    record.updated_at = Timestamp::now();

    Ok(Json(record.clone()))
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

/// Machine-readable text extracted from a document's content (CAPTURE-07).
///
/// Extraction itself (OCR, PDF text layer, …) is a capture-pipeline step in
/// the workflow layer; the server only stores the result and feeds it to
/// full-text search (SEARCH-01) and long-term readability (PRESERVE-03).
/// Kept beside the record rather than on it so document responses stay
/// small.
#[derive(Debug, Clone, Serialize)]
pub struct ExtractedText {
    pub text: String,
    /// Who supplied the text — for OCR runs, the workflow's principal.
    pub extracted_by: String,
    pub extracted_at: Timestamp,
}

#[derive(Debug, Deserialize)]
pub struct SetText {
    pub text: String,
}

/// `PUT /documents/{id}/text` — store or replace the extracted full text.
pub async fn set_text(
    State(state): State<Arc<AppState>>,
    Principal(actor): Principal,
    Path(id): Path<DocumentId>,
    Json(body): Json<SetText>,
) -> Result<Json<ExtractedText>, ApiError> {
    let documents = state.documents.read().expect("documents lock poisoned");
    let record = documents.get(&id).ok_or(ApiError::DocumentNotFound(id))?;
    authorize(&state, record, DocumentAction::Edit, &actor)?;
    drop(documents);

    let extracted = ExtractedText {
        text: body.text,
        extracted_by: actor.user,
        extracted_at: Timestamp::now(),
    };
    state
        .texts
        .write()
        .expect("texts lock poisoned")
        .insert(id, extracted.clone());

    Ok(Json(extracted))
}

/// `GET /documents/{id}/text` — the stored extracted text, if any.
pub async fn get_text(
    State(state): State<Arc<AppState>>,
    Principal(actor): Principal,
    Path(id): Path<DocumentId>,
) -> Result<Json<ExtractedText>, ApiError> {
    let documents = state.documents.read().expect("documents lock poisoned");
    let record = documents.get(&id).ok_or(ApiError::DocumentNotFound(id))?;
    authorize(&state, record, DocumentAction::View, &actor)?;

    let texts = state.texts.read().expect("texts lock poisoned");
    let extracted = texts.get(&id).ok_or(ApiError::NoExtractedText(id))?;
    Ok(Json(extracted.clone()))
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

    // Archiving starts the retention clock (PRESERVE-06): documents without
    // an explicit deadline get the standard one from the retention plan. An
    // already-set deadline — explicit, or from an earlier archiving before a
    // reactivation round-trip — is never overwritten here.
    if record.status == DocumentStatus::Archived && record.retention.is_none() {
        let plan = state
            .retention_plan
            .read()
            .expect("retention plan lock poisoned");
        if let Some(rule) = plan.applicable_rule(record.doc_type.as_deref(), record.team.as_deref())
        {
            record.retention = Some(RetentionDeadline {
                delete_after: crate::retention::deadline_from(now, rule.retain_for_days),
                source: RetentionSource::Plan,
                set_by: actor.user.clone(),
                set_at: now,
            });
        }
    }

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
