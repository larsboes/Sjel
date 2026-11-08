# Maintenance priorities and reusable audit tooling

Reviewed 2026-11-08. The decision is to finish shared contracts, remove stale planning and
automate routine maintenance before expanding agent features. Priority lives in
[`PLAN.md`](../PLAN.md); implementation work stays in the existing area plans.

## What the repository contradicts

The initial census found 14 area plans and 4,003 lines including the priority index. Age and
length alone do not establish obsolescence. The review sampled source-backed
claims across all areas; it did not validate every acceptance check or GitHub issue.

| Claim or backlog | Evidence in the checkout | Consequence |
|---|---|---|
| Agent containment must precede all other work | The old priority index placed containment first and phone-agent tiers next | Superseded by the maintenance-first order; current protections remain |
| API extraction is future restructuring | 26 `crates/*-api/Cargo.toml` manifests already exist; GPUI core depends on shared contracts | Audit remaining copies and transformations before extracting more crates |
| One workspace and directory/package equality describe the current layout | Root has 85 crate manifests; GPUI declares another workspace; 40 root package names differ from their directory names | These remain unfinished targets; rank renaming below removing duplicate behavior |
| GPUI escapes formatting | `tools/ci-run.ts` checks the GPUI manifest; `tools/ci-local.test.ts` covers selection | Retire the old tooling item; retain any separate failure of revision isolation |
| Repository Bun updates need initial automation | `.github/dependabot.yml` now covers all five Bun trees | Keep the read-only freshness gap, but remove claims that Bun coverage is missing |
| Markdown readers need replacing | `ui/src/research/articles.rs` delegates to `doc_view::render_markdown`; the GPUI plan records TextView adoption | Verify remaining consumers instead of repeating that migration |
| Finance ideas need an app | `docs/plans/finance.md` first calls the app open, then describes its built Ideas view | Keep manual acceptance open; remove the obsolete implementation task |
| Learning needs further UI implementation | `docs/plans/learning.md` requires five real teach/practice pairs before new controls, while later sections propose more surfaces | Keep the pilot's evidence gate; defer additional UI until observed friction justifies it |
| The research index describes the product | Its introduction names the deleted dashboard and an idea links to `apps/gpui-prototype` | Repair the index; Git retains the retired rationale |

The plans contain 66 occurrences of the legacy path prefixes `tools/sjel-cli`, `libs/` and
`capabilities/`. Some are historical references, so a global replacement would be wrong.
The current CLI source is `crates/sjel-cli`; each operational reference needs resolution.

The restructure plan carries extensive shipped API inventories, old measurements and execution
history. Reduce each completed package to its verified commit, as the documentation rule
already requires. The website plan similarly keeps completed implementation details under
“Open”. Storage, data and tooling repeat vault migration and backup work. Preserve one owner
and link to it rather than merging unrelated domains into one larger plan.

There is also a policy contradiction: the Sjel skill's `shared-data-boundaries.md` says `c2`
stays local, while the priority index admits `trusted_view` to trusted providers. Resolve this
against the enforced admission implementation before changing either rule. Text cleanup must
not silently decide a security policy.

## Code reduction with the existing architecture

The root workspace already centralizes dependencies, but many manifests repeat `schemars`
requirements. Converge compatible direct requirements through `workspace.dependencies`, while
keeping each consumer's features explicit. This reduces version maintenance; it does not prove
that a transitive duplicate can be removed. Cargo supports inherited workspace dependencies
([Cargo workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html)).

`crates/route-manifest` already derives request schemas through Schemars. `sjel-server` owns
startup and authentication; `sjel-http` owns outbound client defaults. Extend these owners only
where repeated behavior has the same contract. A new OpenAPI framework, generic CRUD engine
or central service superclass has no demonstrated need in this review.

The UI already has `ui/src/primitives`, Kit TextView/date/time controls, NSH base components and
native PDF/map integration. Preserve that boundary. Full UI replacement would add migration
work while device acceptance is still open. The prior
[dependency review](dependency-upgrade-review.md) records lifecycle and compatibility questions;
its old observations must be rechecked before implementation.

Large modules identify inspection targets, not proof of duplication: `comms/src/store.rs` has
4,167 lines, `finance/src/server.rs` 3,589, and `sjel-cli/src/doctor/checks.rs` 3,110 at review.
Split a module around independently used behavior or a test boundary. A file-size target alone
can produce more wrappers and navigation without removing any maintenance.

| Tool | Useful boundary | Recommendation |
|---|---|---|
| [cargo-machete](https://github.com/bnjbvr/cargo-machete) | Unused Rust dependencies | First cleanup candidate; review generated-code and renamed-crate false positives; do not auto-delete |
| [Knip](https://knip.dev/) | Unused TS dependencies, exports and files | Trial on one Bun owner; model extension entry points before trusting findings |
| [jscpd](https://github.com/kucherenko/jscpd) | Repeated token sequences across source files | Discovery report only; exclude generated, vendored and fixture trees; prioritize by shared behavior |
| [cargo tree](https://doc.rust-lang.org/cargo/commands/cargo-tree.html) | Duplicate versions and feature activation | Use the existing Cargo tool before adding a dependency-graph service |
| [cargo-hakari](https://docs.rs/cargo-hakari/latest/cargo_hakari/) | Workspace feature unification to reduce rebuilds | Defer until measured feature-induced rebuild cost justifies a generated workspace-hack crate |

No cleanup tool was installed or run. No duplicate-code percentage or unused-dependency total
has been established. Candidate capability descriptions are project-documented, not Sjel
performance results.

## Security work that can be reused

Sjel already uses OSV and Gitleaks in `crates/sjel-cli/src/audit.rs`, Grype and OSV in the
Security workflow, CodeQL in its workflow, and Socket in each Bun install configuration.
`tools/check-bun-install-policy.sh` checks scanner declarations, release age and script policy.
The existing [Bun scanner API](https://bun.sh/docs/pm/security-scanner-api) can stop installation
on fatal findings. Adding another Socket integration would duplicate current coverage.

These tools answer different questions. [OSV](https://google.github.io/osv-scanner/) matches
dependencies to known advisories. [Socket](https://docs.socket.dev/docs/socket-for-github)
examines package behavior such as install scripts and privileged APIs. Neither establishes
that natural-language instructions are benign. Gitleaks detects secret patterns; it does not
enforce what a running agent can read.

| Candidate | Assessment for Sjel |
|---|---|
| [Cisco Skill Scanner](https://github.com/cisco-ai-defense/skill-scanner) | Best first evaluation for replacing custom skill detection: static patterns, code analysis, optional LLM review and machine-readable findings. Apache-2.0; supports local model configurations. Keep remote analyzers off for a local trial. |
| [Snyk Agent Scan](https://github.com/snyk/agent-scan) | Useful broader MCP/skill discovery comparator. Analysis requires service authentication; the project warns that CLI output is experimental. MCP scanning can start configured servers and contact remote endpoints. A trial must isolate execution and establish egress; avoid automatic host-wide discovery. |
| [cargo-deny](https://embarkstudios.github.io/cargo-deny/checks/index.html) | Fills Rust license, source and dependency-ban policy gaps. Evaluate those checks first; keep OSV as the existing advisory owner instead of adding a second routine CVE queue. License allowlists still require a project decision. |
| [Cargo Vet](https://mozilla.github.io/cargo-vet/) | Shares trusted audits and supports version-diff audits and exemptions. It reduces repeated reading when relevant audits exist, but creates a coverage obligation. Defer a mandatory rollout until an audit-import coverage trial proves lower effort. |

Cisco explicitly describes best-effort detection. Its README reports 7.7% HIGH-severity recall
for rules alone on its held-out corpus; adding its specified judge changes the operating
point. Those vendor-reported figures are not a comparison with Sjel's rules. Benchmark benign
security documentation as well as malicious fixtures before replacing a gate. Snyk's
[CLI reference](https://raw.githubusercontent.com/snyk/agent-scan/main/docs/cli-reference.md)
documents authentication and version-dependent APIs; it is not a local-only alternative.

The independent [ColluSkill preprint](https://arxiv.org/abs/2608.09732) studies malicious behavior
distributed across skills. Its experimental results do not measure this installation, but
support retaining execution restrictions when individual skills scan clean. It is research
evidence, not a reason to add another production scanner or claim complete protection.

### Keep a small Sjel admission layer

`crates/sjel-cli/src/intake/{mod,rules,review,skills}.rs` currently combines scanning, model
review, approvals and installation. Its useful Sjel-specific property is content binding:
stage once, hash paths/bytes/executable bits, review those bytes and publish the same snapshot.
Keep that machinery. The vendor-audit skill already recommends reviewing only changed surfaces
after a prior audit; changing this principle is unnecessary.

The current gate reuses exact approvals and harness receipts. The ordinary inference-role
review does not record an equivalent receipt (`intake::evidence` documents this). That is a
concrete repeat-work candidate, although this review has not measured how often it occurs.
Current receipts also lack scanner/ruleset and policy identity.

Recommended evaluation contract:

1. Trial one upstream detector on synthetic fixtures and explicitly public, tracked skill
   content. Record source, license and evaluation verdict in `upstreams.toml` before integration.
2. Normalize only fields needed by existing intake: detector/ruleset identity, file evidence,
   severity and completion. A scanner outage stays distinguishable from no findings.
3. Reuse completed content-review evidence only while content, executable bits, review context,
   detector/model identity and admission policy remain applicable. Re-evaluate permission or
   destination changes. Keep advisory-database scans scheduled even when content is unchanged.
4. Present one deduplicated finding queue. Narrow exceptions carry scope, reason and expiry;
   unchanged accepted findings should not require the same explanation on every update.
5. Replace overlapping custom detector code only after the trial meets the existing gate's
   acceptance cases. Keep installation verification, containment and publication boundaries.

This is an implementation proposal. It grants no admission bypass, bulk approval or permission
to upload private skills. Changing mandatory review thresholds remains a product decision.

### Avoid another maintenance framework

Dependabot already covers both Cargo workspaces, Actions and the five Bun projects. Keep it.
[Renovate](https://docs.renovatebot.com/configuration-options/) supports grouping and release-age
rules, but those features alone do not justify switching bots. Reconsider only if a concrete
coordinated-update requirement cannot be met with the current setup.

[Lychee](https://lychee.cli.rs/) is a candidate for local Markdown/HTML links. First check whether
the website builder's existing internal-link validation can cover contributor docs too. Neither
one can validate arbitrary backticked source paths without extraction and contextual rules.
Keep the existing stale-path tooling item; do not build a second document registry.

## Limits and next boundary

`tools/doctor` exited 1 with one failing host-patch audit check. It also reported unavailable
host integrations, unverified scheduled producers and backup destinations unavailable on this
machine. These are separate from planning drift; the old plan's backup-timer hypothesis was
not established by this run. No new vulnerability scan or restore test ran.

Concurrent GPUI and restructure edits were present and were not changed by this review. Counts
describe that uncommitted checkout. Native behavior, scanner efficacy, dependency
removal, build-time improvement and current remote issue states remain unverified.

Interceptor returned exit 1 without diagnostics for its help and research initialization.
Public-source research used the available web tool; no Interceptor ledger or browser group was
created. The candidate comparison uses published documentation; source-code security audits of
the tools remain outstanding.

The next implementation boundary is the existing tooling plan's maintenance report and scanner
evaluation. Reconcile source-backed completed entries before choosing any new architecture.
