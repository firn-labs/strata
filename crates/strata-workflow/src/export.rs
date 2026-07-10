//! Flow export and import (WORKFLOW-07).
//!
//! Portability is a first-class feature: a system that accumulates hundreds
//! of flows becomes un-replaceable unless every definition can leave it and
//! come back intact. Exports wrap [`FlowDefinition`]s — the same JSON graphs
//! the editor saves and the engine executes (WORKFLOW-06) — in a small
//! versioned envelope, documented in `docs/flow-export-format.md`.
//!
//! Imports preserve flow ids, so references to a flow stay valid across an
//! export/import cycle, and they are all-or-nothing: the whole envelope is
//! validated (format, version, structure, id collisions) before any flow is
//! stored.

use std::collections::hash_map::Entry;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::error::ApiError;
use crate::flow::{FlowDefinition, FlowId};
use crate::identity::Principal;

/// Marker naming the export format; importers reject anything else.
pub const EXPORT_FORMAT: &str = "strata-flows";

/// Current version of the export format. Bumped only on breaking changes;
/// an importer accepts exactly the versions it knows how to read.
pub const EXPORT_VERSION: u32 = 1;

/// The versioned envelope every export produces and every import accepts.
/// A single-flow export uses the same shape, so any export is importable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowExport {
    pub format: String,
    pub version: u32,
    pub flows: Vec<FlowDefinition>,
}

impl FlowExport {
    fn wrap(flows: Vec<FlowDefinition>) -> Self {
        Self {
            format: EXPORT_FORMAT.to_owned(),
            version: EXPORT_VERSION,
            flows,
        }
    }
}

/// What to do when an imported flow's id already exists.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnConflict {
    /// Reject the whole import, leaving the engine untouched.
    #[default]
    Fail,
    /// Overwrite the existing definitions.
    Replace,
    /// Keep the existing definitions and import only the new flows.
    Skip,
}

#[derive(Debug, Default, Deserialize)]
pub struct ImportOptions {
    #[serde(default)]
    pub on_conflict: OnConflict,
}

/// Per-flow outcome of an import, so callers can verify what happened.
#[derive(Debug, Default, Serialize)]
pub struct ImportReport {
    pub imported: Vec<FlowId>,
    pub replaced: Vec<FlowId>,
    pub skipped: Vec<FlowId>,
}

/// `GET /flows/export` — every registered definition in one envelope.
///
/// Ordered by name, then id, so exporting the same engine twice yields the
/// same document — exports stay diffable and testable.
pub async fn export_all(
    State(state): State<Arc<AppState>>,
    Principal(_actor): Principal,
) -> Json<FlowExport> {
    let flows = state.flows.read().expect("flows lock poisoned");
    let mut all: Vec<_> = flows.values().cloned().collect();
    all.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.0.cmp(&b.id.0)));
    Json(FlowExport::wrap(all))
}

/// `GET /flows/{id}/export` — one flow, in the same envelope.
pub async fn export_one(
    State(state): State<Arc<AppState>>,
    Principal(_actor): Principal,
    Path(id): Path<FlowId>,
) -> Result<Json<FlowExport>, ApiError> {
    let flows = state.flows.read().expect("flows lock poisoned");
    let flow = flows.get(&id).cloned().ok_or(ApiError::FlowNotFound(id))?;
    Ok(Json(FlowExport::wrap(vec![flow])))
}

/// `POST /flows/import?on_conflict=fail|replace|skip` — import an envelope.
///
/// Validation happens up front and the import applies all-or-nothing; a
/// rejected import never leaves the engine partially updated.
pub async fn import(
    State(state): State<Arc<AppState>>,
    Principal(_actor): Principal,
    Query(options): Query<ImportOptions>,
    Json(envelope): Json<FlowExport>,
) -> Result<Json<ImportReport>, ApiError> {
    if envelope.format != EXPORT_FORMAT {
        return Err(ApiError::InvalidImport(format!(
            "unknown format {:?}; this engine imports {EXPORT_FORMAT:?}",
            envelope.format
        )));
    }
    if envelope.version != EXPORT_VERSION {
        return Err(ApiError::InvalidImport(format!(
            "unsupported format version {}; this engine reads version {EXPORT_VERSION}",
            envelope.version
        )));
    }

    let mut seen = std::collections::HashSet::new();
    for flow in &envelope.flows {
        if !seen.insert(flow.id) {
            return Err(ApiError::InvalidImport(format!(
                "flow {} appears more than once in the export",
                flow.id
            )));
        }
        flow.validate().map_err(|reason| {
            ApiError::InvalidImport(format!("flow {} ({:?}): {reason}", flow.id, flow.name))
        })?;
    }

    // Conflicts are checked under the same write lock that applies the
    // import, so a concurrent registration cannot slip in between.
    let mut flows = state.flows.write().expect("flows lock poisoned");

    if options.on_conflict == OnConflict::Fail {
        let conflicts: Vec<_> = envelope
            .flows
            .iter()
            .filter(|flow| flows.contains_key(&flow.id))
            .map(|flow| flow.id.to_string())
            .collect();
        if !conflicts.is_empty() {
            return Err(ApiError::ImportConflict(conflicts.join(", ")));
        }
    }

    let mut report = ImportReport::default();
    for flow in envelope.flows {
        let id = flow.id;
        match flows.entry(id) {
            Entry::Vacant(slot) => {
                slot.insert(flow);
                report.imported.push(id);
            }
            Entry::Occupied(_) if options.on_conflict == OnConflict::Skip => {
                report.skipped.push(id);
            }
            Entry::Occupied(mut slot) => {
                slot.insert(flow);
                report.replaced.push(id);
            }
        }
    }
    Ok(Json(report))
}
