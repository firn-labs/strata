//! Electronic files / dossiers ("E-Akte", STORE-09, STORE-10).
//!
//! A dossier groups documents by business context. Documents are stored once
//! and only *referenced* — one document may appear in any number of dossiers
//! without duplication (STORE-09). A dossier can also reference physical
//! records or records held in third-party systems, so the file stays complete
//! even when not everything lives in Strata (STORE-10).
//!
//! Dossiers carry their own access rules: a [`DossierAcl`] grants view, edit,
//! and manage rights to named users and groups (ACCESS-09), and every entry
//! may additionally carry its own access list narrowing who sees that entry
//! inside the dossier (granular per-document permissions, ACCESS-09 ×
//! STORE-09).

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::DocumentId;
use crate::policy::{Actor, Trustee};

/// Unique identifier of a dossier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DossierId(pub Uuid);

impl DossierId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for DossierId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for DossierId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Unique identifier of one entry inside a dossier.
///
/// Entries need their own identity because the same document may be
/// referenced by many dossiers — removing or restricting "this reference
/// here" must not touch the document or its other references.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DossierEntryId(pub Uuid);

impl DossierEntryId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for DossierEntryId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for DossierEntryId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// What a dossier entry points at.
///
/// Serializes to `{"document": "<id>"}` or
/// `{"external": {"label": "...", "location": "..."}}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryReference {
    /// A document managed by Strata, referenced — never copied (STORE-09).
    Document(DocumentId),
    /// A physical record or one held in a third-party system (STORE-10).
    External {
        /// What the record is, e.g. "signed original, filing cabinet 3".
        label: String,
        /// Where to find it: a shelf mark, an URL, a foreign system's ID.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        location: Option<String>,
    },
}

/// One entry of a dossier: a reference plus its dossier-local context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DossierEntry {
    pub id: DossierEntryId,
    pub reference: EntryReference,
    /// User who added the entry.
    pub added_by: String,
    pub added_at: Timestamp,
    /// Why this record belongs to the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Per-entry access list (ACCESS-09): when set, only matching actors see
    /// this entry inside the dossier. `None` means the dossier's `view` rule
    /// decides alone. `Trustee::Owner` here matches the *dossier's* owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<Vec<Trustee>>,
}

impl DossierEntry {
    /// Whether `actor` may see this entry, given they can already view the
    /// dossier. `is_owner` refers to the dossier's owner.
    pub fn visible_to(&self, actor: &Actor, is_owner: bool) -> bool {
        match &self.access {
            None => true,
            Some(trustees) => actor.matches_any(trustees, is_owner),
        }
    }
}

/// An operation on a dossier that its ACL can gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DossierAction {
    /// See the dossier, its metadata, and its (visible) entries.
    View,
    /// Add and remove entries, change name and metadata.
    Edit,
    /// Administer access: the dossier ACL and per-entry access lists
    /// (ACCESS-09: teams administer their own areas, e.g. team leads).
    Manage,
}

/// Who may do what with a dossier (ACCESS-09).
///
/// Deny by default: an empty list denies the action to everyone but — for
/// `manage` only — the owner, who can always administer access so a dossier
/// can never be orphaned by a bad ACL update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DossierAcl {
    pub view: Vec<Trustee>,
    pub edit: Vec<Trustee>,
    pub manage: Vec<Trustee>,
}

impl DossierAcl {
    /// The ACL a freshly created dossier starts with: private to its owner.
    pub fn private_to_owner() -> Self {
        Self {
            view: vec![Trustee::Owner],
            edit: vec![Trustee::Owner],
            manage: vec![Trustee::Owner],
        }
    }

    /// Whether `actor` may perform `action`. `is_owner` says whether the
    /// actor owns the dossier in question.
    pub fn allows(&self, action: DossierAction, actor: &Actor, is_owner: bool) -> bool {
        if action == DossierAction::Manage && is_owner {
            return true;
        }
        let trustees = match action {
            DossierAction::View => &self.view,
            DossierAction::Edit => &self.edit,
            DossierAction::Manage => &self.manage,
        };
        actor.matches_any(trustees, is_owner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alice() -> Actor {
        Actor {
            user: "alice".into(),
            groups: vec!["accounting".into()],
        }
    }

    #[test]
    fn fresh_dossiers_are_private_to_their_owner() {
        let acl = DossierAcl::private_to_owner();
        for action in [
            DossierAction::View,
            DossierAction::Edit,
            DossierAction::Manage,
        ] {
            assert!(acl.allows(action, &alice(), true));
            assert!(!acl.allows(action, &alice(), false));
        }
    }

    #[test]
    fn the_owner_can_always_manage_even_when_the_acl_forgets_them() {
        let acl = DossierAcl {
            view: vec![],
            edit: vec![],
            manage: vec![Trustee::Group("leads".into())],
        };
        assert!(acl.allows(DossierAction::Manage, &alice(), true));
        assert!(!acl.allows(DossierAction::View, &alice(), true));
        assert!(!acl.allows(DossierAction::Manage, &alice(), false));
    }

    #[test]
    fn acl_grants_match_named_users_and_groups() {
        let acl = DossierAcl {
            view: vec![Trustee::Group("accounting".into())],
            edit: vec![Trustee::User("bob".into())],
            manage: vec![],
        };
        assert!(acl.allows(DossierAction::View, &alice(), false));
        assert!(!acl.allows(DossierAction::Edit, &alice(), false));
    }

    #[test]
    fn entries_without_access_list_follow_the_dossier() {
        let entry = DossierEntry {
            id: DossierEntryId::new(),
            reference: EntryReference::Document(DocumentId::new()),
            added_by: "alice".into(),
            added_at: Timestamp::UNIX_EPOCH,
            note: None,
            access: None,
        };
        assert!(entry.visible_to(&alice(), false));
    }

    #[test]
    fn restricted_entries_are_only_visible_to_listed_trustees() {
        let mut entry = DossierEntry {
            id: DossierEntryId::new(),
            reference: EntryReference::Document(DocumentId::new()),
            added_by: "alice".into(),
            added_at: Timestamp::UNIX_EPOCH,
            note: None,
            access: Some(vec![Trustee::User("bob".into()), Trustee::Owner]),
        };
        assert!(!entry.visible_to(&alice(), false), "alice is not listed");
        assert!(entry.visible_to(&alice(), true), "matched as dossier owner");

        entry.access = Some(vec![Trustee::Group("accounting".into())]);
        assert!(entry.visible_to(&alice(), false), "matched via group");
    }

    #[test]
    fn entry_references_serialize_tagged_by_kind() {
        let doc_id = DocumentId::new();
        let json = serde_json::to_value(EntryReference::Document(doc_id)).unwrap();
        assert_eq!(json, serde_json::json!({ "document": doc_id.to_string() }));

        let json = serde_json::to_value(EntryReference::External {
            label: "signed original".into(),
            location: Some("cabinet 3".into()),
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "external": { "label": "signed original", "location": "cabinet 3" }
            })
        );

        let back: EntryReference =
            serde_json::from_value(serde_json::json!({ "document": doc_id.to_string() })).unwrap();
        assert_eq!(back, EntryReference::Document(doc_id));
    }
}
