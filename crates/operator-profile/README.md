# operator-profile

Sjel's private, versioned source for durable operator preferences, principles, boundaries, and interaction style. It is separate from identity facts (`entities`), project memory, and skills.

The profile store uses the shared SQLite database resolved by `sjel_config::database_path`; profile values stay in the active overlay. It opens that file through `sjel-turso` (LibSQL) rather than `sjel-store`: the table is the same one, and the async connection lets a handler await its read instead of handing it to `spawn_blocking` (the first capability cut over, PLAN.md Phase 4). The API binds to loopback and refuses foreign browser origins. Writes require the revision previously read, so a stale client cannot silently replace the profile.

## First interface

- `operator-profile show` reads the canonical profile.
- `operator-profile validate [FILE|-]` validates a JSON profile document; omitted input reads stdin.
- `operator-profile put --expected-revision N [--input FILE|-]` replaces the profile only when the stored revision matches (`0` means no profile exists yet).
- `operator-profile export --dry-run --harness <claude|codex|pi|opencode>` previews the managed section for a verified harness user-instruction target (`~/.claude/CLAUDE.md`, `~/.codex/AGENTS.md`, `~/.pi/agent/AGENTS.md`, `~/.config/opencode/AGENTS.md`).
- `operator-profile export --apply --expected-revision N --harness <claude|codex|pi|opencode>` atomically writes only the managed section (`<!-- sjel:operator-profile:start --> ... <!-- sjel:operator-profile:end -->`) when the stored profile revision matches `N`, preserving all surrounding prose.
- `POST /api/profile/export` previews (`apply: false`) or applies (`apply: true`, requiring `expected_revision`) the managed section over HTTP.

Export defaults to dry-run and writes nothing unless `--apply --expected-revision N` (or `apply: true` with `expected_revision`) is explicitly supplied. Fields and statements default to no export; each must name its target allowlist. C2 and C3 entries cannot be exported into assistant instructions.

The managed section is a projection, not a source of truth. Changes made inside it are not imported back into the profile. The generated text states that saved preferences are subordinate to the current request and higher-priority instructions.

The test that the profile is wired in comes from the PRD's Q14: an answer names a fact about the
operator that the prompt did not contain. Format rules (no preamble, earned length) are a floor,
not this test.

No actual profile data or generated instruction text belongs in this repository. Recommendations, automatic learning, dashboard editing, skills, and memory are outside this first slice.
