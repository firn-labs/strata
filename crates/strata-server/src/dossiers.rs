//! Dossier ("E-Akte") records and API (STORE-09, STORE-10, ACCESS-09).
//!
//! Dossiers group documents by business context through *references*: the
//! same document may sit in any number of dossiers without being copied.
//! Each dossier carries free, user-extendable metadata (CAPTURE-10 applied
//! to files), its own ACL, and optional per-entry access lists that narrow
//! which entries a viewer sees inside the dossier.
//!
//! Visibility versus administration: per-entry access lists gate *seeing* an
//! entry. Edit and manage operations deliberately work on all entries — a
//! team lead may remove or re-open an entry that is currently hidden from
//! them, which keeps restricted entries administrable and lockout-free.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use strata_common::{
    Actor, DocumentAction, DossierAcl, DossierAction, DossierEntry, DossierEntryId, DossierId,
    EntryReference, Trustee,
};

use crate::AppState;
use crate::error::ApiError;
use crate::identity::Principal;

/// An electronic file, as stored. The API returns it through
/// [`DossierView`], which filters entries by the caller's visibility.
#[derive(Debug, Clone)]
pub struct DossierRecord {
    pub id: DossierId,
    pub name: String,
    /// User who created the dossier; `Trustee::Owner` rules match them.
    pub owner: String,
    /// Free, user-extendable metadata (STORE-09 × CAPTURE-10).
    pub metadata: BTreeMap<String, String>,
    pub acl: DossierAcl,
    pub entries: Vec<DossierEntry>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// A dossier as one caller sees it: entries they may not see are omitted,
/// and `hidden_entries` says only *how many* were withheld — never what.
#[derive(Debug, Serialize)]
pub struct DossierView {
    pub id: DossierId,
    pub name: String,
    pub owner: String,
    pub metadata: BTreeMap<String, String>,
    pub acl: DossierAcl,
    pub entries: Vec<DossierEntry>,
    pub hidden_entries: usize,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl DossierRecord {
    fn view_for(&self, actor: &Actor) -> DossierView {
        let is_owner = self.owner == actor.user;
        let (visible, hidden): (Vec<&DossierEntry>, Vec<&DossierEntry>) = self
            .entries
            .iter()
            .partition(|entry| entry.visible_to(actor, is_owner));
        DossierView {
            id: self.id,
            name: self.name.clone(),
            owner: self.owner.clone(),
            metadata: self.metadata.clone(),
            acl: self.acl.clone(),
            entries: visible.into_iter().cloned().collect(),
            hidden_entries: hidden.len(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

/// Check `action` against the dossier's ACL, treating a denied `View` as
/// "not found" so the API never confirms the existence of dossiers the
/// caller may not see (same rule as for documents).
fn authorize(record: &DossierRecord, action: DossierAction, actor: &Actor) -> Result<(), ApiError> {
    let is_owner = record.owner == actor.user;
    if !record.acl.allows(DossierAction::View, actor, is_owner) {
        return Err(ApiError::DossierNotFound(record.id));
    }
    if !record.acl.allows(action, actor, is_owner) {
        return Err(ApiError::DossierForbidden(action));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct CreateDossier {
    pub name: String,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

/// `POST /dossiers` — open a new electronic file, private to its creator.
pub async fn create(
    State(state): State<Arc<AppState>>,
    Principal(actor): Principal,
    Json(body): Json<CreateDossier>,
) -> (StatusCode, Json<DossierView>) {
    let now = Timestamp::now();
    let record = DossierRecord {
        id: DossierId::new(),
        name: body.name,
        owner: actor.user.clone(),
        metadata: body.metadata,
        acl: DossierAcl::private_to_owner(),
        entries: Vec::new(),
        created_at: now,
        updated_at: now,
    };
    let view = record.view_for(&actor);

    state
        .dossiers
        .write()
        .expect("dossiers lock poisoned")
        .insert(record.id, record);

    (StatusCode::CREATED, Json(view))
}

/// `GET /dossiers` — list the dossiers the caller may view.
pub async fn list(
    State(state): State<Arc<AppState>>,
    Principal(actor): Principal,
) -> Json<Vec<DossierView>> {
    let dossiers = state.dossiers.read().expect("dossiers lock poisoned");
    let mut visible: Vec<DossierView> = dossiers
        .values()
        .filter(|record| {
            let is_owner = record.owner == actor.user;
            record.acl.allows(DossierAction::View, &actor, is_owner)
        })
        .map(|record| record.view_for(&actor))
        .collect();
    visible.sort_by_key(|view| (view.created_at, view.id.0));
    Json(visible)
}

/// `GET /dossiers/{id}` — one dossier, entries filtered by visibility.
pub async fn show(
    State(state): State<Arc<AppState>>,
    Principal(actor): Principal,
    Path(id): Path<DossierId>,
) -> Result<Json<DossierView>, ApiError> {
    let dossiers = state.dossiers.read().expect("dossiers lock poisoned");
    let record = dossiers.get(&id).ok_or(ApiError::DossierNotFound(id))?;
    authorize(record, DossierAction::View, &actor)?;
    Ok(Json(record.view_for(&actor)))
}

#[derive(Debug, Deserialize)]
pub struct UpdateDossier {
    pub name: Option<String>,
    /// When present, replaces the metadata map wholesale. Fetch, extend,
    /// send back — fields stay fully user-defined (CAPTURE-10).
    pub metadata: Option<BTreeMap<String, String>>,
}

/// `PATCH /dossiers/{id}` — rename or reshape the dossier's own metadata.
pub async fn update(
    State(state): State<Arc<AppState>>,
    Principal(actor): Principal,
    Path(id): Path<DossierId>,
    Json(body): Json<UpdateDossier>,
) -> Result<Json<DossierView>, ApiError> {
    let mut dossiers = state.dossiers.write().expect("dossiers lock poisoned");
    let record = dossiers.get_mut(&id).ok_or(ApiError::DossierNotFound(id))?;
    authorize(record, DossierAction::Edit, &actor)?;

    if let Some(name) = body.name {
        record.name = name;
    }
    if let Some(metadata) = body.metadata {
        record.metadata = metadata;
    }
    record.updated_at = Timestamp::now();

    Ok(Json(record.view_for(&actor)))
}

/// `PUT /dossiers/{id}/acl` — replace who may view, edit, and manage.
///
/// This is how teams administer their own areas (ACCESS-09): the owner (or
/// anyone granted `manage`) hands rights to named users and groups.
pub async fn replace_acl(
    State(state): State<Arc<AppState>>,
    Principal(actor): Principal,
    Path(id): Path<DossierId>,
    Json(acl): Json<DossierAcl>,
) -> Result<Json<DossierView>, ApiError> {
    let mut dossiers = state.dossiers.write().expect("dossiers lock poisoned");
    let record = dossiers.get_mut(&id).ok_or(ApiError::DossierNotFound(id))?;
    authorize(record, DossierAction::Manage, &actor)?;

    record.acl = acl;
    record.updated_at = Timestamp::now();

    Ok(Json(record.view_for(&actor)))
}

#[derive(Debug, Deserialize)]
pub struct AddEntry {
    pub reference: EntryReference,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub access: Option<Vec<Trustee>>,
}

/// `POST /dossiers/{id}/entries` — file a reference into the dossier.
///
/// Document references require the caller to be allowed to *view* that
/// document right now — nobody files documents they cannot see. The same
/// document may be filed into any number of dossiers, but only once per
/// dossier (it is a reference, not a copy — STORE-09).
pub async fn add_entry(
    State(state): State<Arc<AppState>>,
    Principal(actor): Principal,
    Path(id): Path<DossierId>,
    Json(body): Json<AddEntry>,
) -> Result<(StatusCode, Json<DossierView>), ApiError> {
    if let EntryReference::Document(doc_id) = body.reference {
        let documents = state.documents.read().expect("documents lock poisoned");
        let document = documents
            .get(&doc_id)
            .ok_or(ApiError::DocumentNotFound(doc_id))?;
        let policy = state.policy.read().expect("policy lock poisoned");
        let is_doc_owner = document.owner == actor.user;
        if !policy.allows(document.status, DocumentAction::View, &actor, is_doc_owner) {
            return Err(ApiError::DocumentNotFound(doc_id));
        }
    }

    let mut dossiers = state.dossiers.write().expect("dossiers lock poisoned");
    let record = dossiers.get_mut(&id).ok_or(ApiError::DossierNotFound(id))?;
    authorize(record, DossierAction::Edit, &actor)?;

    if let EntryReference::Document(doc_id) = body.reference {
        let already_filed = record
            .entries
            .iter()
            .any(|entry| entry.reference == EntryReference::Document(doc_id));
        if already_filed {
            return Err(ApiError::DocumentAlreadyFiled {
                document: doc_id,
                dossier: id,
            });
        }
    }

    let now = Timestamp::now();
    record.entries.push(DossierEntry {
        id: DossierEntryId::new(),
        reference: body.reference,
        added_by: actor.user.clone(),
        added_at: now,
        note: body.note,
        access: body.access,
    });
    record.updated_at = now;

    Ok((StatusCode::CREATED, Json(record.view_for(&actor))))
}

/// `DELETE /dossiers/{id}/entries/{entry_id}` — take a reference out of the
/// file. The referenced document itself is untouched, as are its references
/// in other dossiers.
pub async fn remove_entry(
    State(state): State<Arc<AppState>>,
    Principal(actor): Principal,
    Path((id, entry_id)): Path<(DossierId, DossierEntryId)>,
) -> Result<StatusCode, ApiError> {
    let mut dossiers = state.dossiers.write().expect("dossiers lock poisoned");
    let record = dossiers.get_mut(&id).ok_or(ApiError::DossierNotFound(id))?;
    authorize(record, DossierAction::Edit, &actor)?;

    let before = record.entries.len();
    record.entries.retain(|entry| entry.id != entry_id);
    if record.entries.len() == before {
        return Err(ApiError::EntryNotFound(entry_id));
    }
    record.updated_at = Timestamp::now();

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct SetEntryAccess {
    /// The new per-entry access list; `null` lifts the restriction so the
    /// dossier's `view` rule decides alone.
    pub access: Option<Vec<Trustee>>,
}

/// `PUT /dossiers/{id}/entries/{entry_id}/access` — narrow or lift who sees
/// one entry (granular per-document permissions inside a dossier,
/// ACCESS-09). Requires `manage`, like the dossier ACL itself.
pub async fn set_entry_access(
    State(state): State<Arc<AppState>>,
    Principal(actor): Principal,
    Path((id, entry_id)): Path<(DossierId, DossierEntryId)>,
    Json(body): Json<SetEntryAccess>,
) -> Result<Json<DossierView>, ApiError> {
    let mut dossiers = state.dossiers.write().expect("dossiers lock poisoned");
    let record = dossiers.get_mut(&id).ok_or(ApiError::DossierNotFound(id))?;
    authorize(record, DossierAction::Manage, &actor)?;

    let entry = record
        .entries
        .iter_mut()
        .find(|entry| entry.id == entry_id)
        .ok_or(ApiError::EntryNotFound(entry_id))?;
    entry.access = body.access;
    record.updated_at = Timestamp::now();

    Ok(Json(record.view_for(&actor)))
}
