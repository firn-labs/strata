//! Shared domain types used across all Strata services.
//!
//! Anything that crosses a service boundary (IDs, health payloads,
//! common metadata shapes) lives here so the server and the workflow
//! engine always agree on the wire format.

mod classification;
mod dossier;
mod policy;
mod retention;
mod status;

pub use classification::{
    BackendLocation, BlobPlacement, ClassificationChange, Confidentiality, PlacementDecision,
    PlacementPolicy, PlacementRule,
};
pub use dossier::{
    DossierAcl, DossierAction, DossierEntry, DossierEntryId, DossierId, EntryReference,
};
pub use policy::{Actor, DocumentAction, StatusPolicy, Trustee};
pub use retention::{
    CertificateId, DeletionCertificate, DeletionTrigger, ExpiryAction, RetentionDeadline,
    RetentionNotification, RetentionPlan, RetentionRule, RetentionSource,
};
pub use status::{DocumentStatus, StatusChange, StatusChangedEvent};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier of a document managed by Strata.
///
/// Documents are identified by ID, never by storage path — the path is an
/// implementation detail of the configured storage provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DocumentId(pub Uuid);

impl DocumentId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for DocumentId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for DocumentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Payload returned by every service's `/healthz` endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Health {
    pub service: &'static str,
    pub version: &'static str,
    pub status: HealthStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    Ok,
    Degraded,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_ids_are_unique() {
        assert_ne!(DocumentId::new(), DocumentId::new());
    }

    #[test]
    fn document_id_serializes_as_plain_uuid_string() {
        let id = DocumentId::new();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{id}\""));
    }
}
