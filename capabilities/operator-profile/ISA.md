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

## Claims

Numbered 2026-10-08, from the contract above and the tests that already hold it.

- [x] OPR-1 — a write that names a stale revision does not replace the profile. Falsifier: a `put`
  with an old `--expected-revision` changes the stored profile. Probe:
  `stale_write_does_not_replace_the_profile` (`src/store.rs`) and
  `profile_store_refuses_unconditional_stale_write` (`tests/profile_api.rs`).
- [x] OPR-2 — no C2 or C3 value reaches an export. Falsifier: a rendered section holds a C2 or C3
  value. Probe: `refuses_export_of_c2_or_c3_values` (`src/model.rs`) and
  `does_not_render_c2_or_c3_even_if_an_invalid_document_is_loaded` (`src/render.rs`).
- [x] OPR-3 — profile values do not appear in logs or debug output. Falsifier: a value in `Debug`
  output or in a log line. Probe: `debug_output_does_not_reveal_values` (`src/model.rs`) and
  `profile_input_round_trips_as_json_without_logging_values` (`tests/profile_api.rs`).
- [x] OPR-4 — an export changes only its managed region, and refuses a target whose markers are
  broken or duplicated. Falsifier: text outside the region changes. Probe:
  `inserts_and_replaces_only_the_managed_region` and `refuses_broken_or_duplicated_markers`
  (`src/render.rs`).

## Not yet specified

- **An export that writes.** Export is dry-run only. A write needs its own reviewed contract (see
  Export contract).
- **Harness targets other than Claude Code.** Each needs its user-instruction path verified first.
