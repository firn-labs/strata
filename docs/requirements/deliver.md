# Deliver

Part of the [requirements catalog](README.md). Deliver is sharing and output.
Practitioner insight: users prioritize this stage *lower* than the literature
does — build the essentials well, don't over-invest early.

## DELIVER-01 — Internal sharing · `must`

Sharing with colleagues grants rights directly in the system; the document
never leaves it. Edit protocol and versioning stay intact. Internal share
links allow granting several people access at once via chat or e-mail.

## DELIVER-02 — Folder sharing · `must`

Whole folders/dossiers can be shared with individuals or teams, including
long-lived shared folders for cross-team work.

## DELIVER-03 — External share links with guardrails · `must`

External share links exist, but the system warns before an external release
and offers sending the document as an e-mail attachment instead — directly
from the system, with replies re-ingested (CAPTURE-02).

## DELIVER-04 — Login-protected external access · `must`

Protection-worthy documents shared externally are only accessible after a
login.

## DELIVER-05 — Cleansing before release · `should`

Before external release the document is automatically cleansed of internal
remarks, comments, and internal personal data; the recipient gets the final,
cleaned version (or a cleaned editable original when they must edit).

## DELIVER-06 — Re-import indicator · `should`

Released documents carry a technical indicator so a returned document is
matched to the original and filed as its new version (STORE-11).

## DELIVER-07 — Industry-standard exchange · `later`

Document exchange with other organizations via established industry standards
(EDI-class) instead of proprietary channels. Distributed-ledger sharing is a
research topic only.

## DELIVER-08 — Printing · `should`

Documents print on numbered pages, optionally with their attributes and
signatures, plus a barcode/QR code so a scanned return maps back to the
original (CAPTURE-06).
