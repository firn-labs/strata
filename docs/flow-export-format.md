# Flow export format

Implements **WORKFLOW-07** (portability guarantee), building on
**WORKFLOW-06** (one serializable definition behind every representation).
All workflow definitions can leave a Strata installation and come back —
or move to another one — without loss. A DMS that accumulates hundreds of
flows must never become un-replaceable because its logic is trapped inside.

## Envelope

Every export — whether of one flow or all of them — is a single JSON
document with the same shape:

```json
{
  "format": "strata-flows",
  "version": 1,
  "flows": [
    {
      "id": "0b8f8f2e-6a3f-4d9d-9c1e-2f4a9d6b7c8d",
      "name": "Incoming invoices",
      "owner": "accounting",
      "nodes": [
        { "id": "upload", "kind": "trigger", "config": { "source": "capture" } },
        { "id": "big?", "kind": "condition",
          "config": { "input": "amount", "operator": "greater_than", "value": 1000 } },
        { "id": "approve", "kind": "step", "config": { "action": "request_approval" } },
        { "id": "file", "kind": "step", "config": { "action": "move", "target": "invoices/" } }
      ],
      "edges": [
        { "from": "upload", "to": "big?" },
        { "from": "big?", "to": "approve", "branch": "true" },
        { "from": "big?", "to": "file", "branch": "false" }
      ]
    }
  ]
}
```

- `format` — always `"strata-flows"`. Importers reject anything else.
- `version` — format version, currently `1`. Bumped only on breaking
  changes; an importer accepts exactly the versions it knows how to read
  and rejects the rest with a clear error instead of guessing.
- `flows` — the flow definitions themselves, **verbatim**. This is the same
  JSON graph structure the visual editor saves and the engine executes
  (WORKFLOW-06); the export adds nothing and strips nothing.

Exports are deterministic: flows are ordered by name, then id, so exporting
the same engine twice yields the same document (diffable, versionable).

## API

| Endpoint | Meaning |
| --- | --- |
| `GET /flows/export` | All registered flows in one envelope |
| `GET /flows/{id}/export` | One flow, same envelope shape |
| `POST /flows/import?on_conflict=fail\|replace\|skip` | Import an envelope |

Because single-flow exports use the same envelope, any export is a valid
import payload.

## Import semantics

- **Ids are preserved.** A re-imported flow keeps its id, so references to
  it (run history, links, other systems) stay valid across the round trip.
- **All-or-nothing.** The entire envelope is validated first — format
  marker, version, structural validity of every flow, duplicate ids within
  the payload. A rejected import leaves the engine untouched.
- **Conflicts are explicit.** If an imported id already exists,
  `on_conflict` decides: `fail` (default) rejects the whole import with
  `409 Conflict`, `replace` overwrites the existing definitions, `skip`
  keeps them and imports only the new flows. The response reports every
  flow as `imported`, `replaced`, or `skipped`.

## Round-trip guarantee

Export → import → identical behavior. This is contractual and covered by
tests (`crates/strata-workflow/tests/export_import_api.rs`): a flow
exported from one engine and imported into another produces an identical
export there, and executing the same trigger with the same input yields the
same step trace — same nodes visited, same decisions, same outcomes.
