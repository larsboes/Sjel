# ISA — operator-profile

## Purpose
Own the canonical operator preference profile used to shape local assistant context.

## Data boundary
Profile values are private operator state. Store them only in the overlay-backed shared database. Never commit profile values, exports, or fixtures containing real personal data. C2/C3 fields are refused from assistant-file exports; export is opt-in per field and per harness. Do not log profile values.

## Ownership
This capability owns profile validation, revision-checked writes, local API, CLI, and rendered profile section semantics. `entities` continues to own descriptive identity facts. `tools/harnesses` continues to own Pack deployment; profile text is not a Pack.

## Export contract
Exports are one-way projections from the canonical profile. The first target is Claude Code's user-level `~/.claude/CLAUDE.md`, as identified by the repository's trim-context tooling. The first implementation is dry-run only. Writes require a later explicit implementation and reviewed contract. Unknown harness paths are refused, never guessed.

## Out of scope
Project memory, skill management, automatic preference learning, two-way sync from generated files, and dashboard editing.
