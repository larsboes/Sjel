# tools/claude-code-config

Applies and checks the user-level Claude Code settings file — `~/.claude/settings.json`, or
`$CLAUDE_CONFIG_DIR/settings.json` when that is set.

```bash
tools/claude-code-config/claude-code-config            # merge the baseline, existing-wins
tools/claude-code-config/claude-code-config check      # report drift from the baseline
tools/claude-code-config/claude-code-config --force    # restore values the baseline declares
tools/claude-code-config/claude-code-config --dry-run  # say what would change, write nothing
```

| Exit | Meaning |
| --- | --- |
| 0 | done, or nothing to change |
| 1 | usage, or the baseline is unreadable |
| 2 | the target exists but is not valid JSON — **left alone**, never overwritten |
| 3 | `check` found drift |

## Rust, not the TypeScript this replaced

This tool was `tools/claude-code-config.ts` until 2026-10-02. It is Rust now for the reason
`on-dependencies-and-build.md` gives — new backend logic is Rust — and because most of the
TypeScript had become dead weight rather than logic worth porting: `mergeFragment`,
`stageManagedPolicy`, `managedHandoffInstructions`, the sudo handoff and the atomic policy
writer all existed to serve the managed layer, and the managed layer was retired. What
survived is a merge and a comparison, which is a fresh crate rather than a translation.
`tools/storage/storage` made the same move on 2026-09-03.

## Why there is no managed layer any more

The floor used to be deployed as a root-owned policy at
`/Library/Application Support/ClaudeCode/managed-settings.json` (`/etc/claude-code/` on Linux).
The principal ruled on 2026-10-02 that it should not be: it needed sudo for every change, it
had to be deployed per device, it carried an MCP allowlist that silently blocked this
machine's own `graphify` server from 2026-08-02 until it was measured, and `tools/`,
`Packs/` and ISA all had to describe it.

The floor now lives in `tools/templates/claude-code/settings.base.json` and lands in the same
user-level file as everything else. **What that gives up is stated rather than hidden:** a
user-level file is writable by the agent sessions that run as that user, so the floor is no
longer something a session *cannot* lift, only something it *should not*. `check` is the
replacement for that guarantee — it reports drift, and this tool is the only thing in the tree
that would notice it.

Two keys in the retired policy were managed-only and could not come along:

- `disableSideloadFlags` — `--plugin-dir`, `--plugin-url`, `--agents` and `--mcp-config` are
  accepted again. Setting it in a user-level file does nothing; Claude Code ignores it there.
- `sandbox.enabledPlatforms` — inert at user level. It only ever mattered to keep the sandbox
  configuration inactive on Windows, which this deployment does not run.

`allowManagedPermissionRulesOnly`, `allowManagedMcpServersOnly`,
`sandbox.filesystem.allowManagedReadPathsOnly` and `allowedMcpServers` were managed-only by
definition, and removing them is the point of the move: you can define your own permission
rules again, and MCP servers load without an allowlist.

## Two behaviours worth knowing

**A write re-serializes with sorted keys.** `serde_json`'s `Map` is a `BTreeMap` unless its
`preserve_order` feature is on, and that feature unifies across a whole build graph — enabling
it here would change ordering for all thirty-odd members to gain a cosmetic one here. So the
first write sorts the keys of a file you may have ordered by hand. Values are untouched.

Measured 2026-10-02, because the second half of that reasoning turned out not to hold. Something
else already enables the feature: `libs/extraction` reaches `serde_json/preserve_order` through
`xberg`, so under `cargo test --workspace` the map is an `IndexMap` and two literals with the same
keys in a different order hash differently — `the_digest_is_key_order_independent` failed there
while passing under `-p sjel-claude-config`. `digest` now sorts explicitly rather than trusting the
map, so it answers the same under either build.

The write path is unaffected in practice, and by accident rather than by design: the launcher
builds `-p sjel-claude-config`, which resolves the feature off. Verified end to end — a file
written `{"zebra": …, "alpha": …}` comes back with `alpha` first. Under a workspace build it would
keep the caller's order instead, which is a latent difference rather than a live one.

**`apply` never overwrites.** An edited value survives it by design, which is what makes
re-running safe and also means an emptied `permissions.deny` is not restored by it. That is
what `--force` is for, and `check` is how you find out you need it.

## Placement

`tools/<name>/` holding a Cargo member plus a thin launcher is the established shape
(`tools/storage`, `tools/capability-auth`, `tools/fda-launcher`). Operator machinery lives in
`tools/`; where code belongs is `on-placement.md` in the `sjel` skill.
