# Preserve

Part of the [requirements catalog](README.md). Preserve is long-term,
legally compliant archiving: documents that provably cannot be tampered with,
remain readable for decades, and are deleted exactly when they must be.

## PRESERVE-01 — Revision-safe archiving · `must`

Archived documents cannot be changed or deleted (accidentally or
deliberately) for the duration of their retention period. Permitted changes
are logged completely, documents remain reproducible, and the protection
measures are demonstrably effective and **verifiable by third parties**.

## PRESERVE-02 — Tamper evidence and provenance · `must`

Cryptographic measures applied at archive time make any manipulation
detectable and provide evidence of a document's origin.

## PRESERVE-03 — Long-term formats · `should`

Documents can be converted into long-term archival formats (PDF/A-class) with
precondition checks before conversion. Archival formats retain embedded OCR
text so archived documents stay full-text searchable.

## PRESERVE-04 — Media migration strategy · `should`

The archive implements a deliberate media migration strategy (refreshment,
replication, repackaging, transformation — individually or combined) so
documents survive failing or obsolete storage media. The choice is a
documented deployment decision, not an accident.

## PRESERVE-05 — Central, accessible archive · `must`

Archiving is central, and archived documents remain searchable and
retrievable — though not everything needs instant direct access. A document
can be archived when collaboration on it ends (status-driven, see ACCESS-10).

## PRESERVE-06 — Deletion deadlines · `must`

Documents can be tagged with a deletion date at archive time **or later**
(the deadline is often unknown initially). Until the date is reached, deletion
is blocked. Standard deadlines are definable per document type and per team,
kept in a retention plan that is reviewed against legal changes.

## PRESERVE-07 — Deletion execution · `must`

When a deadline expires, the system either deletes automatically or notifies
the responsible person — configurable per document class.

## PRESERVE-08 — Deletion evidence · `must`

Every deletion produces a deletion certificate, and the system keeps a
deletion history for deletion-obligated documents.

## PRESERVE-09 — Auditability of the archive · `should`

Operating the archive in a legally compliant way requires audit processes and
procedural documentation; the system supports this with exports of its
protection measures, logs, and configuration.
