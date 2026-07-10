//! The retention and deletion engine (PRESERVE-06, PRESERVE-07, PRESERVE-08).
//!
//! Deadlines: a document's deletion deadline is set explicitly here (only at
//! or after archive time — `PUT /documents/{id}/retention`) or derived from
//! the retention plan when the document is archived (see
//! `documents::change_status`). The plan itself is administered via
//! `GET`/`PUT /retention/plan`, mirroring the status policy: plain data,
//! reviewable when legal requirements change.
//!
//! Deletion: `DELETE /documents/{id}` is blocked while the deadline lies in
//! the future. Expired deadlines are acted on by `POST /retention/sweep` —
//! deliberately an API call rather than a built-in timer, so the workflow
//! engine (or a cron) drives it and every run is observable and testable.
//! Per document class the sweep either deletes automatically or notifies the
//! responsible person (PRESERVE-07); without a plan rule it never destroys
//! anything on its own. Every deletion issues a certificate into the
//! queryable deletion history (PRESERVE-08).

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};
use strata_common::{
    CertificateId, DeletionCertificate, DeletionTrigger, DocumentAction, DocumentId,
    DocumentStatus, EntryReference, ExpiryAction, RetentionDeadline, RetentionNotification,
    RetentionPlan, RetentionSource,
};

use crate::AppState;
use crate::documents::{DocumentRecord, authorize};
use crate::error::ApiError;
use crate::identity::Principal;

/// A standard deadline of `days` retention, counted from `start`.
pub(crate) fn deadline_from(start: Timestamp, days: u32) -> Timestamp {
    start
        .checked_add(SignedDuration::from_hours(i64::from(days) * 24))
        .unwrap_or(Timestamp::MAX)
}

fn expired(record: &DocumentRecord, now: Timestamp) -> bool {
    record
        .retention
        .as_ref()
        .is_some_and(|deadline| deadline.delete_after <= now)
}

/// `GET /retention/plan` — the retention plan currently in force.
pub async fn plan_show(
    State(state): State<Arc<AppState>>,
    Principal(_actor): Principal,
) -> Json<RetentionPlan> {
    Json(
        state
            .retention_plan
            .read()
            .expect("retention plan lock poisoned")
            .clone(),
    )
}

/// `PUT /retention/plan` — replace the plan; takes effect immediately.
///
/// Any authenticated caller may do this for now, like the status policy;
/// restricting it to administrators comes with real roles (ACCESS-09).
pub async fn plan_replace(
    State(state): State<Arc<AppState>>,
    Principal(actor): Principal,
    Json(plan): Json<RetentionPlan>,
) -> Json<RetentionPlan> {
    tracing::info!(by = %actor.user, rules = plan.rules.len(), "retention plan replaced");
    *state
        .retention_plan
        .write()
        .expect("retention plan lock poisoned") = plan.clone();
    Json(plan)
}

#[derive(Debug, Deserialize)]
pub struct SetDeadline {
    pub delete_after: Timestamp,
}

/// `PUT /documents/{id}/retention` — set or move the deletion deadline.
///
/// Only possible at or after archive time (PRESERVE-06: the deadline is
/// often unknown initially and added later). An explicit deadline replaces
/// whatever was in force, including a plan-derived one.
pub async fn set_deadline(
    State(state): State<Arc<AppState>>,
    Principal(actor): Principal,
    Path(id): Path<DocumentId>,
    Json(body): Json<SetDeadline>,
) -> Result<Json<DocumentRecord>, ApiError> {
    let mut documents = state.documents.write().expect("documents lock poisoned");
    let record = documents
        .get_mut(&id)
        .ok_or(ApiError::DocumentNotFound(id))?;

    // Existence masking first (a caller who may not view gets 404), then the
    // lifecycle invariant: the retention clock starts at archiving, so this
    // is a 409 even for callers a policy would otherwise permit — the rule is
    // PRESERVE-06's, not the policy's. The permission check comes last.
    authorize(&state, record, DocumentAction::View, &actor)?;
    if !matches!(
        record.status,
        DocumentStatus::Archived | DocumentStatus::Deletable
    ) {
        return Err(ApiError::RetentionBeforeArchive {
            status: record.status,
        });
    }
    authorize(&state, record, DocumentAction::SetRetention, &actor)?;

    let now = Timestamp::now();
    record.retention = Some(RetentionDeadline {
        delete_after: body.delete_after,
        source: RetentionSource::Explicit,
        set_by: actor.user,
        set_at: now,
    });
    record.updated_at = now;

    Ok(Json(record.clone()))
}

/// Remove `record` for good: purge dossier entries referencing it, issue the
/// deletion certificate, and append it to the deletion history (PRESERVE-08).
///
/// Callers must have removed the record from the documents map already —
/// this function only handles the consequences.
fn record_deletion(
    state: &AppState,
    record: DocumentRecord,
    deleted_by: String,
    trigger: DeletionTrigger,
    now: Timestamp,
) -> DeletionCertificate {
    // A deletion obligation extends to the file: dossiers must not keep
    // dangling references to a destroyed document (STORE-09 references are
    // pointers, not copies — nothing to preserve once the target is gone).
    let mut dossiers = state.dossiers.write().expect("dossiers lock poisoned");
    for dossier in dossiers.values_mut() {
        dossier
            .entries
            .retain(|entry| entry.reference != EntryReference::Document(record.id));
    }
    drop(dossiers);

    let certificate = DeletionCertificate {
        id: CertificateId::new(),
        document: record.id,
        title: record.title,
        doc_type: record.doc_type,
        team: record.team,
        owner: record.owner,
        delete_after: record.retention.map(|deadline| deadline.delete_after),
        deleted_at: now,
        deleted_by,
        trigger,
    };

    state
        .deletions
        .write()
        .expect("deletions lock poisoned")
        .push(certificate.clone());
    certificate
}

/// `DELETE /documents/{id}` — destroy a document, deadline permitting.
///
/// Requires `delete` permission at the document's current status (baseline:
/// only `deletable` documents, by their owner). While the deletion deadline
/// lies in the future the request is refused (PRESERVE-06); the response is
/// the certificate that proves the deletion (PRESERVE-08).
pub async fn delete_document(
    State(state): State<Arc<AppState>>,
    Principal(actor): Principal,
    Path(id): Path<DocumentId>,
) -> Result<Json<DeletionCertificate>, ApiError> {
    let now = Timestamp::now();

    // Authorize and snapshot under the lock, destroy the blob without it
    // (storage I/O must not happen under the state lock).
    let content = {
        let documents = state.documents.read().expect("documents lock poisoned");
        let record = documents.get(&id).ok_or(ApiError::DocumentNotFound(id))?;

        authorize(&state, record, DocumentAction::Delete, &actor)?;

        if let Some(deadline) = &record.retention
            && deadline.delete_after > now
        {
            return Err(ApiError::DeletionBlocked {
                document: id,
                until: deadline.delete_after,
            });
        }
        record.content.clone()
    };

    // Bytes first, certificate second: a failing storage medium leaves the
    // record intact and the request errors — no certificate may claim a
    // deletion whose blob still exists (PRESERVE-08).
    if let Some(placement) = &content {
        crate::placement::destroy_blob(&state, id, placement).await?;
    }

    let mut documents = state.documents.write().expect("documents lock poisoned");
    let record = documents
        .remove(&id)
        .ok_or(ApiError::DocumentNotFound(id))?;
    drop(documents);

    let certificate = record_deletion(&state, record, actor.user, DeletionTrigger::Manual, now);
    Ok(Json(certificate))
}

/// What one sweep run did (PRESERVE-07).
#[derive(Debug, Serialize)]
pub struct SweepOutcome {
    /// Documents deleted automatically, as their certificates.
    pub deleted: Vec<DeletionCertificate>,
    /// Notifications issued to responsible persons this run.
    pub notified: Vec<RetentionNotification>,
}

/// `POST /retention/sweep` — act on every expired deletion deadline.
///
/// For each archived or deletable document whose deadline has passed, the
/// document class's plan rule decides (PRESERVE-07): `auto_delete` destroys
/// it with a certificate, `notify_responsible` notifies the owner — once,
/// not on every run. Documents without a matching plan rule are treated as
/// notify: the engine never destroys anything nobody configured it to.
/// Reactivated documents (back in `in_use`) are left alone.
///
/// The engine acts with its own authority here — per-user permissions do not
/// apply, only the plan does. Idempotent: a second run right after finds
/// nothing left to do.
pub async fn sweep(
    State(state): State<Arc<AppState>>,
    Principal(actor): Principal,
) -> Json<SweepOutcome> {
    let now = Timestamp::now();
    let mut outcome = SweepOutcome {
        deleted: Vec::new(),
        notified: Vec::new(),
    };

    // Scoped so the state lock is provably released before the blob
    // deletions below (storage I/O must not happen under it).
    let mut to_delete = Vec::new();
    {
        let mut documents = state.documents.write().expect("documents lock poisoned");
        let plan = state
            .retention_plan
            .read()
            .expect("retention plan lock poisoned")
            .clone();

        let due: Vec<DocumentId> = documents
            .values()
            .filter(|record| {
                matches!(
                    record.status,
                    DocumentStatus::Archived | DocumentStatus::Deletable
                ) && expired(record, now)
            })
            .map(|record| record.id)
            .collect();

        for id in due {
            let record = &documents[&id];
            let action = plan
                .applicable_rule(record.doc_type.as_deref(), record.team.as_deref())
                .map(|rule| rule.on_expiry)
                .unwrap_or(ExpiryAction::NotifyResponsible);

            match action {
                ExpiryAction::AutoDelete => {
                    to_delete.push(documents.remove(&id).expect("id came from this map"));
                }
                ExpiryAction::NotifyResponsible => {
                    let mut notifications = state
                        .notifications
                        .write()
                        .expect("notifications lock poisoned");
                    if notifications.iter().all(|n| n.document != id) {
                        let notification = RetentionNotification {
                            document: id,
                            title: record.title.clone(),
                            responsible: record.owner.clone(),
                            delete_after: record
                                .retention
                                .as_ref()
                                .expect("expired() implies a deadline")
                                .delete_after,
                            created_at: now,
                        };
                        notifications.push(notification.clone());
                        outcome.notified.push(notification);
                    }
                }
            }
        }
    }

    for record in to_delete {
        // The sweep runs unattended: a blob whose medium errors is logged
        // and the metadata deletion proceeds — the next sweep cannot retry
        // (the record is gone), so the log line is the operator's signal to
        // clean the backend up manually.
        if let Some(placement) = &record.content
            && let Err(error) = crate::placement::destroy_blob(&state, record.id, placement).await
        {
            tracing::error!(id = %record.id, %error, "sweep could not destroy blob");
        }

        let certificate = record_deletion(
            &state,
            record,
            actor.user.clone(),
            DeletionTrigger::RetentionExpiry,
            now,
        );
        outcome.deleted.push(certificate);
    }

    if !outcome.deleted.is_empty() || !outcome.notified.is_empty() {
        tracing::info!(
            by = %actor.user,
            deleted = outcome.deleted.len(),
            notified = outcome.notified.len(),
            "retention sweep acted on expired deadlines"
        );
    }
    Json(outcome)
}

/// `GET /retention/deletions` — the deletion history: every certificate ever
/// issued, oldest first (PRESERVE-08).
pub async fn deletions_list(
    State(state): State<Arc<AppState>>,
    Principal(_actor): Principal,
) -> Json<Vec<DeletionCertificate>> {
    Json(
        state
            .deletions
            .read()
            .expect("deletions lock poisoned")
            .clone(),
    )
}

/// `GET /retention/notifications` — expiry notifications issued so far,
/// oldest first (PRESERVE-07).
pub async fn notifications_list(
    State(state): State<Arc<AppState>>,
    Principal(_actor): Principal,
) -> Json<Vec<RetentionNotification>> {
    Json(
        state
            .notifications
            .read()
            .expect("notifications lock poisoned")
            .clone(),
    )
}
