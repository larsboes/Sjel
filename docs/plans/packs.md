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
2. **Account copies.** Decided 2026-11-09: remove the six from the claude.ai account and rely on
   Pack deployment alone. Done when `ls ~/.claude/skills/synced/*/` in a new session lists none of
   `council`, `crystallize`, `asd-ste100`, `skill-creator`, `obsidian` or `teach`.
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

Decided 2026-11-09: distinguish Sjel-owned agent behavior from reviewed upstream-managed native
dependencies. Community integrations can qualify with explicit component/host approval. Routine
updates are delegated to upstream within the approved trust boundary; material permission,
execution, egress or licence changes require re-review. Source-porting permission, native-use
approval, activation and running a job are separate. `CONTRIBUTING.md#owned-agent-components`
owns the rule; this replaces the blanket owned-only restriction without approving or restoring
any plugin. Matt and Anatomy store snapshots remain reference-only and held off every harness.
Matt's native Claude/Codex bundles remain disabled; tracker setup and labels remain cancelled.
`web-research` remains Sjel-owned.

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
and diff checks passed. The latest doctor run passes with warnings; the earlier Packs backup
source-root failure is resolved. Full-workspace checks did not run.

Decided 2026-11-09: disable the native Claude `claude-security`, `code-simplifier`,
`frontend-design`, `plugin-dev` and `skill-creator` plugins, plus Matt's separately enabled Codex
bundle. The security scanner is no longer available through that plugin; it was not ported.
Its installed proprietary licence is not eligible for a public Sjel/Pi adaptation. Only the six
enablement values changed; other Claude settings and Codex configuration matched their
pre-change fingerprints. Packages and caches remain installed. Tool connectors, language
servers, Interceptor and account-managed components were not changed. Existing sessions may
retain loaded behavior until restarted.

Decided 2026-11-09: adopt an owned `interactive-figures` workflow in presentations and improve
`tools/skill-eval` under the existing writing owner. The original ideal-lever example uses one
model for forces and geometry; the retained Anatomy source remains reference-only. Evaluation
adds opt-in repeats, per-case descriptive statistics, exact grading coverage and task/grader
cost accounting, not a second framework or upstream runtime plugin. Owning READMEs identify
reviewed sources, licences and rejected material; complete MIT and Apache-2.0 grants are retained.
The synthetic figure baseline passed 9/9 assertions without the new skill, so no reasoning gain
is claimed. Actual Obsidian embedding, native keyboard actions and runtime triggering remain
unverified. Scoped checks passed: 42 offline tests, 33 metadata checks (three deliberate fixtures
skipped), both relevant suite checks, stored pins, self-model freshness, publication hygiene and
diff checks. Five browser states at actual 500/900 px widths, including endpoints and dark mode,
passed and their captures were read. Preserved licence grants match the reviewed sources byte
for byte. Independent review passed after replacing the undeclared Node commands with Bun;
the documented Bun commands and all focused checks passed again. The three presentations
skills are registered for Pi; other harnesses were not redeployed. Full-workspace and paid
native eval runs did not run.

**Matt reference assessment — 2026-11-09.** All 25 retained skills and their technique-bearing
references were compared with authoritative owned guidance, not deployed copies. Source:
[mattpocock/skills](https://github.com/mattpocock/skills), MIT, copyright 2026 Matt Pocock; the
[full grant](https://github.com/mattpocock/skills/blob/4ae2317ddb22b046e26455e4f9c9b3aaaf62212d/LICENSE)
was checked from the primary source. Retained Pack manifest SHA-256:
`e499311775a8d94802c9953478a571db5099d079169035eef42e825f246f7087`;
`tools/skills verify` confirmed its pins. Nineteen retained skill bodies match native 1.3.1
(`49dd158d1076134a641b33efb035946536778336`); six differ, so that commit is not asserted as
the source identity of every retained file. Substantial reuse requires the applicable snapshot
and notices to be resolved before copying.

Disposition: 10 covered, eight candidate rows representing six techniques, five rejected and
two deferred. Coverage is a semantic assessment, not a runtime or performance result. No
instruction-performance deficit was measured. No upstream workflow was executed or activated,
and no source text or code was copied into owned guidance during this assessment.

| Retained skill | Disposition | Existing owner | Reason or smallest conditional technique |
|---|---|---|---|
| ask-matt | Reject | sjel | Competing lifecycle/persona router; retain the owned workflow. |
| code-review | Covered | unslop, prior-art, effective-rust | Requirements, conventions and scoped review already have owners. |
| codebase-design | Candidate M3 | prior-art | Caller obligations and the module-deletion thought experiment. |
| diagnosing-bugs | Candidate M4 | technical-precision, root-cause | Improve symptom sensitivity, turnaround and intermittent reproduction rate. |
| domain-modeling | Covered | crystallize, technical-precision, effective-rust | Grounded terminology, explicit decisions and domain types. |
| grill-me | Covered | crystallize, ask | Alias for the interview technique, not a separate capability. |
| grill-with-docs | Covered | crystallize, technical-precision | Interview plus contemporaneous terminology/decision capture. |
| grilling | Candidate M1 | crystallize | Only ask decisions whose prerequisites are settled. |
| handoff | Defer | sjel, crystallize, runtime compaction | Cross-harness redacted continuation needs a concrete losing-context case. |
| implement | Reject | prior-art, sjel | Thin orchestration macro; automatic full-suite runs and commits are not defaults. |
| improve-codebase-architecture | Candidate M3 | prior-art, sjel | Same interface-depth diagnostic, not another adaptation. |
| pr | Covered | sjel | Existing change descriptions; work-package PRs remain disabled while PLAN is active. |
| prototype | Covered | interactive-figures, ui-craftsmanship, unslop | Explicit model, meaningful interaction and host-appropriate variants. |
| research | Covered | technical-precision, prior-art, web-research | Primary sources, bounded investigation and owned output placement. |
| retro | Candidate M6 | root-cause, suggest-skills, trim | Session-environment review; distinguish absent checks from unwired checks. |
| setup-matt-pocock-skills | Reject | sjel | Tracker, labels and bootstrap scaffolding remain cancelled. |
| tdd | Candidate M2 | sjel verification, effective-rust | Independent behavioral expectations and a relevant observed failing regression. |
| to-questionnaire | Candidate M5 | crystallize | Self-contained questions for an absent knowledge holder, not the present user. |
| to-spec | Covered | crystallize, sjel | Grounded synthesis, acceptance behavior and canonical specification placement. |
| to-tickets | Defer | sjel, crystallize | Test vertical slices/expand-contract on a real feature before adding process. |
| triage | Reject | root-cause, prior-art, crystallize | No demonstrated need for a label/state-machine queue or duplicate rejection store. |
| wait-what | Covered | asd-ste100, technical-precision | Clear re-explanation with necessary context. |
| wayfinder | Candidate M1 | crystallize | Separate blocked decisions, unformed intent and already-decided work. |
| wizard | Reject | existing manual setup owner | No generic plaintext-secret persistence or CI-secret workflow. |
| writing-for-agents | Covered | skill-creator, asd-ste100, sjel | Progressive disclosure, canonical owners, completion criteria and lean context. |

Conditional techniques: M1 prerequisite-aware questions and incremental intent; M2 independent
behavioral test oracles and relevant red-before-repair evidence; M3 caller-obligation/deletion
diagnostic; M4 reproduction-signal quality; M5 absent-stakeholder questionnaires; M6 session
environment retrospectives. Approved prerequisite-order and test-expectation probes (M1/M2)
passed four cases and all 12 manual assertions under the existing guidance. No new guard block is added: crystallize only loses its contradictory
fixed question-count minimum, and verification guidance remains unchanged. The receipt is
`Packs/deliberation/skills/crystallize/evals/guard-probe.json`; owning attribution and the complete
MIT grant are preserved. One synthetic model call is not an isolated comparison or proof of a
reasoning gain. M3-M6 require a concrete task showing why existing guidance is insufficient.
No new standalone skill, tracker, vocabulary system or orchestration framework is proposed.

Two additional native-bundle entries were already excluded: `teach` remains with the owned
vault-based teaching/practice system; `implement-spec` requires integration branches, worktrees
and tracker orchestration incompatible with the current workflow. Their main instructions were
checked to confirm those exclusions; they are not counted as two further full-reference audits.
The next approved investigation is Claude Security's primary source and applicable licence;
source availability does not authorize porting the restricted installed plugin.

**Security source assessment — 2026-11-09.** The official
[claude-security LICENSE](https://github.com/anthropics/claude-plugins-official/blob/0412be76f50ab4f4f8abff3753efa9269fbbca5b/plugins/claude-security/LICENSE)
is proprietary and matches the installed 0.12.0 restrictions: Anthropic-product use only,
no redistribution and no use to develop non-Anthropic products. Source availability is not
permission to port it; no restricted prompts, agents or workflows are adopted.

A different project,
[claude-code-security-review](https://github.com/anthropics/claude-code-security-review/tree/0c6a49f1fa56a1d472575da86a94dbc1edb78eda),
has an MIT grant (copyright 2025 Anthropic). Its command, filter, API client, action and
dependency manifest were read at that revision; it is a PR/diff reviewer, not the newer plugin.
Whole-workflow adoption is declined. The command excludes Rust memory-safety and AI-prompt
findings (`.claude/commands/security-review.md:148,152`); the hard filter also removes matching
non-C/C++ reports (`claudecode/findings_filter.py:140-143`). Failed or disabled model filtering
assigns confidence 10.0 (`claudecode/findings_filter.py:299-314`), not an unknown/unverified state.
The action installs the native CLI and publishes comments/artifacts by default; none of it was
installed, executed or activated. This is a bounded source/permission assessment, not a complete
dependency, history or runtime audit.

Existing coverage is distinct: `crates/sjel-cli/src/audit.rs:155-157` runs gitleaks and OSV;
`crates/sjel-cli/src/intake/review.rs:24-33` reviews third-party agent material for manipulation;
`Packs/harness/agents/review.md` verifies own changes without an explicit vulnerability-review
contract. No dedicated own-code vulnerability route was located in those searched surfaces;
this does not establish that the general reviewer lacks the reasoning ability. Decided: reuse
that reviewer for checkout-based security review with an explicit target and security brief,
not a new skill or scanner. Four public synthetic diffs (three defects, one benign control)
satisfied 12 manual semantic assertions. The fixture-only no-tools assertion failed: the actor
ran its mandatory Git inspection, so the result is not isolated. The receipt is
`Packs/harness/evals/review-security-baseline.json`; the review definition is unchanged. No
fixture code, exploit, application test, native scan or paid native-Claude evaluation ran.
Both upstream workflow/source adoptions are declined in `upstreams.toml`; native Claude Security
use is a separate, not-yet-approved dependency decision under the revised boundary. No proprietary
or MIT upstream instructions/code are adapted into the security workflow.

Open: review native and account-synced plugins and application-bundled skills; classify each as
owned adaptation, explicitly approved upstream-managed native dependency, or unapproved behavior
requiring a disposition. A native dependency does not require an owned wrapper merely to qualify.
Existing installations are not grandfathered into approval. Native approval/enforcement and
activation inventory remain incomplete; this policy-only change modifies no plugin settings or
runtime holds. Full-workspace checks did not run.

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
