//! Document lifecycle status (ACCESS-10).
//!
//! Every document carries exactly one [`DocumentStatus`]. The lifecycle is a
//! forward chain — draft → in use → archived → deletable — with two deliberate
//! back-edges: an archived document can be reactivated when collaboration
//! resumes (PRESERVE-05 keeps archives retrievable, not frozen), and a
//! document marked deletable can be pulled back to archived while the
//! retention engine has not yet destroyed it. Nothing ever returns to draft,
//! and no status may be skipped.
//!
//! Status changes are the trigger surface for the workflow engine
//! (WORKFLOW-08): every applied transition is published as a
//! [`StatusChangedEvent`] on the server's event feed.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::DocumentId;

/// Lifecycle status of a document (ACCESS-10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentStatus {
    /// Being authored; not yet part of regular business.
    Draft,
    /// Actively used / collaborated on.
    InUse,
    /// Collaboration ended; kept searchable and retrievable (PRESERVE-05).
    Archived,
    /// Cleared for destruction by the retention engine (PRESERVE-06).
    Deletable,
}

impl DocumentStatus {
    /// All statuses, in lifecycle order.
    pub const ALL: [DocumentStatus; 4] = [
        DocumentStatus::Draft,
        DocumentStatus::InUse,
        DocumentStatus::Archived,
        DocumentStatus::Deletable,
    ];

    /// Statuses this one may transition to.
    pub fn allowed_transitions(self) -> &'static [DocumentStatus] {
        match self {
            DocumentStatus::Draft => &[DocumentStatus::InUse],
            DocumentStatus::InUse => &[DocumentStatus::Archived],
            DocumentStatus::Archived => &[DocumentStatus::InUse, DocumentStatus::Deletable],
            DocumentStatus::Deletable => &[DocumentStatus::Archived],
        }
    }

    pub fn can_transition_to(self, next: DocumentStatus) -> bool {
        self.allowed_transitions().contains(&next)
    }
}

impl std::fmt::Display for DocumentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            DocumentStatus::Draft => "draft",
            DocumentStatus::InUse => "in_use",
            DocumentStatus::Archived => "archived",
            DocumentStatus::Deletable => "deletable",
        };
        f.write_str(name)
    }
}

/// One applied status transition, kept in a document's history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusChange {
    pub from: DocumentStatus,
    pub to: DocumentStatus,
    /// User who requested the transition.
    pub by: String,
    pub at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

/// A status transition as published on the server's event feed.
///
/// The workflow engine polls this feed and matches events against trigger
/// nodes (WORKFLOW-08). `seq` increases strictly monotonically, so consumers
/// resume with `?after=<last seen seq>` and never miss or re-process events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusChangedEvent {
    pub seq: u64,
    pub document: DocumentId,
    pub from: DocumentStatus,
    pub to: DocumentStatus,
    pub by: String,
    pub at: Timestamp,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_moves_forward_without_skipping() {
        use DocumentStatus::*;
        assert!(Draft.can_transition_to(InUse));
        assert!(InUse.can_transition_to(Archived));
        assert!(Archived.can_transition_to(Deletable));

        // No skipping ahead.
        assert!(!Draft.can_transition_to(Archived));
        assert!(!Draft.can_transition_to(Deletable));
        assert!(!InUse.can_transition_to(Deletable));
    }

    #[test]
    fn archived_documents_can_be_reactivated_but_nothing_returns_to_draft() {
        use DocumentStatus::*;
        assert!(Archived.can_transition_to(InUse));
        assert!(Deletable.can_transition_to(Archived));

        for status in DocumentStatus::ALL {
            assert!(
                !status.can_transition_to(Draft),
                "{status} → draft must be impossible"
            );
        }
    }

    #[test]
    fn no_status_transitions_to_itself() {
        for status in DocumentStatus::ALL {
            assert!(!status.can_transition_to(status));
        }
    }

    #[test]
    fn status_serializes_as_snake_case_string() {
        assert_eq!(
            serde_json::to_string(&DocumentStatus::InUse).unwrap(),
            "\"in_use\""
        );
        let back: DocumentStatus = serde_json::from_str("\"deletable\"").unwrap();
        assert_eq!(back, DocumentStatus::Deletable);
    }
}
