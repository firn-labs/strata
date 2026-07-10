# Contributing to Strata

Thanks for your interest! Strata is young — the best way to help right now is to
pick up an open issue or start a discussion before building something large.

## Workflow

1. Every change starts from a GitHub issue.
2. Create a feature branch from `main` (e.g. `feat/123-s3-provider`).
3. Keep the branch focused on that one issue.
4. Make sure all checks pass locally (see below).
5. Open a PR; it lands via **"Rebase and merge"** to keep history linear.
   If `main` moved, rebase your branch and force-push with `--force-with-lease` —
   don't merge `main` into your branch.

## Local checks

```bash
cargo fmt --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
cd frontend && npm run lint && npm run check && npm run test:unit -- --run
```

## License

By contributing you agree that your contributions are licensed under
AGPL-3.0-or-later.
