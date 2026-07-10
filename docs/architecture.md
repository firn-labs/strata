# Strata architecture

Strata implements the classic ECM lifecycle — **Capture → Store → Access → Workflow →
Search → Deliver → Preserve** — as three independently deployable layers. The founding
requirements come from a bachelor thesis on customizable document management; the
structured requirements catalog distilled from it lives in `docs/requirements/`.

That research validates this exact shape: its core finding is that an
organization-wide DMS needs *functional segmentation* into user interface, logic
(expressed entirely as workflows), and storage management, connected only through
APIs. One rule follows and binds all server work: **every core capability must be
exposed via API and callable by the workflow layer — nothing may be UI-only.**

## The three layers

### 1. Core server (`strata-server`) — "Speicherverwaltung" and system of record

The only component with access to storage media. Responsibilities:

- **Storage abstraction** (`strata-storage`): the `StorageProvider` trait hides the
  medium. Local filesystem (covers host-mounted NFS/SMB) exists; S3-compatible object
  storage and native SMB are planned. Admins configure providers at runtime; an
  ILM/media-migration strategy can later distribute documents across providers.
- **Documents & metadata**: IDs, custom metadata fields (per department, user-defined),
  document classes, E-Akte groupings (a document may appear in several files).
- **Versioning & history**: every change creates a version attributed to user + time;
  access and change logging with classification-dependent depth.
- **AuthN/AuthZ**: SSO/OIDC via the company identity provider, MFA-capable; permissions
  by user/group, document status, and granular rights within an E-Akte.
- **Retention & preservation (Preserve)**: deletion deadlines per document type or
  department, deletion evidence (Löschnachweis), tamper-evident audit trail, long-term
  format conversion hooks.
- **Search**: full-text (OCR-fed), filters, categories, boolean queries — exposed as an
  API the frontend builds visual search experiences on.

### 2. Workflow engine (`strata-workflow`) — the routing brain

Departments automate document handling without code:

- Flows are **JSON graph definitions** (trigger / step / condition nodes + edges) —
  see `crates/strata-workflow/src/flow.rs`. The same structure powers the graphical,
  textual, and semantic representations.
- The **visual editor lives in the frontend** (Svelte Flow); this service stores,
  validates, versions, exports, and executes definitions.
- Every execution is **logged step-by-step** (Protokollierung während der Ausführung).
- Flow permissions: who may create, edit, and run each flow.
- Steps call the core server's API for all document operations; integration steps can
  call external tools (e.g. Stirling PDF) over HTTP.

### 3. Frontend (`frontend/`) — "Benutzeroberfläche"

SvelteKit + Svelte 5 + Tailwind v4. Three headline capabilities:

- **Interface builder**: teams compose their own views (dashboards, capture forms,
  E-Akte layouts) from prebuilt blocks without coding — with an escape hatch to real
  code. Candidate foundation: GrapesJS; decision pending a prototype.
- **Flow editor**: drag-and-drop workflow building on Svelte Flow (xyflow).
- **Document viewer/editor**: embedded via the **WOPI protocol** so Collabora Online
  or ONLYOFFICE render and edit office formats; annotations overlay the document
  without altering the stored original; PDF tooling via Stirling PDF.

## Deployment

Everything ships as containers (`deploy/`): `strata-server`, `strata-workflow`,
`frontend` (SvelteKit node adapter), plus optional `collabora` and `stirling-pdf`
sidecars. A desktop companion app may come later; the server is the product.

## Decisions log

| # | Decision | Why |
|---|---|---|
| 1 | Monorepo | Tightly coupled APIs at this stage; split later if needed |
| 2 | Rust for both backend services | One toolchain, shared types via `strata-common`, performance |
| 3 | AGPL-3.0-or-later | Server product; network copyleft keeps hosted forks open |
| 4 | SvelteKit frontend | Team expertise, Svelte Flow available, light runtime |
| 5 | WOPI for office documents | Battle-tested (Nextcloud model); avoids building an editor |
| 6 | API-first: every server capability callable by the workflow layer | Logic lives in workflows (founding research); UI-only features would break that model |
