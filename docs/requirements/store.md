# Store

Part of the [requirements catalog](README.md). The Store stage is the pillar
everything else rests on: where documents live, how they are versioned, and
how their integrity is guaranteed.

## STORE-01 — Central filing interface · `must`

The system presents one central interface for storing and retrieving
documents, independent of where the bytes physically live. Users and workflows
never address a storage medium directly.

## STORE-02 — Pluggable storage backends · `must`

Multiple storage media (local/network filesystems, S3-compatible object
storage, further backends) can be attached simultaneously. The storage layer
abstracts them behind a single provider interface (`StorageProvider`).

## STORE-03 — Information lifecycle management · `later`

The system distributes documents across attached media by access frequency and
a content-derived value, keeping hot documents on fast storage and cold
documents on cheap storage. Requires tight coupling between ILM logic and
document metadata.

## STORE-04 — Classification-driven placement and encryption · `must`

Every document carries a confidentiality classification. Placement policy is
derived from it: highly confidential data may be stored on external
infrastructure only after encryption with an operator-owned key; data on
external infrastructure is encrypted at rest and strictly segregated per
tenant.

## STORE-05 — Department-held encryption · `should`

Selected document sets can be encrypted such that only an authorized group can
read them — administrators operating the system must not be able to access the
plaintext during maintenance.

## STORE-06 — Data ownership and full export · `must`

The operating organization remains owner of all data. A complete export of
documents, metadata, files/dossiers, and workflow definitions must be possible
at any time (no lock-in, provider migration support).

## STORE-07 — Tamper-proof and delete-proof filing · `must`

Designated documents (e.g. original e-mails, policies, official or
tax-relevant records) can be stored so they cannot be altered or deleted,
with the protection verifiable by third parties (see PRESERVE-01).

## STORE-08 — Backup and reliable restore · `must`

The system supports consistent backups and verified, loss-free restore of
documents and metadata after a system failure.

## STORE-09 — Electronic files (dossiers) · `must`

Users can create and manage electronic files ("E-Akte") that group documents
by business context. Documents are stored independently and only *referenced*:
one document may appear in any number of files without duplication. Files
carry their own user-extendable metadata. Regulated professions may be legally
required to keep such files.

## STORE-10 — External references in dossiers · `should`

A dossier can reference physical documents and records held in third-party
systems, so the file is complete even when not everything lives in Strata.

## STORE-11 — Versioning · `must`

Every change to a document creates a version attributed to a user and a
timestamp. Versions can be compared and restored; version comments can be made
mandatory per document class. Saving through an office-suite integration and
re-importing an externally returned document each create a new version.

## STORE-12 — Audit trail (history) · `must`

Independent of versioning, the system logs accesses and changes — including
permission changes — per document. Logging depth follows the document's
classification. The audit trail is append-only; entries cannot be restored
into content.

## STORE-13 — Enforced filing rules · `must`

Teams define binding filing structures and mandatory metadata. The system
prompts for obligatory information at filing time and offers no path that
bypasses the rules.
