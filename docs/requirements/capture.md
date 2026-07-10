# Capture

Part of the [requirements catalog](README.md). Capture covers every way a
document enters Strata — digital-first: most inbound documents are digital,
and the scan pipeline is deliberately secondary.

## CAPTURE-01 — Drag & drop filing · `must`

Documents are filed through the UI via drag & drop. At filing time the system
prompts for obligatory metadata (per STORE-13) and pre-fills everything it can
determine automatically.

## CAPTURE-02 — E-mail integration · `must`

E-mails and attachments can be filed directly from the mail client:
drag & drop into a dossier, a visible folder tree inside the client, and
ideally a mirror of client folders so an e-mail filed once lands in both
places.

## CAPTURE-03 — Import from third-party systems · `must`

Documents and their attributes can be imported from existing systems through
APIs, with the option to filter and clean data during migration instead of
copying everything blindly.

## CAPTURE-04 — Scanning · `should`

Scanned paper documents enter the system either via upload or directly from a
scanner into a DMS inbox. A standalone scan component is acceptable; a fully
automated physical-mail pipeline is explicitly not a priority.

## CAPTURE-05 — Content recognition · `should`

The system extracts text from image-based documents: OCR for machine print,
handwriting recognition where feasible (quality is language-dependent),
context-aware recognition (dictionary/reference matching), and form/mark
recognition for pre-printed forms.

## CAPTURE-06 — Barcode/QR matching · `should`

Barcodes or QR codes printed by the system (see DELIVER-08) are recognized on
scanned returns and the scan is automatically attached to the original
document as a new version.

## CAPTURE-07 — OCR enrichment of all documents · `must`

Every stored document is enriched with extracted text so it is machine-readable
and full-text searchable (feeds SEARCH-01 and PRESERVE-03).

## CAPTURE-08 — Automated indexing with human override · `must`

Indexing (metadata extraction and assignment) is automated to a high degree
and must be reliable; per-team indexing rules differ. Users can always inspect,
correct, and complete what automation produced — corrections should feed back
into recognition quality.

## CAPTURE-09 — Classification · `must`

Documents are classified into user-defined document classes, organized by
document type or business transaction. One document may belong to several
classes. Assignment uses document attributes; sender plus document type often
suffice. ML-based classification is desirable but must remain under human
control — automatic reorganization must never silently move documents or
delete classes.

## CAPTURE-10 — Free metadata definition · `must`

Teams define their own document types, metadata fields, and remarks without
central bottlenecks. The number of *mandatory* fields per class should stay
small to keep manual entry accurate. Every document carries its
classification tier (STORE-04) and lifecycle status (ACCESS-10) as metadata.
