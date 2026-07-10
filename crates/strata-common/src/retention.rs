//! Retention and deletion (PRESERVE-06, PRESERVE-07, PRESERVE-08).
//!
//! Archived documents carry an optional [`RetentionDeadline`]: until
//! `delete_after` is reached, deletion is blocked. The deadline is set
//! explicitly (at archive time **or later** — it is often unknown initially)
//! or derived from the [`RetentionPlan`], which defines standard deadlines
//! per document type and per team. The plan is plain serializable data,
//! administered over the server's API like the status policy, so it can be
//! reviewed against legal changes without redeploying.
//!
//! What happens when a deadline expires is configured per document class via
//! [`ExpiryAction`]: the engine either deletes automatically or notifies the
//! responsible person (PRESERVE-07). Every deletion — manual or automatic —
//! produces a [`DeletionCertificate`] and is recorded in the server's
//! deletion history (PRESERVE-08).

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::DocumentId;

/// What the engine does when a document's deletion deadline expires
/// (PRESERVE-07), configurable per document class through the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpiryAction {
    /// Delete the document without further human involvement.
    AutoDelete,
    /// Notify the responsible person; a human performs the deletion.
    NotifyResponsible,
}

/// One standard-deadline rule of the retention plan (PRESERVE-06).
///
/// `doc_type` and `team` are matchers: a `None` matcher accepts every
/// document, so a rule with both unset is a catch-all default. When several
/// rules match a document, the most specific one (most set matchers) wins;
/// among equally specific rules the first in the plan wins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    /// Standard retention period, counted from the moment of archiving.
    pub retain_for_days: u32,
    pub on_expiry: ExpiryAction,
}

impl RetentionRule {
    fn matches(&self, doc_type: Option<&str>, team: Option<&str>) -> bool {
        let matcher_accepts = |matcher: &Option<String>, value: Option<&str>| match (matcher, value)
        {
            (None, _) => true,
            (Some(wanted), Some(actual)) => wanted == actual,
            (Some(_), None) => false,
        };
        matcher_accepts(&self.doc_type, doc_type) && matcher_accepts(&self.team, team)
    }

    fn specificity(&self) -> u8 {
        self.doc_type.is_some() as u8 + self.team.is_some() as u8
    }
}

/// The retention plan: standard deletion deadlines per document type and per
/// team (PRESERVE-06). Kept as server state and administered via the API so
/// it can be reviewed and updated when legal requirements change.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RetentionPlan {
    pub rules: Vec<RetentionRule>,
}

impl RetentionPlan {
    /// The rule governing a document with the given type and team, if any:
    /// the most specific matching rule, earliest in the plan on ties.
    pub fn applicable_rule(
        &self,
        doc_type: Option<&str>,
        team: Option<&str>,
    ) -> Option<&RetentionRule> {
        self.rules
            .iter()
            .filter(|rule| rule.matches(doc_type, team))
            .reduce(|best, candidate| {
                if candidate.specificity() > best.specificity() {
                    candidate
                } else {
                    best
                }
            })
    }
}

/// Where a document's deletion deadline came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetentionSource {
    /// Set by a user through the API (at archive time or later).
    Explicit,
    /// Derived from the retention plan when the document was archived.
    Plan,
}

/// A document's deletion deadline (PRESERVE-06). Until `delete_after` is
/// reached, deletion of the document is blocked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionDeadline {
    pub delete_after: Timestamp,
    pub source: RetentionSource,
    /// User whose action produced the deadline (the setter, or for
    /// plan-derived deadlines the user whose archiving triggered the plan).
    pub set_by: String,
    pub set_at: Timestamp,
}

/// What caused a deletion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeletionTrigger {
    /// A user deleted the document through the API.
    Manual,
    /// The retention engine deleted it after its deadline expired
    /// (PRESERVE-07, `ExpiryAction::AutoDelete`).
    RetentionExpiry,
}

/// Unique identifier of a deletion certificate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CertificateId(pub Uuid);

impl CertificateId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for CertificateId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for CertificateId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Proof of a performed deletion (PRESERVE-08).
///
/// Issued for every deletion and kept in the server's deletion history; the
/// certificate preserves what is needed to demonstrate that a deletion
/// obligation was met after the document itself is gone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeletionCertificate {
    pub id: CertificateId,
    pub document: DocumentId,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    pub owner: String,
    /// The deadline in force at deletion time, if one was set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete_after: Option<Timestamp>,
    pub deleted_at: Timestamp,
    /// User who performed the deletion, or the caller who ran the sweep for
    /// automatic deletions.
    pub deleted_by: String,
    pub trigger: DeletionTrigger,
}

/// A pending "deadline expired, please act" notification (PRESERVE-07,
/// `ExpiryAction::NotifyResponsible`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionNotification {
    pub document: DocumentId,
    pub title: String,
    /// The person responsible for acting on the expiry — the document owner.
    pub responsible: String,
    pub delete_after: Timestamp,
    pub created_at: Timestamp,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(
        doc_type: Option<&str>,
        team: Option<&str>,
        retain_for_days: u32,
        on_expiry: ExpiryAction,
    ) -> RetentionRule {
        RetentionRule {
            doc_type: doc_type.map(str::to_owned),
            team: team.map(str::to_owned),
            retain_for_days,
            on_expiry,
        }
    }

    #[test]
    fn the_most_specific_matching_rule_wins() {
        let plan = RetentionPlan {
            rules: vec![
                rule(None, None, 365, ExpiryAction::NotifyResponsible),
                rule(Some("invoice"), None, 3650, ExpiryAction::NotifyResponsible),
                rule(
                    Some("invoice"),
                    Some("accounting"),
                    2555,
                    ExpiryAction::AutoDelete,
                ),
            ],
        };

        let both = plan.applicable_rule(Some("invoice"), Some("accounting"));
        assert_eq!(both.unwrap().retain_for_days, 2555);

        let type_only = plan.applicable_rule(Some("invoice"), Some("sales"));
        assert_eq!(type_only.unwrap().retain_for_days, 3650);

        let fallback = plan.applicable_rule(Some("memo"), None);
        assert_eq!(fallback.unwrap().retain_for_days, 365);
    }

    #[test]
    fn among_equally_specific_rules_the_first_wins() {
        let plan = RetentionPlan {
            rules: vec![
                rule(Some("invoice"), None, 10, ExpiryAction::NotifyResponsible),
                rule(
                    None,
                    Some("accounting"),
                    20,
                    ExpiryAction::NotifyResponsible,
                ),
            ],
        };
        let winner = plan.applicable_rule(Some("invoice"), Some("accounting"));
        assert_eq!(winner.unwrap().retain_for_days, 10);
    }

    #[test]
    fn a_set_matcher_rejects_documents_without_that_attribute() {
        let plan = RetentionPlan {
            rules: vec![rule(
                Some("invoice"),
                None,
                10,
                ExpiryAction::NotifyResponsible,
            )],
        };
        assert!(plan.applicable_rule(None, None).is_none());
        assert!(plan.applicable_rule(Some("memo"), None).is_none());
        assert!(plan.applicable_rule(Some("invoice"), None).is_some());
    }

    #[test]
    fn an_empty_plan_governs_nothing() {
        let plan = RetentionPlan::default();
        assert!(
            plan.applicable_rule(Some("invoice"), Some("accounting"))
                .is_none()
        );
    }

    #[test]
    fn plan_serializes_as_a_plain_rule_list() {
        let plan = RetentionPlan {
            rules: vec![rule(
                Some("invoice"),
                Some("accounting"),
                2555,
                ExpiryAction::AutoDelete,
            )],
        };
        let json = serde_json::to_value(&plan).unwrap();
        assert_eq!(
            json,
            serde_json::json!([{
                "doc_type": "invoice",
                "team": "accounting",
                "retain_for_days": 2555,
                "on_expiry": "auto_delete",
            }])
        );
        let back: RetentionPlan = serde_json::from_value(json).unwrap();
        assert_eq!(back, plan);
    }
}
