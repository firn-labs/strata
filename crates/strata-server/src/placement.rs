//! Classification-driven storage placement (STORE-04).
//!
//! Content enters and leaves Strata here: `PUT`/`GET
//! /documents/{id}/content`. On every upload the server derives the target
//! backend from the document's confidentiality tier and the placement
//! policy — callers never pick a medium (STORE-01) — and encrypts the bytes
//! with the operator-owned key before they reach any backend the policy
//! says must not see plaintext.
//!
//! Reclassifying a document (`PUT /documents/{id}/classification`) re-checks
//! its stored blob against the new tier: a blob whose current placement the
//! new tier forbids is moved (and re-encrypted) before the request returns,
//! so the invariant "placement always satisfies the classification" never
//! holds only eventually.
//!
//! The policy itself is administered at runtime via `GET`/`PUT
//! /policy/placement`, mirroring the status policy.

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::header;
use jiff::Timestamp;
use serde::Deserialize;
use strata_common::{
    BackendLocation, BlobPlacement, ClassificationChange, Confidentiality, DocumentAction,
    DocumentId, PlacementDecision, PlacementPolicy,
};
use strata_storage::StorageProvider;

use crate::AppState;
use crate::documents::{DocumentRecord, authorize};
use crate::error::ApiError;
use crate::identity::Principal;

/// One storage medium attached to the server.
///
/// `location` is deployment configuration (see [`BackendLocation`]): the
/// admin attaching a backend declares whose infrastructure holds its bytes,
/// because the provider type alone cannot know. Backends are consulted in
/// configuration order; the first one the placement policy permits wins, so
/// preferred (typically internal) media are listed first.
pub struct StorageBackend {
    pub name: String,
    pub location: BackendLocation,
    pub provider: Arc<dyn StorageProvider>,
}

/// Pick the backend for a blob of `tier`: the first configured backend the
/// placement policy allows, together with the policy's conditions.
fn place(
    state: &AppState,
    tier: Confidentiality,
) -> Result<(&StorageBackend, PlacementDecision), ApiError> {
    let policy = state.placement.read().expect("placement lock poisoned");
    state
        .backends
        .iter()
        .find_map(|backend| {
            policy
                .decide(tier, backend.location)
                .map(|decision| (backend, decision))
        })
        .ok_or(ApiError::NoPlacementBackend { tier })
}

/// Look up the backend a recorded placement points at. Missing means the
/// deployment detached a backend that still holds blobs — an operator
/// error we surface loudly rather than mask.
fn backend_named<'a>(state: &'a AppState, name: &str) -> Result<&'a StorageBackend, ApiError> {
    state
        .backends
        .iter()
        .find(|backend| backend.name == name)
        .ok_or_else(|| ApiError::BackendDetached(name.to_owned()))
}

/// Best-effort removal of a blob that placement moved away from `backend`.
/// A missing blob is fine (the move is what mattered); other failures are
/// logged, not fatal — the bytes were already re-placed correctly.
async fn evict(backend: &StorageBackend, id: DocumentId) {
    if let Err(error) = backend.provider.delete(id).await
        && !matches!(error, strata_storage::StorageError::NotFound(_))
    {
        tracing::warn!(%id, backend = %backend.name, %error, "failed to remove superseded blob");
    }
}

/// Destroy a document's stored bytes as part of destroying the document
/// (used by the retention engine, PRESERVE-06/08). A blob that is already
/// gone is tolerated — absence is the goal — but a detached backend or a
/// failing medium is a real error the caller must see.
pub(crate) async fn destroy_blob(
    state: &AppState,
    id: DocumentId,
    placement: &BlobPlacement,
) -> Result<(), ApiError> {
    let backend = backend_named(state, &placement.backend)?;
    match backend.provider.delete(id).await {
        Ok(()) => Ok(()),
        Err(strata_storage::StorageError::NotFound(_)) => {
            tracing::warn!(%id, backend = %backend.name, "blob already missing at deletion time");
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

/// `PUT /documents/{id}/content` — store the document's content.
///
/// The body is the raw content. Placement and encryption are derived from
/// the document's classification; a re-upload that lands on a different
/// backend removes the superseded blob.
pub async fn upload(
    State(state): State<Arc<AppState>>,
    Principal(actor): Principal,
    Path(id): Path<DocumentId>,
    body: Bytes,
) -> Result<Json<DocumentRecord>, ApiError> {
    // Authorize and snapshot under the lock, then do storage I/O without it
    // (the state lock must never be held across an await).
    let (tier, previous) = {
        let documents = state.documents.read().expect("documents lock poisoned");
        let record = documents.get(&id).ok_or(ApiError::DocumentNotFound(id))?;
        authorize(&state, record, DocumentAction::Edit, &actor)?;
        (record.classification, record.content.clone())
    };

    let now = Timestamp::now();
    let (backend, decision) = place(&state, tier)?;
    let stored = if decision.encrypt {
        state.operator_key.encrypt(&body)
    } else {
        body.to_vec()
    };
    backend.provider.put(id, &stored).await?;

    if let Some(previous) = &previous
        && previous.backend != backend.name
        && let Ok(old) = backend_named(&state, &previous.backend)
    {
        evict(old, id).await;
    }

    let placement = BlobPlacement {
        backend: backend.name.clone(),
        location: backend.location,
        encrypted: decision.encrypt,
        size: body.len() as u64,
        stored_by: actor.user,
        stored_at: now,
    };

    let mut documents = state.documents.write().expect("documents lock poisoned");
    let record = documents
        .get_mut(&id)
        .ok_or(ApiError::DocumentNotFound(id))?;
    record.content = Some(placement);
    record.updated_at = now;
    Ok(Json(record.clone()))
}

/// `GET /documents/{id}/content` — retrieve the document's content.
///
/// Bytes come back decrypted regardless of how placement stored them;
/// encryption at rest is a storage concern, never a caller's.
pub async fn download(
    State(state): State<Arc<AppState>>,
    Principal(actor): Principal,
    Path(id): Path<DocumentId>,
) -> Result<impl axum::response::IntoResponse, ApiError> {
    let placement = {
        let documents = state.documents.read().expect("documents lock poisoned");
        let record = documents.get(&id).ok_or(ApiError::DocumentNotFound(id))?;
        authorize(&state, record, DocumentAction::View, &actor)?;
        record.content.clone().ok_or(ApiError::NoContent(id))?
    };

    let backend = backend_named(&state, &placement.backend)?;
    let stored = backend.provider.get(id).await?;
    let content = if placement.encrypted {
        state
            .operator_key
            .decrypt(&stored)
            .ok_or(ApiError::UnreadableBlob(id))?
    } else {
        stored
    };

    Ok((
        [(header::CONTENT_TYPE, "application/octet-stream")],
        content,
    ))
}

#[derive(Debug, Deserialize)]
pub struct ReclassifyRequest {
    pub to: Confidentiality,
    #[serde(default)]
    pub comment: Option<String>,
}

/// `PUT /documents/{id}/classification` — change the confidentiality tier.
///
/// Requires `classify` permission at the document's current status. When the
/// document has stored content whose placement the new tier no longer
/// permits — wrong backend, or missing encryption — the blob is moved and
/// re-encrypted as part of this request. If no attached backend may hold the
/// new tier, the reclassification is refused entirely.
pub async fn reclassify(
    State(state): State<Arc<AppState>>,
    Principal(actor): Principal,
    Path(id): Path<DocumentId>,
    Json(body): Json<ReclassifyRequest>,
) -> Result<Json<DocumentRecord>, ApiError> {
    let now = Timestamp::now();

    // Apply the metadata change under the lock; plan the blob move first so
    // an impossible placement refuses the whole request up front.
    let move_plan = {
        let mut documents = state.documents.write().expect("documents lock poisoned");
        let record = documents
            .get_mut(&id)
            .ok_or(ApiError::DocumentNotFound(id))?;
        authorize(&state, record, DocumentAction::Classify, &actor)?;

        if record.classification == body.to {
            return Ok(Json(record.clone()));
        }

        let move_plan = match &record.content {
            Some(current) => {
                let policy = state.placement.read().expect("placement lock poisoned");
                let still_compliant = policy
                    .decide(body.to, current.location)
                    .is_some_and(|decision| decision.encrypt == current.encrypted);
                drop(policy);
                if still_compliant {
                    None
                } else {
                    // Errors here (no backend for the new tier) leave the
                    // document untouched.
                    let (target, decision) = place(&state, body.to)?;
                    Some((current.clone(), target.name.clone(), decision))
                }
            }
            None => None,
        };

        record.classification_history.push(ClassificationChange {
            from: record.classification,
            to: body.to,
            by: actor.user.clone(),
            at: now,
            comment: body.comment,
        });
        record.classification = body.to;
        record.updated_at = now;
        move_plan
    };

    if let Some((current, target_name, decision)) = move_plan {
        let source = backend_named(&state, &current.backend)?;
        let stored = source.provider.get(id).await?;
        let plaintext = if current.encrypted {
            state
                .operator_key
                .decrypt(&stored)
                .ok_or(ApiError::UnreadableBlob(id))?
        } else {
            stored
        };

        let target = backend_named(&state, &target_name)?;
        let restored = if decision.encrypt {
            state.operator_key.encrypt(&plaintext)
        } else {
            plaintext.clone()
        };
        target.provider.put(id, &restored).await?;
        if current.backend != target.name {
            evict(source, id).await;
        }

        let mut documents = state.documents.write().expect("documents lock poisoned");
        let record = documents
            .get_mut(&id)
            .ok_or(ApiError::DocumentNotFound(id))?;
        record.content = Some(BlobPlacement {
            backend: target.name.clone(),
            location: target.location,
            encrypted: decision.encrypt,
            size: plaintext.len() as u64,
            stored_by: current.stored_by,
            stored_at: current.stored_at,
        });
        return Ok(Json(record.clone()));
    }

    let documents = state.documents.read().expect("documents lock poisoned");
    let record = documents.get(&id).ok_or(ApiError::DocumentNotFound(id))?;
    Ok(Json(record.clone()))
}

/// `GET /policy/placement` — the placement policy currently in force.
pub async fn policy_show(
    State(state): State<Arc<AppState>>,
    Principal(_actor): Principal,
) -> Json<PlacementPolicy> {
    Json(
        state
            .placement
            .read()
            .expect("placement lock poisoned")
            .clone(),
    )
}

/// `PUT /policy/placement` — replace the policy; takes effect immediately.
///
/// Applies to placements decided from now on; existing blobs are re-placed
/// when their document is next reclassified or re-uploaded. Any
/// authenticated caller may do this for now, like the status policy, until
/// real roles land (ACCESS-09).
pub async fn policy_replace(
    State(state): State<Arc<AppState>>,
    Principal(actor): Principal,
    Json(policy): Json<PlacementPolicy>,
) -> Json<PlacementPolicy> {
    tracing::info!(by = %actor.user, tiers = policy.rules.len(), "placement policy replaced");
    *state.placement.write().expect("placement lock poisoned") = policy.clone();
    Json(policy)
}
