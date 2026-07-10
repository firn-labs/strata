# Access

Part of the [requirements catalog](README.md). Access covers viewing,
creating, and editing documents, collaboration, permissions, and
authentication. Practitioner insight: **integration into the office tools
people already use outweighs a standalone viewer** — the best DMS is one you
barely notice.

## ACCESS-01 — Web-based document viewer · `must`

Documents render in the browser. Content is transferred as binary data over
standard interfaces, encrypted in transit, with no local copy required.
Common formats render natively; convertible formats (e.g. Markdown) are
converted for display; unsupported special formats open in the matching
desktop application.

## ACCESS-02 — Viewer functions · `should`

Once a document is loaded: fast in-document search, multi-page navigation,
and an auto-generated table of contents with jump marks.

## ACCESS-03 — Annotations · `must`

Annotations are displayed as an overlay and never modify the stored original.
Export, print, and share can include or exclude annotations.

## ACCESS-04 — Accessibility · `must`

The viewer provides a read-aloud function and a screen magnifier; the UI as a
whole meets accessibility standards.

## ACCESS-05 — Authenticity check · `later`

On opening, a document's authenticity can be verified via signatures
(distributed-ledger approaches are a research option, not a commitment).

## ACCESS-06 — Document creation · `must`

Documents can be created inside the system — blank or from templates — with
creation metadata filled automatically, user metadata added manually, and
metadata insertable into the document content itself.

## ACCESS-07 — Office-suite integration · `must`

Documents open and save in the user's office applications (desktop, web, and
mobile) via integration; saving creates a new version in Strata (STORE-11).
Web surfaces must not lose functionality compared to desktop ones.

## ACCESS-08 — Collaboration · `must`

Multiple users can work on documents and folders concurrently, backed by
version control: each state maps to a user and a time, and conflicts are
avoided, auto-merged, or resolved with user support. Commenting on documents
must exist even for teams that rarely co-edit.

## ACCESS-09 — Named access management · `must`

Access is granted to named users and groups, administered by the teams
themselves (e.g. team leads), including permission management for their own
areas. Delete/move rights are restrictable for audit safety; for highly
classified data the authorized circle is deliberately small. Granular
permissions apply inside dossiers (STORE-09).

## ACCESS-10 — Document status concept · `must`

Every document has a lifecycle status (draft → in use → archived → deletable);
permissions and workflow triggers can depend on it.

## ACCESS-11 — Authentication · `must`

Authentication happens against the operator's identity provider (SSO/OIDC)
with MFA support, so leavers are locked out centrally. Continuous
authentication is a future enhancement. Externally shared documents require a
login when they are protection-worthy (DELIVER-04).

## ACCESS-12 — Performance · `must`

Filing, searching, opening, and versioning must feel instant, under many
parallel accesses, and equally fast in web and desktop surfaces. Speed is a
lever on user motivation; capacity planning must account for OCR-heavy
corpora.
