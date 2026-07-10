# Requirements catalog

The functional requirements for Strata originate from founding research on
customizable, company-wide document management systems (a qualitative study
combining ECM literature and practitioner interviews), structured along the
ECM lifecycle stages of the "House of DMS": **Capture, Preserve, Access,
Search, Deliver** and **Workflow**, all resting on the pillar **Store**.

The study's core conclusion is the reason Strata exists: a DMS suitable for
organization-wide use must be flexible at every level, and that flexibility is
achieved through **functional segmentation** into three independently adaptable
components — user interface, logic (expressed entirely as workflows), and
storage management — connected exclusively through APIs. Strata's three-layer
architecture implements exactly this; see [architecture.md](../architecture.md).

## Catalog

| Stage | File | ID prefix |
|---|---|---|
| Store | [store.md](store.md) | `STORE` |
| Capture | [capture.md](capture.md) | `CAPTURE` |
| Preserve | [preserve.md](preserve.md) | `PRESERVE` |
| Access | [access.md](access.md) | `ACCESS` |
| Search | [search.md](search.md) | `SEARCH` |
| Deliver | [deliver.md](deliver.md) | `DELIVER` |
| Workflow | [workflow.md](workflow.md) | `WORKFLOW` |

## Conventions

- **Stable IDs.** Every requirement has an ID (`STORE-03`). IDs are never
  reused or renumbered; superseded requirements are marked as such and kept.
- **Priority.** `must` (core product promise), `should` (expected of a mature
  release), `later` (valuable, deliberately deferred).
- **Traceability.** Implementation issues and PRs reference requirement IDs.
  A requirement without a linked issue is unplanned, not rejected.
- **Wording.** "The system" means Strata as a whole; the layer responsible
  (server, workflow engine, frontend) is named where it matters.
