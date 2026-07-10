//! The search facade (SEARCH-01 … SEARCH-05).
//!
//! One query core powers every search mode, and every mode is exposed on
//! the same `/search` surface (SEARCH-05) so finding a document never
//! depends on knowing which corner of the system to ask:
//!
//! - full text over extracted document text, titles, and keywords
//!   (SEARCH-01, fed by CAPTURE-07 via `PUT /documents/{id}/text`),
//! - boolean `field:value` filter strings (SEARCH-02, grammar in
//!   `strata_common::FilterExpr`),
//! - folder-tree and time-based navigation (SEARCH-03) via `/search/folders`
//!   and `/search/timeline`,
//! - stable `strata:doc:<uuid>` references, resolvable at `/refs/{reference}`
//!   (SEARCH-04); every hit carries its reference.
//!
//! Everything here is read-only and permission-filtered: a document the
//! caller may not view is invisible to every mode, exactly as in
//! `GET /documents`.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde::{Deserialize, Serialize};
use strata_common::{
    Actor, Confidentiality, DocumentAction, DocumentId, DocumentRef, DocumentStatus, FilterExpr,
};

use crate::AppState;
use crate::documents::{DocumentRecord, normalize_folder};
use crate::error::ApiError;
use crate::identity::Principal;

/// One search result. Carries the document's stable reference (SEARCH-04)
/// and enough descriptive fields to render a result list without a second
/// round-trip; the full record stays behind `GET /documents/{id}`.
#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub reference: DocumentRef,
    pub id: DocumentId,
    pub title: String,
    pub folder: Option<String>,
    pub doc_type: Option<String>,
    pub team: Option<String>,
    pub status: DocumentStatus,
    pub classification: Confidentiality,
    pub keywords: Vec<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    /// Extract of the document text around the first full-text match; only
    /// set for text queries that matched the extracted text.
    pub snippet: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SearchResponse {
    /// Matches before `limit` was applied.
    pub total: usize,
    pub hits: Vec<SearchHit>,
}

/// Parameters of `GET /search` — all modes combine freely in one request
/// (SEARCH-05): full text, boolean filter, folder subtree, and time range.
#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    /// Full-text words (SEARCH-01); every word must occur in the document's
    /// extracted text, title, or keywords, case-insensitively.
    pub text: Option<String>,
    /// Boolean filter string (SEARCH-02), e.g.
    /// `type:invoice AND (team:finance OR keyword:urgent)`.
    pub filter: Option<String>,
    /// Restrict to documents filed in this folder or below (SEARCH-03).
    pub folder: Option<String>,
    /// Only documents registered at or after this instant (SEARCH-03).
    pub created_after: Option<Timestamp>,
    /// Only documents registered at or before this instant (SEARCH-03).
    pub created_before: Option<Timestamp>,
    /// Cap the number of returned hits; `total` still reports all matches.
    pub limit: Option<usize>,
}

/// The compiled form of a query, shared by `/search` and `/search/timeline`.
struct Criteria {
    words: Vec<String>,
    filter: Option<FilterExpr>,
    folder: Option<String>,
    created_after: Option<Timestamp>,
    created_before: Option<Timestamp>,
}

impl Criteria {
    fn compile(
        text: Option<&str>,
        filter: Option<&str>,
        folder: Option<&str>,
        created_after: Option<Timestamp>,
        created_before: Option<Timestamp>,
    ) -> Result<Self, ApiError> {
        let words = text
            .map(|t| t.split_whitespace().map(str::to_lowercase).collect())
            .unwrap_or_default();
        let filter = filter
            .map(FilterExpr::parse)
            .transpose()
            .map_err(|e| ApiError::InvalidFilter(e.0))?;
        let folder = folder.map(normalize_folder).transpose()?;
        Ok(Self {
            words,
            filter,
            folder,
            created_after,
            created_before,
        })
    }

    /// Whether `record` matches; `text` is its extracted full text, if any.
    fn matches(&self, record: &DocumentRecord, text: Option<&str>) -> Result<bool, ApiError> {
        if let Some(folder) = &self.folder {
            let filed_below = record
                .folder
                .as_ref()
                .is_some_and(|f| f == folder || f.starts_with(&format!("{folder}/")));
            if !filed_below {
                return Ok(false);
            }
        }
        if let Some(after) = self.created_after
            && record.created_at < after
        {
            return Ok(false);
        }
        if let Some(before) = self.created_before
            && record.created_at > before
        {
            return Ok(false);
        }
        if let Some(filter) = &self.filter {
            let matched = filter
                .matches(&|field| field_values(record, field))
                .map_err(|unknown| {
                    ApiError::InvalidFilter(format!(
                        "unknown field '{}'; expected one of title, type, team, owner, \
                         status, classification, keyword, folder, or meta.<key>",
                        unknown.0
                    ))
                })?;
            if !matched {
                return Ok(false);
            }
        }
        if !self.words.is_empty() {
            let title = record.title.to_lowercase();
            let text = text.map(str::to_lowercase);
            let keywords: Vec<String> = record.keywords.iter().map(|k| k.to_lowercase()).collect();
            let all_found = self.words.iter().all(|word| {
                title.contains(word)
                    || text.as_deref().is_some_and(|t| t.contains(word))
                    || keywords.iter().any(|k| k.contains(word))
            });
            if !all_found {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

/// The document's values for a filterable field, or `None` for a field the
/// filter language does not know. `meta.<key>` reads the metadata map; a
/// missing key is an empty value set, not an unknown field.
fn field_values(record: &DocumentRecord, field: &str) -> Option<Vec<String>> {
    match field {
        "title" => Some(vec![record.title.clone()]),
        "type" | "doc_type" => Some(record.doc_type.clone().into_iter().collect()),
        "team" => Some(record.team.clone().into_iter().collect()),
        "owner" => Some(vec![record.owner.clone()]),
        "status" => Some(vec![record.status.to_string()]),
        "classification" => Some(vec![record.classification.to_string()]),
        "keyword" | "keywords" => Some(record.keywords.clone()),
        "folder" => Some(record.folder.clone().into_iter().collect()),
        _ => field
            .strip_prefix("meta.")
            .map(|key| record.metadata.get(key).cloned().into_iter().collect()),
    }
}

/// All documents the caller may view, matched against `criteria`, newest
/// first. The single query core behind every search mode.
fn run_query(
    state: &AppState,
    actor: &Actor,
    criteria: &Criteria,
) -> Result<Vec<DocumentRecord>, ApiError> {
    let documents = state.documents.read().expect("documents lock poisoned");
    let policy = state.policy.read().expect("policy lock poisoned");
    let texts = state.texts.read().expect("texts lock poisoned");

    let mut matches = Vec::new();
    for record in documents.values() {
        let is_owner = record.owner == actor.user;
        if !policy.allows(record.status, DocumentAction::View, actor, is_owner) {
            continue;
        }
        let text = texts.get(&record.id).map(|t| t.text.as_str());
        if criteria.matches(record, text)? {
            matches.push(record.clone());
        }
    }
    matches.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.id.0.cmp(&b.id.0))
    });
    Ok(matches)
}

fn to_hit(state: &AppState, record: DocumentRecord, words: &[String]) -> SearchHit {
    let snippet = words.first().and_then(|word| {
        let texts = state.texts.read().expect("texts lock poisoned");
        texts.get(&record.id).and_then(|t| snippet(&t.text, word))
    });
    SearchHit {
        reference: DocumentRef::new(record.id),
        id: record.id,
        title: record.title,
        folder: record.folder,
        doc_type: record.doc_type,
        team: record.team,
        status: record.status,
        classification: record.classification,
        keywords: record.keywords,
        created_at: record.created_at,
        updated_at: record.updated_at,
        snippet,
    }
}

/// Extract of `text` around the first case-insensitive occurrence of `word`.
fn snippet(text: &str, word: &str) -> Option<String> {
    const CONTEXT: usize = 60;
    let start = text.to_lowercase().find(word)?;
    let from = floor_char_boundary(text, start.saturating_sub(CONTEXT));
    let to = floor_char_boundary(text, (start + word.len() + CONTEXT).min(text.len()));
    let mut out = String::new();
    if from > 0 {
        out.push('…');
    }
    out.push_str(text[from..to].trim());
    if to < text.len() {
        out.push('…');
    }
    Some(out)
}

/// Largest char boundary at or below `index`. Lowercasing can shift byte
/// offsets for non-ASCII text, so a position found in the lowercased copy
/// must be clamped before slicing the original.
fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// `GET /search` — the unified search endpoint (SEARCH-01/02/05).
pub async fn search(
    State(state): State<Arc<AppState>>,
    Principal(actor): Principal,
    Query(query): Query<SearchQuery>,
) -> Result<Json<SearchResponse>, ApiError> {
    let criteria = Criteria::compile(
        query.text.as_deref(),
        query.filter.as_deref(),
        query.folder.as_deref(),
        query.created_after,
        query.created_before,
    )?;
    let mut matches = run_query(&state, &actor, &criteria)?;

    let total = matches.len();
    if let Some(limit) = query.limit {
        matches.truncate(limit);
    }
    let hits = matches
        .into_iter()
        .map(|record| to_hit(&state, record, &criteria.words))
        .collect();

    Ok(Json(SearchResponse { total, hits }))
}

#[derive(Debug, Deserialize)]
pub struct FoldersQuery {
    /// Folder to list; omit for the root of the filing structure.
    pub under: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Subfolder {
    pub path: String,
    pub name: String,
    /// Viewable documents filed in this folder or anywhere below it.
    pub documents: usize,
}

#[derive(Debug, Serialize)]
pub struct FolderListing {
    pub folder: String,
    pub subfolders: Vec<Subfolder>,
    /// Documents filed directly in `folder` (not in subfolders).
    pub documents: Vec<SearchHit>,
}

/// `GET /search/folders[?under=/a/b]` — one level of the folder tree
/// (SEARCH-03): immediate subfolders with subtree counts, plus the documents
/// filed right here. Counts only include what the caller may view, so the
/// tree leaks nothing the result list would not show.
pub async fn folders(
    State(state): State<Arc<AppState>>,
    Principal(actor): Principal,
    Query(query): Query<FoldersQuery>,
) -> Result<Json<FolderListing>, ApiError> {
    let under = query
        .under
        .as_deref()
        .map(normalize_folder)
        .transpose()?
        .unwrap_or_else(|| "/".to_string());

    let criteria = Criteria::compile(None, None, None, None, None)?;
    let visible = run_query(&state, &actor, &criteria)?;

    // Path prefix child folders extend: "" at the root (paths start with
    // '/'), the folder itself everywhere else.
    let prefix = if under == "/" { "" } else { under.as_str() };

    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut direct = Vec::new();
    for record in visible {
        let Some(folder) = record.folder.clone() else {
            continue;
        };
        if folder == under {
            direct.push(to_hit(&state, record, &[]));
        } else if let Some(rest) = folder.strip_prefix(prefix)
            && let Some(child) = rest.strip_prefix('/').and_then(|r| r.split('/').next())
        {
            *counts.entry(child.to_string()).or_default() += 1;
        }
    }

    let subfolders = counts
        .into_iter()
        .map(|(name, documents)| Subfolder {
            path: format!("{prefix}/{name}"),
            name,
            documents,
        })
        .collect();

    Ok(Json(FolderListing {
        folder: under,
        subfolders,
        documents: direct,
    }))
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Granularity {
    Year,
    #[default]
    Month,
    Day,
}

impl Granularity {
    fn bucket(self, at: Timestamp) -> String {
        let zoned = at.to_zoned(TimeZone::UTC);
        let format = match self {
            Granularity::Year => "%Y",
            Granularity::Month => "%Y-%m",
            Granularity::Day => "%Y-%m-%d",
        };
        zoned.strftime(format).to_string()
    }
}

/// Parameters of `GET /search/timeline` — the same criteria as `/search`
/// plus a bucket size, so a calendar view can histogram any result set.
#[derive(Debug, Deserialize)]
pub struct TimelineQuery {
    pub text: Option<String>,
    pub filter: Option<String>,
    pub folder: Option<String>,
    pub created_after: Option<Timestamp>,
    pub created_before: Option<Timestamp>,
    #[serde(default)]
    pub granularity: Granularity,
}

#[derive(Debug, Serialize)]
pub struct TimelineBucket {
    /// `2026`, `2026-07`, or `2026-07-10`, depending on granularity (UTC).
    pub period: String,
    pub count: usize,
}

/// `GET /search/timeline` — viewable matches bucketed by registration time
/// (SEARCH-03), oldest bucket first; empty buckets are omitted.
pub async fn timeline(
    State(state): State<Arc<AppState>>,
    Principal(actor): Principal,
    Query(query): Query<TimelineQuery>,
) -> Result<Json<Vec<TimelineBucket>>, ApiError> {
    let criteria = Criteria::compile(
        query.text.as_deref(),
        query.filter.as_deref(),
        query.folder.as_deref(),
        query.created_after,
        query.created_before,
    )?;
    let matches = run_query(&state, &actor, &criteria)?;

    let mut buckets: BTreeMap<String, usize> = BTreeMap::new();
    for record in &matches {
        *buckets
            .entry(query.granularity.bucket(record.created_at))
            .or_default() += 1;
    }

    Ok(Json(
        buckets
            .into_iter()
            .map(|(period, count)| TimelineBucket { period, count })
            .collect(),
    ))
}

/// `GET /refs/{reference}` — resolve a stable `strata:doc:<uuid>` reference
/// to its document (SEARCH-04). Same visibility rule as `GET /documents/{id}`:
/// a reference to a document the caller may not view reads as not found.
pub async fn resolve(
    State(state): State<Arc<AppState>>,
    Principal(actor): Principal,
    Path(raw): Path<String>,
) -> Result<Json<DocumentRecord>, ApiError> {
    let reference: DocumentRef = raw.parse().map_err(|_| ApiError::InvalidReference(raw))?;
    let id = reference.0;

    let documents = state.documents.read().expect("documents lock poisoned");
    let record = documents.get(&id).ok_or(ApiError::DocumentNotFound(id))?;
    crate::documents::authorize(&state, record, DocumentAction::View, &actor)?;
    Ok(Json(record.clone()))
}
