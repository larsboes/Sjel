# Packs: one source, every harness, user and project level

Sjel's skills and agents live in `Packs/` and are deployed into each agent harness. Today every
deployment goes to a harness's user-level root on one machine. The goal is one deployment model
that also covers the project level (a checkout, including a cloud container that clones it) and the
account level (skills a hosted harness syncs from the user's account), with one source and no
drifting copies.

## 1. Facts (checked 2026-10-28)

- **Sources.** 32 skill directories across 13 Packs (`ls Packs/*/skills`), measured 2026-11-07;
  27 across 9 when this section was written. Each Pack has a `pack.toml`
  (`schemas/pack.toml.example`).
- **Harnesses.** `crates/sjel-cli/src/harnesses/registry.rs` enumerates claude, codex, opencode,
  pi, copilot, sjel-agent and antigravity. Claude Code, Codex, opencode and copilot get
  materialized copies. pi gets registry entries (`pi_settings.rs`).
- **Ledgers.** One per harness under `~/.local/state/sjel/pack-deployments/`, owned and backed up by
  `services/packs`. A ledger records each unit, its Pack and the installed digest.
- **Profiles.** `profiles.toml` defines the available profiles and `[harness_profile]` bindings.
  `tools/harnesses apply` reconciles each installed harness to its binding; `use <profile>` is an
  explicit override. `[unit_hold]` keeps named units off named harnesses. Claude Code, Codex,
  Antigravity, opencode and Copilot bind to `coding-harness`; pi binds to `full`. sjel-agent is
  unbound and retains its overlay-owned selection.
- **Agent model tiers on pi.** A Pack agent's Claude `model:` tier maps to a pi model through
  `[tiers]` in the overlay's `config/pi-models.toml` (`harnesses/agentfile.rs`). An unmapped tier
  is dropped and the agent inherits the session model. The `review` agent (`opus`) is how a small
  session model gets its work checked by a stronger one.
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

1. **Account copies drift silently.** Nothing in Sjel knows the account-synced copies exist, so
   the six above drifted with no report. A session cannot tell which `council` is current.

2. **Third-party screening now binds the installed bytes (P6).** `tools/skills add` and `update`
   stage upstream's output once under `~/.local/state/sjel/intake/staged/`, read every staged
   skill into memory, gate that same snapshot, and publish those bytes together with the staged
   lock entry, so there is no second fetch after screening
   (`crates/sjel-cli/src/intake/skills.rs`).
   The gate is fail-closed: a missing or failed model review holds the content unless that exact
   digest was approved, `--no-review` is gone, and an unapproved `--against` baseline is
   ignored. Without the `intake_review` role, `tools/intake review --reviewer claude` records a
   harness review receipt for one digest, and the gate reads it in place of the model review.
   Completed inference-role reviews also record reusable evidence. Reuse is scoped to source,
   content, approved baseline, model/endpoint settings and the intake policy fingerprint;
   incomplete file reads and failed persistence hold the candidate. Findings retain their
   normal severity. Legacy receipts without a policy fingerprint require a new review.
   A real Claude harness review of Anatomy completed on 2026-11-08 and its receipt was reused
   by the store gate. It reported no holding finding; binary assets were explicitly unread.
   Snapshots refuse symlinks, special files and unreadable directories. Text is limited to
   2 MiB, binary assets to 32 MiB per file, and complete packages to 128 MiB. Binary bytes
   remain in the installed snapshot and digest; the text review reports them as unread.
   Limits: publication is not
   crash-atomic across layouts (backups are restored on ordinary failure, the lock is written
   last), and approval records still do not prove who read the findings.
3. **Reference storage is not owned adoption.** The store and profile infrastructure was built
   in P7. Unchanged Anatomy and Matt snapshots remain verified references, excluded from runtime
   deployment under section 5. The old global Anatomy channel was removed; helper cleanup and
   independent application-supplied discovery remain part of the ownership audit.

Fixed in `383cb67`: `sync` ignored the profile, the doctor warned for units the profile leaves
out, and its Claude Code fix hint named the `tools/packs.sh` shim. A ledger now records the
profile `use` activated, and both read its selection. A ledger written before `383cb67` carries
no profile until the next `use <profile>` on that machine.

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
4. **Names that cannot collide.** When the same skill reaches a session through two channels,
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
4. **Approved external skills.** Whether an approved upstream skill is installed project-locally
   or registered from an isolated, content-pinned store per harness. The route must preserve
   upstream ownership, avoid global name collisions and support pi sessions started in the vault.
   Decided 2026-11-03: the content-pinned store, as recommended below.
   - **Project-local install.** This is what `tools/skills add` does today. It needs one copy per
     project, plus one in the vault for pi. The profile never sees these copies, so defect 3
     stays open, and Anatomy still needs a second channel to reach the vault.
   - **Content-pinned store.** Approved bytes go into a store keyed by the P6 digest. The deploy
     engine reads the store as one more Pack root. It copies into the materialized harnesses
     (Claude, Codex, OpenCode, Copilot) and registers the store in the registry harnesses (pi,
     sjel-agent, Antigravity) (`crates/sjel-cli/src/harnesses/registry.rs`, `Model`). Profiles then
     select external skills like Pack skills. Cost: a store layout and a name-collision rule.
   - Recommendation (an estimate, not measured): the store. It closes defect 3 and the vault
     case with the engine that exists, and the P6 digest binds each copy to its approval.

Coding harnesses use `coding-harness`; pi keeps `full`. Content-pinned upstream snapshots are
reference material; runtime selection follows the owned-component decision in section 5.

## 5. Work packages

Decided 2026-11-08: skills and harness plugins must be owned, maintained adaptations, not wholesale
upstream installations. This supersedes the store-deployment choice for unchanged skills.
`CONTRIBUTING.md#owned-agent-components` owns the rule. Matt and Anatomy store snapshots remain
reference material and are held off every harness. Matt's native Claude and Codex plugins are
disabled; tracker setup and label provisioning are cancelled. `web-research` remains Sjel-owned.

Decided 2026-11-09: retire Ponytail from runtime and close its temporary migration exception.
The native Claude plugin is disabled at user scope. Automatic instruction repetition and the
remaining command suite are explicitly not adopted; no replacement hook or plugin is planned.
Retained source and its MIT notice are reference material only. No Ponytail code or text was copied.

Selected behavior remains in existing owners: `prior-art` covers minimum-sufficient coding
through planning, implementation and review; `unslop` needs no additional complexity block.
Decided 2026-11-09: broaden `prior-art` triggers and add only two sentences for meaningful
checkpoints, not a new checklist or per-edit loop. The lifecycle probe passed all four assertions
with both previous and expanded guidance; the baseline already made the desired decisions, so
no reasoning-quality gain is claimed. Its ladder and safety body is unchanged. The earlier
planning/review comparison passed 18 assertions for each recorded version. Evidence and
limitations live in each skill's `evals/`.
Common inherited context remains a confound, and runtime triggering was not tested. Pi resolves
both owned sources without diagnostics. Materialized copies remain unchanged to avoid deploying
unrelated concurrent edits. Metadata and suite checks, self-model freshness, publication hygiene
and diff checks passed. Doctor still reports an invalid Packs backup source root. Full-workspace
checks did not run.

Decided 2026-11-09: disable the native Claude `claude-security`, `code-simplifier`,
`frontend-design`, `plugin-dev` and `skill-creator` plugins, plus Matt's separately enabled Codex
bundle. The security scanner is no longer available through that plugin; it was not ported.
Its installed proprietary licence is not eligible for a public Sjel/Pi adaptation. Only the six
enablement values changed; other Claude settings and Codex configuration matched their
pre-change fingerprints. Packages and caches remain installed. Tool connectors, language
servers, Interceptor and account-managed components were not changed. Existing sessions may
retain loaded behavior until restarted.

Open: audit other native and account-synced plugins and application-bundled skills; classify each
as owned adaptation, a dependency behind an owned adapter, or unadopted agent behavior requiring
migration or disabling. Existing unadopted components are unresolved violations, not implicit
exceptions. The wider migration is not complete. Full-workspace checks did not run.

| # | Work | Done when |
|---|---|---|
| P1 | Done in `383cb67`: `sync` keeps a Pack to the recorded profile. | Checked in a cloud container. |
| P2 | Done in `383cb67`: the doctor reports units outside the recorded profile as not selected, and names `tools/packs-claude deploy`. | Checked in a cloud container. Not checked on the Mac: run `tools/packs-claude use full` once there first. |
| P3 | Report account-synced skills that share a name with a Pack skill, with their drift (defect 1). | In a cloud session, `tools/harnesses status` lists the six names above with "differs". |
| P4 | Deployment levels in the ledger and registry (idea 1), after decision 4.1. | A ledger entry carries its level, and `harnesses status` groups by it. |
| P5 | Done in `97c51d07`: harness-bound coding/full profiles, deployed and checked on the Mac. | 401 CLI tests pass serially; five coding harnesses omit knowledge and learning; pi retains both. |
| P6 | Done in `cf6563d9`, with failure rollback in `f0538875`: bind third-party installation to screened content; review failure holds. | 408 CLI tests and 14 `tools/skills` cases pass; model-review failure holds; an unapproved baseline cannot suppress findings; a failed layout or lock write rolls earlier layouts back. |
| P7 | Store and verification built in `095960fc`; bounded assets, optional profile selection and verified replacement in `35d866ed`. Anatomy store, four materialized destinations and two registry deployments verified. Open: old helper-file cleanup, source revision metadata when the upstream CLI omits it, and application-owned links. | Stored bytes match their digest; each destination loads the reviewed source without a second global channel, including a vault-started Pi session. |
| P8 | Done in `5ddc2049`: `vendor-audit`, the four-lens method for vetting third-party software before it runs, plus the octet v0.10.0 pass as its worked example. Method only, deliberately: `tools/audit` keeps its inward scope and the verdict lands in `upstreams.toml`. | `tools/check-skill-metadata.sh` passes with it included; a later audit of a differently-shaped program (a Node CLI, a Python daemon) reaches a verdict without leaving a lens unrun. |

Verification: all 408 `sjel-cli` tests pass with `--test-threads=1`; strict Clippy and `rustfmt
--check` pass. A parallel rerun hit a managed-settings lock failure in
`pi_config::tests::repeated_apply_retains_first_original_and_is_byte_idempotent`; that test passed
in isolation. Live `harnesses status` reports no active-unit defects, no knowledge or learning
units on the five coding harnesses, and both Packs retained by pi. Doctor has seven unrelated
failures: four stale backups, two failed scheduled runs and one host-patch audit finding.

Profile validation does not exercise Anatomy rendering or Obsidian embedding. Those live checks
remain in [`learning.md`](learning.md). Full-workspace checks did not run.
