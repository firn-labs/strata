# Strata — project conventions

Strata is an open-source (AGPL-3.0-or-later) document management system by Firn Labs.
Three layers: Rust core server (storage + system of record), Rust workflow engine,
SvelteKit frontend. See `docs/architecture.md` before making cross-layer changes.

## Layout

- `crates/strata-common` — types shared across services (wire format lives here)
- `crates/strata-storage` — `StorageProvider` trait + backends; only stores bytes, no metadata logic
- `crates/strata-server` — core API: documents, metadata, versioning, auth, audit, retention, search
- `crates/strata-workflow` — workflow engine: flow definitions (JSON graphs), execution, logging
- `frontend/` — SvelteKit + Svelte 5 + Tailwind v4 + TypeScript
- `deploy/` — Dockerfiles and compose stack
- `docs/` — architecture and requirements (distilled from the founding thesis + mind maps)

## Architecture rules

- Only `strata-server` talks to storage. The workflow engine and frontend go through its API.
- Storage providers implement `StorageProvider` and store *bytes only* — versioning,
  permissions, and metadata belong to the server.
- Documents are addressed by `DocumentId`, never by storage path.
- Types crossing a service boundary go in `strata-common`.
- Workflow definitions must stay plain serializable data (JSON graphs) — the visual
  editor, textual view, and engine all consume the same structure.

## Checks (run before every push)

```bash
cargo fmt --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
cd frontend && npm run lint && npm run check && npm run test:unit -- --run
```

## Git workflow

- Feature branch per issue; branch from `main`.
- PRs land via **"Rebase and merge"** — keep linear history. Resolve conflicts by
  rebasing onto `main` and force-pushing with `--force-with-lease`; never merge `main` in.
- One issue = one PR, even if the issue has multiple phases.

## Frontend conventions

- Svelte 5 runes. Careful with logging inside `$effect`/`$derived` — reads become
  reactive dependencies; only log `$state` after the first `await`.
- Check the icon registry before adding new SVG icon files.
