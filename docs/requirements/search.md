# Search

Part of the [requirements catalog](README.md). Queries are heterogeneous, so
all search modes live on one surface — and finding a document must never
depend on one colleague's private knowledge.

## SEARCH-01 — Full-text search · `must`

The full text of all documents (OCR-fed, per CAPTURE-07) is searchable,
independent of indexing quality. Refinements: synonym matching and
logical-context matching.

## SEARCH-02 — Filter search with boolean logic · `must`

Search by keywords, categories, and metadata, freely combinable into search
strings with boolean operators.

## SEARCH-03 — Visual search · `must`

Navigate the filing structure (folder tree) and search by time via calendar
or timeline views.

## SEARCH-04 — Direct references · `must`

Every document is addressable by a stable reference (link) for direct access
from other documents, systems, and chats.

## SEARCH-05 — Unified search surface · `must`

All search modes are offered together in one overview, not scattered across
the UI.

## SEARCH-06 — Recommender · `later`

The system can suggest documents whose content is similar to a result or an
open document.
