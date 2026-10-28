# Packs: one source, every harness, user and project level

Sjel's skills and agents live in `Packs/` and are deployed into each agent harness. Today every
deployment goes to a harness's user-level root on one machine. The goal is one deployment model
that also covers the project level (a checkout, including a cloud container that clones it) and the
account level (skills a hosted harness syncs from the user's account), with one source and no
drifting copies.

## 1. Facts (checked 2026-10-28)

- **Sources.** 27 skill directories across 9 Packs (`ls Packs/*/skills`). Each Pack has a
  `pack.toml` (`schemas/pack.toml.example`).
- **Harnesses.** `tools/sjel-cli/src/harnesses/registry.rs` enumerates claude, codex, opencode,
  pi, copilot, sjel-agent and antigravity. Claude Code, Codex, opencode and copilot get
  materialized copies. pi gets registry entries (`pi_settings.rs`).
- **Ledgers.** One per harness under `~/.local/state/axon/pack-deployments/`, owned and backed up by
  `capabilities/packs`. A ledger records each unit, its Pack and the installed digest.
- **Profiles.** 7 in `profiles.toml`. `use <profile>` replaces a harness's set with the profile's
  units. `[unit_hold]` keeps named units off named harnesses.
- **Project level.** Nothing is deployed per project. `.claude/skills/` is gitignored for
  third-party installs (`.gitignore`). Cloud sessions get Packs through `tools/cloud-session-start`,
  which runs `tools/packs-claude use repository` into the container's user-level root
  (`3b838f2`). Skills deployed this way load into a running session.
- **Account level.** A Claude Code cloud session also loads the skills synced from the user's
  claude.ai account (`~/.claude/skills/synced/<org>_<account>/`, exposed as `anthropic-skills:*`).
  Six of them share a name with a Pack skill: `council`, `crystallize`, `asd-ste100`,
  `skill-creator`, `obsidian`, `teach`. All six differ from their Pack source, by 19 to 644 lines
  (`diff -r Packs/*/skills/<name> ~/.claude/skills/synced/*/<name>`). Both copies load in one
  session.

## 2. Defects

1. **Sync ignores the profile.** `tools/packs-claude sync harness` deployed `trim` and
   `harness-sync` while the active profile (`repository`) selects only `sjel` from that Pack.
   Re-running `use <profile>` restores the set.
2. **Account copies drift silently.** Nothing in Sjel knows the account-synced copies exist, so
   the six above drifted with no report. A session cannot tell which `council` is current.
3. **The doctor judges against every Pack, not the active profile.** In a session on the
   `repository` profile, the Packs section warns 9 times for units the profile deliberately
   leaves out. A warning that is expected on every run teaches readers to skip the section.
4. **The doctor's fix hint names a shim.** `tools/sjel-cli/src/doctor/checks.rs`
   `pack_commands` prints `tools/packs.sh link <pack>`. `tools/packs.sh` is a compatibility shim
   over `tools/packs-claude`.

## 3. Ideas

1. **Three deployment levels, one model.** A destination has a level: `user` (a harness's home
   root, today's only level), `project` (inside a checkout), `account` (a hosted harness's synced
   set). The ledger records the level, so drift and ownership checks work the same at each.
2. **Project level from the repository itself.** A checkout declares the profile its sessions
   want (as `repository` is declared today in `tools/cloud-session-start`). Any harness opening
   the checkout gets that set, on a workstation as well as in a cloud container.
3. **The account as a harness.** Treat the claude.ai account's skill set as a destination with
   its own ledger, so `harnesses status` reports its drift. Upload stays a manual step if no API
   exists for it (unverified).
4. **Doctor reads the active profile.** A unit outside the active profile is "not selected",
   not "not deployed", and is not a warning.
5. **Names that cannot collide.** When the same skill reaches a session through two channels,
   one wins by rule, and the doctor reports the loser.

## 4. Open decisions

1. **Project-level channel.** Either track `.claude/skills/<name>` in the repository (copies or
   links into `Packs/`), or keep project level as a hook that deploys to the user-level root.
   Tracked copies duplicate the source in git. Links contradict the "copies, never symlinks"
   rule for deployment (`tools/packs.sh` header). The hook works only where hooks run.
2. **Account copies.** Keep the six account-synced skills and manage them as a destination, or
   remove them from the account and rely on Pack deployment alone.
3. **Profile per level.** Whether the user level and the project level may carry different
   profiles on one machine, and which one wins for a skill both select.

## 5. Work packages

| # | Work | Done when |
|---|---|---|
| P1 | `sync` refreshes only the units the active profile selects (defect 1). | `use repository`, then `sync harness`, leaves `trim` and `harness-sync` undeployed. Unit test in `tools/sjel-cli/src/harnesses/`. |
| P2 | The doctor judges Pack state against the active profile and prints the `packs-claude` command (defects 3, 4). | A repository-profile session shows no Pack warnings. A unit removed by hand still warns. |
| P3 | Report account-synced skills that share a name with a Pack skill, with their drift (defect 2). | In a cloud session, `tools/harnesses status` lists the six names above with "differs". |
| P4 | Deployment levels in the ledger and registry (idea 1), after decision 4.1. | A ledger entry carries its level, and `harnesses status` groups by it. |
