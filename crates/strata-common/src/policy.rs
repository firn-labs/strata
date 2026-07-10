//! Status-based permissions (ACCESS-10 × ACCESS-09).
//!
//! A [`StatusPolicy`] declares, for every lifecycle status, which
//! [`DocumentAction`]s are allowed and for whom. "Whom" is expressed as
//! [`Trustee`]s — the document owner, everyone, or named users and groups
//! (ACCESS-09: access is granted to named users and groups).
//!
//! The policy is plain serializable data, administered through the server's
//! API rather than compiled in, so deployments (and later, workflows) can
//! reshape it. [`StatusPolicy::baseline`] is only the shipped starting point.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::DocumentStatus;

/// An operation on a document that permissions can gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentAction {
    /// Read the document and its metadata.
    View,
    /// Modify content or metadata.
    Edit,
    /// Destroy the document (used by the retention engine, PRESERVE-06).
    Delete,
    /// Move the document to another lifecycle status.
    ChangeStatus,
}

impl DocumentAction {
    pub const ALL: [DocumentAction; 4] = [
        DocumentAction::View,
        DocumentAction::Edit,
        DocumentAction::Delete,
        DocumentAction::ChangeStatus,
    ];
}

/// Who a permission rule applies to.
///
/// Serializes to `"owner"`, `"anyone"`, `{"user": "alice"}`, or
/// `{"group": "accounting"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trustee {
    /// The user recorded as the document's owner.
    Owner,
    /// Any authenticated user.
    Anyone,
    /// A named user (ACCESS-09).
    User(String),
    /// A named group (ACCESS-09).
    Group(String),
}

/// The identity a permission check runs against.
///
/// Deliberately identity-provider-agnostic: the server fills this from its
/// authentication layer, the policy only matches names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Actor {
    pub user: String,
    pub groups: Vec<String>,
}

impl Actor {
    fn matches(&self, trustee: &Trustee, is_owner: bool) -> bool {
        match trustee {
            Trustee::Anyone => true,
            Trustee::Owner => is_owner,
            Trustee::User(name) => *name == self.user,
            Trustee::Group(name) => self.groups.iter().any(|g| g == name),
        }
    }

    /// Whether any of `trustees` matches this actor. `is_owner` says whether
    /// the actor owns the object the trustees belong to (document, dossier).
    pub fn matches_any(&self, trustees: &[Trustee], is_owner: bool) -> bool {
        trustees.iter().any(|t| self.matches(t, is_owner))
    }
}

/// Permissions per lifecycle status: status → action → allowed trustees.
///
/// An action absent from a status's map is denied for everyone — deny by
/// default, allow by explicit rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StatusPolicy {
    pub rules: BTreeMap<DocumentStatus, BTreeMap<DocumentAction, Vec<Trustee>>>,
}

impl StatusPolicy {
    /// Whether `actor` may perform `action` on a document in `status`.
    /// `is_owner` says whether the actor owns the document in question.
    pub fn allows(
        &self,
        status: DocumentStatus,
        action: DocumentAction,
        actor: &Actor,
        is_owner: bool,
    ) -> bool {
        self.rules
            .get(&status)
            .and_then(|actions| actions.get(&action))
            .is_some_and(|trustees| actor.matches_any(trustees, is_owner))
    }

    /// The policy shipped as the initial server configuration.
    ///
    /// - **Draft**: private to the owner.
    /// - **In use**: anyone may view and edit (dossier-level ACLs narrow this
    ///   later, STORE-09); only the owner moves it on.
    /// - **Archived**: anyone may view (PRESERVE-05: archives stay
    ///   accessible), nobody edits, the owner may reactivate or release for
    ///   deletion.
    /// - **Deletable**: visible to the owner, deletable by the owner (the
    ///   retention engine acts with its own authority later, PRESERVE-06).
    pub fn baseline() -> Self {
        use DocumentAction::*;
        use DocumentStatus::*;
        use Trustee::{Anyone, Owner};

        let mut rules: BTreeMap<DocumentStatus, BTreeMap<DocumentAction, Vec<Trustee>>> =
            BTreeMap::new();

        rules.insert(
            Draft,
            BTreeMap::from([
                (View, vec![Owner]),
                (Edit, vec![Owner]),
                (ChangeStatus, vec![Owner]),
            ]),
        );
        rules.insert(
            InUse,
            BTreeMap::from([
                (View, vec![Anyone]),
                (Edit, vec![Anyone]),
                (ChangeStatus, vec![Owner]),
            ]),
        );
        rules.insert(
            Archived,
            BTreeMap::from([(View, vec![Anyone]), (ChangeStatus, vec![Owner])]),
        );
        rules.insert(
            Deletable,
            BTreeMap::from([
                (View, vec![Owner]),
                (Delete, vec![Owner]),
                (ChangeStatus, vec![Owner]),
            ]),
        );

        Self { rules }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use DocumentAction::*;
    use DocumentStatus::*;

    fn alice() -> Actor {
        Actor {
            user: "alice".into(),
            groups: vec!["accounting".into()],
        }
    }

    #[test]
    fn baseline_keeps_drafts_private_to_the_owner() {
        let policy = StatusPolicy::baseline();
        assert!(policy.allows(Draft, View, &alice(), true));
        assert!(policy.allows(Draft, Edit, &alice(), true));
        assert!(!policy.allows(Draft, View, &alice(), false));
    }

    #[test]
    fn baseline_freezes_archived_documents_but_keeps_them_readable() {
        let policy = StatusPolicy::baseline();
        assert!(policy.allows(Archived, View, &alice(), false));
        assert!(
            !policy.allows(Archived, Edit, &alice(), true),
            "not even the owner edits an archive"
        );
    }

    #[test]
    fn unlisted_actions_are_denied_by_default() {
        let policy = StatusPolicy::baseline();
        assert!(!policy.allows(InUse, Delete, &alice(), true));
        let empty = StatusPolicy {
            rules: BTreeMap::new(),
        };
        assert!(!empty.allows(InUse, View, &alice(), true));
    }

    #[test]
    fn named_user_and_group_trustees_match_by_name() {
        let mut rules = BTreeMap::new();
        rules.insert(
            InUse,
            BTreeMap::from([
                (View, vec![Trustee::Group("accounting".into())]),
                (Edit, vec![Trustee::User("bob".into())]),
            ]),
        );
        let policy = StatusPolicy { rules };

        assert!(
            policy.allows(InUse, View, &alice(), false),
            "matched via group"
        );
        assert!(
            !policy.allows(InUse, Edit, &alice(), false),
            "rule names a different user"
        );
    }

    #[test]
    fn policy_roundtrips_through_json_with_readable_shape() {
        let policy = StatusPolicy::baseline();
        let json = serde_json::to_value(&policy).unwrap();
        assert_eq!(json["draft"]["view"], serde_json::json!(["owner"]));
        assert_eq!(json["in_use"]["view"], serde_json::json!(["anyone"]));

        let back: StatusPolicy = serde_json::from_value(json).unwrap();
        assert_eq!(back, policy);
    }

    #[test]
    fn named_trustees_serialize_as_tagged_objects() {
        let json = serde_json::to_value(vec![
            Trustee::User("bob".into()),
            Trustee::Group("accounting".into()),
        ])
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!([{"user": "bob"}, {"group": "accounting"}])
        );
    }
}
