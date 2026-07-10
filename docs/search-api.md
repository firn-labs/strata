# Search API

The search facade (SEARCH-01 … SEARCH-05): one permission-filtered query
core behind every search mode, all exposed on one surface. A document the
caller may not view is invisible to every mode — searching never reveals
more than `GET /documents` would.

## Feeding full-text search (CAPTURE-07)

The server stores extracted text; extraction itself (OCR, PDF text layer)
is a capture-pipeline step in the workflow layer.

| Route | Purpose |
| --- | --- |
| `PUT /documents/{id}/text` | Store or replace extracted text (`{"text": "…"}`). Requires edit permission — a workflow OCR step acts as its own principal. |
| `GET /documents/{id}/text` | The stored text with `extracted_by` / `extracted_at` provenance. |

Indexing fields (CAPTURE-08) live on the document and are set at creation
or via `PATCH /documents/{id}`: `keywords` (list), `metadata` (free
key-value map), and `folder` (filing path, normalized to `/a/b/c` form).
Automated indexing and human override go through the same endpoint.

## `GET /search` — the unified endpoint (SEARCH-01/02/05)

All parameters combine freely in one request:

| Parameter | Meaning |
| --- | --- |
| `text` | Full-text words; every word must occur in the extracted text, title, or keywords (case-insensitive). |
| `filter` | Boolean filter string, see grammar below. |
| `folder` | Restrict to this folder and everything below it. |
| `created_after` / `created_before` | Registration-time range (RFC 3339). |
| `limit` | Cap returned hits; `total` still reports all matches. |

Hits come newest first. Each hit carries the document's stable `reference`,
descriptive fields, and — for text queries that matched the extracted
text — a `snippet` around the first match.

### Filter grammar (SEARCH-02)

```
type:invoice AND (team:finance OR keyword:urgent) NOT status:draft
```

- Terms are `field:value`; values with spaces are quoted
  (`title:"annual report"`). Matching is case-insensitive equality.
- Operators, loosest binding first: `OR`, `AND` (also implied between
  adjacent terms), `NOT`; parentheses group. Keywords are case-insensitive.
- Fields: `title`, `type`, `team`, `owner`, `status`, `classification`,
  `keyword`, `folder`, and `meta.<key>` for the metadata map. An unknown
  field is a `400`, not an empty result.
- The same expression exists as plain serializable data
  (`strata_common::FilterExpr`), so workflow steps can build filters as
  JSON instead of concatenating strings.

## Navigation queries (SEARCH-03)

| Route | Purpose |
| --- | --- |
| `GET /search/folders?under=/a/b` | One level of the folder tree: immediate subfolders with viewable-subtree document counts, plus documents filed directly in `under`. Omit `under` for the root. |
| `GET /search/timeline?granularity=year\|month\|day` | Histogram of matches by registration time (UTC buckets, oldest first). Accepts the same `text`/`filter`/`folder`/time-range parameters as `/search`. |

## Stable references (SEARCH-04)

Every document is addressable as `strata:doc:<uuid>` — the identity is the
ID, so references survive renames, refilings, and storage moves.

| Route | Purpose |
| --- | --- |
| `GET /refs/{reference}` | Resolve a reference to its document. Unviewable resolves as `404`, exactly like `GET /documents/{id}`. |
