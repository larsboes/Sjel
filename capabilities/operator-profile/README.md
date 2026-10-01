# operator-profile

Sjel's private, versioned source for durable operator preferences, principles, boundaries, and interaction style. It is separate from identity facts (`entities`), project memory, and skills.

The profile store uses the shared SQLite database resolved by `sjel_config::database_path`; profile values stay in the active overlay. The API binds to loopback and refuses foreign browser origins. Writes require the revision previously read, so a stale client cannot silently replace the profile.

## First interface

- `operator-profile show` reads the canonical profile.
- `operator-profile validate [FILE|-]` validates a JSON profile document; omitted input reads stdin.
- `operator-profile put --expected-revision N [--input FILE|-]` replaces the profile only when the stored revision matches (`0` means no profile exists yet).
- `operator-profile export --dry-run --harness claude` previews the managed section for the verified Claude Code target `~/.claude/CLAUDE.md`.

Export is dry-run only. It does not write files. Other harness targets remain unavailable until their user-instruction paths are verified. Fields and statements default to no export; each must name its target allowlist. C2 and C3 entries cannot be exported into assistant instructions.

The managed section is a projection, not a source of truth. Changes made inside it are not imported back into the profile. The generated text states that saved preferences are subordinate to the current request and higher-priority instructions.

No actual profile data or generated instruction text belongs in this repository. Recommendations, automatic learning, dashboard editing, skills, and memory are outside this first slice.
