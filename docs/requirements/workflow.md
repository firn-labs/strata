# Workflow

Part of the [requirements catalog](README.md). The workflow system *is*
Strata's logic layer: the founding research concludes that expressing all
business logic as workflows — created by the teams who need them — is what
makes a DMS flexible enough for organization-wide use. This is Strata's core
differentiator.

## WORKFLOW-01 — Team-built flows (low-code) · `must`

Teams create and modify their own workflows in a visual, low-code editor
without central development bottlenecks. Central operators provide base
workflows, governance, and support; selected power users can hold elevated
rights.

## WORKFLOW-02 — Custom interfaces per workflow · `must`

Teams build individual user interfaces for their workflows (forms, views,
dashboards) without coding — with an escape hatch to real code. Specific
needs are rarely implementable centrally; self-service is the point.

## WORKFLOW-03 — Workflow permissions · `must`

Who may create, edit, and execute each workflow is explicitly controlled.

## WORKFLOW-04 — Automated execution with intervention points · `must`

Flows run as automated as possible, without routine human intervention — but
with defined intervention points where automation can err (e.g. recognition
results), including escalation and deputy rules for stalled tasks.

## WORKFLOW-05 — Execution logging · `must`

Every run is logged step by step: trigger, inputs, decisions, outcomes,
timestamps. The log supports later diagnosis and communicating processing
status to third parties.

## WORKFLOW-06 — Three representations, one definition · `must`

A single flow definition (serializable data, JSON graph) renders graphically,
textually, and semantically. The visual editor, the engine, and exports all
consume the same structure.

## WORKFLOW-07 — Export and import of all workflows · `must`

All workflow definitions can be exported and re-imported. Portability is a
first-class feature: systems that accumulate hundreds of flows become
un-replaceable without it.

## WORKFLOW-08 — Tight DMS integration, API-first · `must`

Workflows can invoke every capability of the core server — filing, metadata,
permissions, archiving, sharing. Document status changes (ACCESS-10) act as
triggers. Nothing may be UI-only (see the architecture decisions log).

## WORKFLOW-09 — Third-system steps · `should`

Flow steps can call external systems over HTTP/APIs (e.g. trigger records in
an ERP, call a PDF-processing service) and exchange data with them.

## WORKFLOW-10 — Built-in automations as flows · `should`

Standard automations — automated filing of scanned documents, metadata and
filename assignment, template release, comment/approval threads on documents
(e-signing integrations) — are themselves implemented as workflows, so teams
can adapt them.

## WORKFLOW-11 — Late enablement · `should`

The workflow system can be adopted after an initial document-management-only
rollout; a phased introduction must be a supported path.
