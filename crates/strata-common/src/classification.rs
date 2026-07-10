//! Confidentiality classification and storage placement (STORE-04).
//!
//! Every document carries exactly one [`Confidentiality`] tier as metadata
//! (CAPTURE-10). The storage layer never decides placement ad hoc: a
//! [`PlacementPolicy`] derives, from the tier and a backend's
//! [`BackendLocation`], whether the backend may hold the blob at all and
//! whether the bytes must be encrypted with the operator-owned key before
//! they are handed to it.
//!
//! One rule is not policy but invariant: bytes that reach *external*
//! infrastructure are always encrypted at rest (STORE-04). A policy can
//! forbid external placement per tier, but it cannot allow plaintext there.
//!
//! Like [`StatusPolicy`](crate::StatusPolicy), the placement policy is plain
//! serializable data administered through the server's API;
//! [`PlacementPolicy::baseline`] is only the shipped starting point.
//! Department-held keys (STORE-05) are a later, separate step.

use std::collections::BTreeMap;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// Confidentiality tier of a document, lowest to highest (STORE-04).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidentiality {
    /// May be published; no protection requirements.
    Public,
    /// Regular business documents; the default for new documents.
    Internal,
    /// Restricted audience; encrypted at rest everywhere.
    Confidential,
    /// Must never leave operator-controlled infrastructure.
    StrictlyConfidential,
}

impl Confidentiality {
    /// All tiers, lowest to highest.
    pub const ALL: [Confidentiality; 4] = [
        Confidentiality::Public,
        Confidentiality::Internal,
        Confidentiality::Confidential,
        Confidentiality::StrictlyConfidential,
    ];
}

impl std::fmt::Display for Confidentiality {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Confidentiality::Public => "public",
            Confidentiality::Internal => "internal",
            Confidentiality::Confidential => "confidential",
            Confidentiality::StrictlyConfidential => "strictly_confidential",
        };
        f.write_str(name)
    }
}

/// One applied classification change, kept in a document's history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassificationChange {
    pub from: Confidentiality,
    pub to: Confidentiality,
    /// User who requested the change.
    pub by: String,
    pub at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

/// Whose infrastructure a storage backend runs on.
///
/// This is deployment configuration, not a property of the provider type: the
/// same S3 provider is `Internal` against an operator-run MinIO and
/// `External` against a public cloud. The admin attaching a backend declares
/// where its bytes physically end up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendLocation {
    /// Operator-controlled infrastructure.
    Internal,
    /// Third-party infrastructure (public cloud, hosted object storage).
    External,
}

impl std::fmt::Display for BackendLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            BackendLocation::Internal => "internal",
            BackendLocation::External => "external",
        })
    }
}

/// Placement requirements for one confidentiality tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlacementRule {
    /// Whether blobs of this tier may live on external infrastructure at
    /// all. When they do, encryption is not up to the rule — external bytes
    /// are always encrypted (STORE-04 invariant).
    pub allow_external: bool,
    /// Whether blobs of this tier are encrypted even on internal backends.
    pub encrypt_internal: bool,
}

/// What the policy demands for a concrete (tier, backend) pairing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlacementDecision {
    /// Encrypt with the operator-owned key before handing bytes to the
    /// backend.
    pub encrypt: bool,
}

/// Placement requirements per confidentiality tier.
///
/// A tier absent from the map may be placed nowhere — deny by default,
/// allow by explicit rule, like the status policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlacementPolicy {
    pub rules: BTreeMap<Confidentiality, PlacementRule>,
}

impl PlacementPolicy {
    /// Whether a blob of `tier` may be stored on a backend at `location`,
    /// and if so, under what conditions. `None` means the backend must not
    /// hold the blob.
    pub fn decide(
        &self,
        tier: Confidentiality,
        location: BackendLocation,
    ) -> Option<PlacementDecision> {
        let rule = self.rules.get(&tier)?;
        match location {
            BackendLocation::Internal => Some(PlacementDecision {
                encrypt: rule.encrypt_internal,
            }),
            BackendLocation::External => rule.allow_external.then_some(PlacementDecision {
                // Invariant, not policy: external bytes are encrypted at
                // rest with the operator-owned key (STORE-04).
                encrypt: true,
            }),
        }
    }

    /// The policy shipped as the initial server configuration.
    ///
    /// - **Public / Internal**: any backend; plaintext on internal media.
    /// - **Confidential**: any backend, encrypted at rest everywhere.
    /// - **Strictly confidential**: internal backends only, encrypted.
    pub fn baseline() -> Self {
        use Confidentiality::*;

        let rules = BTreeMap::from([
            (
                Public,
                PlacementRule {
                    allow_external: true,
                    encrypt_internal: false,
                },
            ),
            (
                Internal,
                PlacementRule {
                    allow_external: true,
                    encrypt_internal: false,
                },
            ),
            (
                Confidential,
                PlacementRule {
                    allow_external: true,
                    encrypt_internal: true,
                },
            ),
            (
                StrictlyConfidential,
                PlacementRule {
                    allow_external: false,
                    encrypt_internal: true,
                },
            ),
        ]);

        Self { rules }
    }
}

/// Where a document's blob currently lives, as recorded on the document.
///
/// `backend` is the configured backend's name, not a path: documents are
/// addressed by ID everywhere, and the placement only says which attached
/// medium answers for the bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobPlacement {
    pub backend: String,
    pub location: BackendLocation,
    /// Whether the stored bytes are encrypted with the operator-owned key.
    pub encrypted: bool,
    /// Size of the plaintext content in bytes.
    pub size: u64,
    /// User whose upload produced the current bytes.
    pub stored_by: String,
    pub stored_at: Timestamp,
}

#[cfg(test)]
mod tests {
    use super::*;
    use BackendLocation::{External, Internal as OnPrem};
    use Confidentiality::*;

    #[test]
    fn tiers_order_from_public_to_strictly_confidential() {
        assert!(Public < Internal);
        assert!(Internal < Confidential);
        assert!(Confidential < StrictlyConfidential);
    }

    #[test]
    fn baseline_keeps_strictly_confidential_off_external_backends() {
        let policy = PlacementPolicy::baseline();
        assert_eq!(policy.decide(StrictlyConfidential, External), None);
        assert_eq!(
            policy.decide(StrictlyConfidential, OnPrem),
            Some(PlacementDecision { encrypt: true })
        );
    }

    #[test]
    fn external_placement_always_encrypts_regardless_of_rule() {
        // Even a rule with encrypt_internal: false cannot produce plaintext
        // on external infrastructure — that part is invariant.
        let policy = PlacementPolicy::baseline();
        for tier in [Public, Internal, Confidential] {
            assert_eq!(
                policy.decide(tier, External),
                Some(PlacementDecision { encrypt: true }),
                "{tier} must be encrypted externally"
            );
        }
    }

    #[test]
    fn baseline_stores_low_tiers_plaintext_internally() {
        let policy = PlacementPolicy::baseline();
        assert_eq!(
            policy.decide(Public, OnPrem),
            Some(PlacementDecision { encrypt: false })
        );
        assert_eq!(
            policy.decide(Confidential, OnPrem),
            Some(PlacementDecision { encrypt: true })
        );
    }

    #[test]
    fn unlisted_tiers_may_be_placed_nowhere() {
        let empty = PlacementPolicy {
            rules: BTreeMap::new(),
        };
        for tier in Confidentiality::ALL {
            assert_eq!(empty.decide(tier, OnPrem), None);
            assert_eq!(empty.decide(tier, External), None);
        }
    }

    #[test]
    fn policy_roundtrips_through_json_with_readable_shape() {
        let policy = PlacementPolicy::baseline();
        let json = serde_json::to_value(&policy).unwrap();
        assert_eq!(json["public"]["allow_external"], serde_json::json!(true));
        assert_eq!(
            json["strictly_confidential"]["allow_external"],
            serde_json::json!(false)
        );

        let back: PlacementPolicy = serde_json::from_value(json).unwrap();
        assert_eq!(back, policy);
    }

    #[test]
    fn confidentiality_serializes_as_snake_case_string() {
        assert_eq!(
            serde_json::to_string(&StrictlyConfidential).unwrap(),
            "\"strictly_confidential\""
        );
        let back: Confidentiality = serde_json::from_str("\"public\"").unwrap();
        assert_eq!(back, Public);
    }
}
