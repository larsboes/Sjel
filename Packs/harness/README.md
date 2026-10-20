

# harness pack

Every other Pack does work. This one maintains the thing that does the work — and the repository
that holds it: what skills should exist, what should stop being loaded into every session, and how
to operate Sjel itself.

- **`sjel`** operates and changes the Sjel repository: capability discovery, service and feed
  operations, architecture placement, focused change. It carries the `sjel` CLI
  (`help`, `search`, `doctor`, `context`, `storage`, `gates`, `test`, `cargo`, `capability`,
  `pack`) and the rules for working in this repo — where a change belongs, what to read before
  making it, and which verification is proportionate to it. Merged into this Pack 2026-09-17;
  see the note at the end of this section.
- **`suggest-skills`** reads this machine's own prompt history and installed skills, finds the work
  that recurred across sessions and the places the user had to correct the agent, and returns at
  most three proposals — each with the evidence that produced it and the coverage check that
  survived. It is read-only and proposal-only by construction: it cannot create or edit a skill,
  because a proposer that can also build grades its own homework.
- **`harness-sync`** answers where every Pack skill is deployed across the harnesses installed on
  this machine, what has drifted, and which of the two reverse moves keeps an edit someone made
  inside a harness. It drives `tools/harnesses` and carries the judgment the tool's `--help`
  cannot: sync is one-way and destructive at the destination by design, and the two moves back into
  Sjel (`promote`, `accept`) are manual because an edit made inside a harness is a decision.
- **`trim`** measures what actually loads every session — the user and project `CLAUDE.md`, their
  `@`-imports, the memory index — removes what is provably dead, then makes each judgment call in
  front of the user, one at a time, under a gate that refuses any edit which loses a directive.

The three maintenance skills put a deterministic script in front of the model.
`collect-signals.ts` gathers and counts; `measure-context.ts` measures and resolves. Neither
judges, and neither writes. Two runs over the same files return the same corpus, so a proposal can
be argued with instead of trusted, and the model spends its judgment on the part that needs
judgment. `sjel` is the exception and is not that shape: it is a runbook over an existing CLI.

**Why `sjel` is here.** It was a one-skill Pack. Every profile that held it also held this one, and
both answer the same question from the same direction — the agent system looking at itself. A Pack
is a deployment unit, not a category, so co-deployment is the test; the skills keep their names, so
nothing changed about what loads. Note the consequence for profiles: `productive` held `sjel` but
not this Pack, so it now selects `skills = { "harness" = ["sjel"] }` rather than dragging in three
maintenance skills it never loaded.

## The tool and the hooks

`tools/harnesses` is the cross-harness front end over the per-harness `packs-*` adapters:

```bash
tools/harnesses list                     # which harnesses exist, and which are installed here
tools/harnesses status [<pack>]          # one matrix: every Pack skill x every harness
tools/harnesses drift [<pack>] [--diff]  # per-file detail, with the diff
tools/harnesses sync <pack>|--all        # one-way Sjel -> harness
tools/harnesses promote <skill> --pack <p>   # a harness skill Sjel does not own
tools/harnesses accept <pack> <skill>        # an edit to a skill Sjel already owns
```

It exists because no adapter ever asked whether its harness was installed. On 2026-09-07 this
machine had three Packs materialized into `~/.agents/skills` for a Codex that is not installed,
while pi — installed, and one of the three harnesses actually in use — carried one Pack of
fourteen and had no row in `tools/doctor`. Deployment tracked adapters that exist rather than
harnesses that are installed. `tools/sjel-cli/src/harnesses/registry.rs` is now the one place that knows the
difference, and it also names what is NOT supported, so a report can say so instead of omitting
it. Antigravity was on that list until 2026-10-17, when it became a registry harness
(`tools/sjel-cli/src/harnesses/antigravity.rs`, which writes `~/.gemini/config/skills.json`).

Two Claude Code hooks call `tools/pack-drift-hook` and report without ever blocking:

```jsonc
// ~/.claude/settings.json
"hooks": {
  "SessionStart": [{ "matcher": "startup",
    "hooks": [{ "type": "command", "command": "<sjel>/tools/pack-drift-hook", "timeout": 15 }] }],
  "FileChanged":  [{ "matcher": "SKILL.md",
    "hooks": [{ "type": "command", "command": "<sjel>/tools/pack-drift-hook", "timeout": 15 }] }]
}
```

`SessionStart` reports drifted copies once, at the top of a session. `FileChanged` fires the moment
a SKILL.md under a harness skill root is written and says that the file is a deployed copy, naming
both moves. That is the moment the information is worth having: a week later the edit is either
lost to a sync or mysterious. Both print nothing when there is nothing to report, which is the
whole reason they can stay on — a hook that speaks every session gets muted, and a muted hook
reports nothing forever.

## The boundary

Three things sit near each other and do different jobs:

| Question | Where it is answered |
|---|---|
| *What* skill should exist? | `suggest-skills`, here |
| *How* is a skill written and audited? | `skill-creator`, in `Packs/writing` |
| What loads every session, and can it be smaller? | `trim`, here |
| Where is it deployed, and what drifted? | `harness-sync`, here |
| How do I operate or change Sjel itself? | `sjel`, here |

`suggest-skills` hands its shortlist to the authoring skill and stops. The authoring skill never
decides what to build. Keeping the two apart is the point: the same agent doing both will propose
what it feels like writing.

The authoring skill is `skill-creator` (`Packs/writing`), promoted into Sjel 2026-09-11 from the
private overlay's staging `meta` pack and superseding `writing-skills`, which the writing pack now
retires. The boundary above is about jobs, not about which authoring skill currently holds the
second row.

## Scripts

```bash
bun skills/suggest-skills/scripts/collect-signals.ts --days 30 --json   # the corpus
bun skills/trim/scripts/measure-context.ts --root "$PWD"                # the always-on set
```

`harness-sync` ships no script of its own: its tool is `tools/harnesses` in the repository, because
deployment machinery is operator machinery and belongs in `tools/`, not inside a skill that gets
copied into every harness.

Both read only local files — the harness history, the installed skills, the context files — and
neither sends anything anywhere. `collect-signals.ts` prints raw prompt text, so its output is as
private as the prompts were: keep it local, and do not paste it into anything that leaves the
machine.

## Activate

```bash
"$SJEL_ROOT/tools/packs.sh" link harness      # → ~/.claude/skills/{suggest-skills,trim,harness-sync}
"$SJEL_ROOT/tools/packs-pi" deploy harness    # → registered in ~/.pi/agent/settings.json
                                             #   skills, the vendored pi package by path,
                                             #   and ~/.pi/agent/agents/Explore.md
```

`deploy harness` also registers `pi-packages/pi-subagents` as a path in settings.json
`packages`. That is a third artifact kind, alongside `skills/` and `extensions/`, and it exists
because pi's own `pi install` cannot keep a package inside a repo you edit: an npm source is
opaque, and a git source is cloned to `~/.pi/agent/git/` and reset on reconcile. A local path is
neither, so the checkout stays in the Pack and a customization is an edit rather than a
re-vendor. Its dependencies are installed in place and never committed:

```bash
cd "$SJEL_ROOT/Packs/harness/pi-packages/pi-subagents" && bun install
```

## Attribution

Sjel distinguishes optional products from intentionally maintained adaptations. Accordion is
an optional upstream product. Pi-subagents and pi-web-access are maintained here so Sjel can
improve their behavior. Human-writing, unslop and Anatomy are also maintained in their Packs.
These are explicit ownership decisions, not a license to copy every discovered project.

Keep upstream attribution and a reviewed source revision. Evaluate incoming changes against
local behavior and tests; do not replace these trees with stock installations automatically.
Machine configuration and private values remain outside the public source.

Ported from Daniel Miessler's LifeOS, MIT: <https://github.com/danielmiessler/LifeOS>
(`SuggestSkills` and `Trim`, read 2026-09-07). The register verdict for that upstream is
`inspiration` — see the `[lifeos]` row in `upstreams.toml`. No code was vendored: both scripts here
are ours, written against the files this machine actually has.

What the port changed, and why:

| Upstream | Here |
|---|---|
| A satisfaction/rating store as the frustration signal | There is no rating store on this machine. Friction is inferred from the wording of a prompt and the prompt before it, and `suggest-skills` states that this over-fires and under-fires rather than presenting it as data. |
| `Tools/CollectSignals.ts` over the LifeOS session and rating stores | `collect-signals.ts` over `~/.claude/history.jsonl` and the installed skill roots, with every source overridable by flag. |
| Trim's deterministic pass = `ProposalGC.ts` over a LifeOS proposal inbox | No such inbox exists here. The deterministic pass became dead pointers and directives duplicated across two always-on files — both checkable, both free, and both found on the first real run. |
| Trim commits to a private USER_DATA repository | Harness-neutral reversibility: the repository when the file is in one and clean, a timestamped `.bak` copy with a printed restore command when it is not. `~/.claude` is not a repository here. |
| The voice notification, the JSONL execution log, the CUSTOMIZATIONS preamble | Removed, for the reasons the `deliberation` Pack's README already records — they address a LifeOS layout Sjel does not run. |

### pi-subagents (vendored, not ported)

[tintinweb/pi-subagents](https://github.com/tintinweb/pi-subagents), MIT, pinned `e955e29`
(v0.19.0 plus one commit, 2026-09-16). Full grant text in this Pack's [`LICENSE`](LICENSE).

These are the three things in this pack that are somebody else's code, and this is why each is
here rather than left to its own installer: pi has no subagent tool without pi-subagents,
`deliberation`'s agent files are inert on pi until it loads, and the two below are the context
manager and the web access every session uses.

| | |
|---|---|
| Vendored | `pi-packages/pi-subagents/` — `src/`, `package.json`, `tsconfig.json`, `LICENSE` |
| Not copied | `docs/`, `examples/`, `.github/`, `media/`, and the upstream README and CHANGELOG. `test/` was not copied either — the one file in it now is ours |
| Not committed | `node_modules/` (installed in place), and upstream's own lockfile (the local `bun.lock` is committed) |
| Local deltas | FOUR. (1) One doc comment in `src/output-file.ts`: its POSIX path example became a placeholder so it stops reading as a workstation path to `tools/check-publication-hygiene.sh`. No behavioural change. `tsconfig.json` is upstream's, added to the vendored set so a customization can be typechecked before a restart. (2) `"overrides": { "undici": "^8.10.2" }` in `package.json`, added 2026-09-29 for GHSA-3wwx-pv8p-q78v / CVE-2026-85024: undici's WebSocket client kills the whole process on a malformed permessage-deflate block, fixed in 8.10.2. An update cannot reach it, because the vendored dev version of `@earendil-works/pi-coding-agent` pins `"undici": "8.9.0"` **exactly** — so the resolution sat one patch below the fix. `bun install` resolved the lockfile to 8.11.2 and `osv-scanner` reports no issues. (3) Host-provided `@sinclair/typebox` and `typebox` moved from runtime dependencies to `"*"` peers; the three pi peers also use `"*"`. This follows Pi 0.99's extension-loader contract and removes duplicate-module warnings. (4) `resolveModel` scopes a `provider/modelId` query to that provider, added 2026-10-03 with `test/model-resolver.test.ts` as its regression. OpenRouter publishes the upstream provider inside the model id, so its `anthropic/claude-haiku-4.5` normalizes to exactly the query `anthropic/claude-haiku-4-5`, scored 100 in the fuzzy pass, and beat every real candidate — which routed the built-in `Explore` agent's pin to a gateway the config never named, where the request was refused on that account's balance. An unreachable pin now falls through to the bare id on another provider (step 3, as documented) or reports itself unresolved (step 4), instead of silently choosing a different gateway. |

| Owner | Sjel. Upstream is a source to re-read, not a dependency that updates itself — `pi update` does not touch a local-path package |

**The Pack carried no agent file until 2026-10-03.** It does now: `agents/Explore.md` overrides pi-subagents' built-in `Explore` — the only agent in that registry pinning a model (`anthropic/claude-haiku-4-5`), which is the one thing `tools/lib/pi-agent-file.ts` exists to strip on pi. A deployer cannot know which providers a machine can actually reach, and on this one the pin reached none that serve Haiku: no `anthropic` provider at all, `opencode` out of funds, `github-copilot` refusing the id. The override carries no `model:` line, so Explore inherits the session model, and anything wanting a cheaper tier has to name it on a machine that can reach it. There is no `provider/default-model` mechanism to fall back on: pi declares one default chat model per provider, not a fast or small tier, so "cheap where the provider defines one, inherit otherwise" is "inherit" today.

The file is Claude-native like every other agent source in the repo, but it is deployed to pi only, deliberately: Claude Code is Anthropic-native, so the built-in pin resolves there and there is nothing to fix — deploying it would *remove* a working Haiku tier from that harness. The `agents/` row for `harness` in `tools/harnesses status` therefore stays empty in every column, and that is a recorded state rather than an unfinished one.

That decision was prose until 2026-10-09, and prose does not stop a deploy: `tools/packs-claude sync harness` writes every unit a Pack carries, so running it to ship an edit to `harness-sync` also put `Explore.md` into `~/.claude/agents/harness/` — the exact harm above, done by the routine verb. It is now declared in `profiles.toml`'s `[unit_hold]` table as `claude = ["harness/agents/"]`: `deploy` and `sync` skip the unit and say why, and `tools/doctor` reports it as held back instead of warning that it is missing — and as a violation if it is ever deployed anyway. A held unit is never removed by `sync` either, so the override is reported rather than silently undone.

**Known drift, measured 2026-10-01.** The pi SDK devDependencies here pin `0.84.2`, the version
upstream developed against, while this machine runs Pi `1.0.0`. Raising them to `0.99.2` makes
`bun run typecheck` fail in `src/mention-clone.ts` — `ExtensionContext` is no longer assignable to
`ExtensionToolContext` (missing `tools`, `executeTool`, line 128) and `systemPrompt` is read-only
(line 178). The pins were left where upstream put them rather than carrying a local patch into the
mention-clone path; the drift is written down so the next person decides with the measurement in
hand instead of rediscovering it. Editing the tree is allowed — this is a choice, not a rule.
`e955e29` is upstream HEAD, so there is nothing newer to take either. The mention-clone path is the
one that may feel this at runtime, since the second error is a property Pi now marks read-only.

This reverses, for these three packages, the convention every other Pack README states: that a
third-party tool is "driven through its own install/update tooling, never vendored" (see
`tools/agent-integrations.sh`). That convention is right for anything we merely consume. It is
wrong for the extension that carries our agent types, because the alternative is a package that
updates under the deliberation Pack without either of them knowing — and because the ability to
change a subagent's behaviour is the reason this Pack exists at all.

### Local CLM classifier (experimental Pi extension)

`extensions/clm-classifier.ts` is an opt-in adapter for the real CLM System One API, not a substitute for its encoder or projection heads. `/clm-probe` is registered either way — with the gate shut it names the variable to set rather than probing — but the classifier itself is not registered unless `SJEL_CLM_ENABLE=1`, so a session that has not opted in holds no provider that could reach the service. It never connects to anything unless `SJEL_CLM_ENABLE=1`.

The same adapter serves two local servers, because both expose the same `POST /v1/systemone` shape: CLM's reference server ([API reference](https://github.com/Contrastive-LM/CLM#api-reference)) and Ollama 0.35 or later ([System One API](https://docs.ollama.com/api/systemone)). Three variables select the server:

| Variable | Default | Meaning |
|---|---|---|
| `SJEL_SYSTEMONE_BACKEND` | `clm` | `clm` or `ollama`. Sets the provider (`sjel-clm` or `sjel-ollama`) and the defaults below. |
| `SJEL_SYSTEMONE_URL` | `http://127.0.0.1:8700/v1/` for CLM, `http://127.0.0.1:11434/v1/` for Ollama | Must be `http://` on loopback. Any other address is refused, so a setting cannot send a prompt off this machine. |
| `SJEL_SYSTEMONE_MODEL` | `clm-latest` for CLM, `nimble` for Ollama | Model id. Ollama may answer with the `:latest` tag. The adapter accepts that tag and no other model. |

Try it without deploying the Pack:

```bash
SJEL_CLM_ENABLE=1 pi -e ./Packs/harness/extensions/clm-classifier.ts
SJEL_CLM_ENABLE=1 SJEL_SYSTEMONE_BACKEND=ollama pi -e ./Packs/harness/extensions/clm-classifier.ts
# In pi: /clm-probe
```

For CLM, `/clm-probe` checks that `/health` reports a working encoder, the `clm-latest` head and no mock mode. Ollama has no such route. For Ollama the probe reads `/api/tags` and checks that the model is pulled. Then the probe asks one synthetic question. Accuracy on Sjel-shaped decisions is measured separately in [`research/benchmarks/decisions/`](../../research/benchmarks/decisions/README.md). Only **after** a real service passes that probe can Pi scripts call `models.getModelOfType("classifier", "sjel-clm", "clm-latest")` (or `"sjel-ollama", "nimble"`) and `models.classify(...)` (enable codemode with `--tools read,bash,edit,write,codemode` for a one-off session). A passing synthetic probe checks the wire contract, **not** decision quality: compare labeled Sjel routes against the current keyword baseline before activating any product route. `tools/pi-clm-classifier.test.ts` checks the adapter against synthetic responses without claiming a model was run.

The CLM v0.1 release is Apache-2.0 (checkpoint and code, <https://huggingface.co/Contrastive-LM/CLM-v0.1-8B>); it needs the matching Qwen3-8B last-token pooling encoder plus its trained heads. An Ollama `/api/embed` cosine of Qwen3-8B vectors is **not CLM**. The reference `contrastive-lm` package currently installs vLLM, and its documented serving recipe targets an NVIDIA GPU. vllm-metal documents experimental LAST pooling on macOS, but that is not yet an end-to-end validation of this checkpoint. On this Mac the service and Qwen3-8B are not installed; with 35 GiB free, no model download or background service was started. Keep this extension off until an exact encoder/head match and labeled accuracy are measured. Hosted Jev through Pi's built-in `typesafe/jev-latest` is the next, separate step after local validation; it needs credentials and Sjel's egress approval before personal data is sent.

### Accordion (optional upstream install)

Accordion is not bundled or enabled by this Pack. The detached source, browser bundle,
conductors, and Sjel's default-on folding patches were removed. Session data remains owned by
Accordion; removal from this repository does not delete it.

```bash
tools/agent-integrations.sh list
tools/agent-integrations.sh install accordion pi   # explicit opt-in; delegates to pi install
```

The install source is `npm:@a-fig/accordion`. Pi owns the installed package and its updates
(`pi update`); Sjel carries no replacement source or default-conductor configuration.
`install --all-configured` does not include optional packages.

### Locally maintained adaptations

Audit method: compare tracked files with their recorded upstream Git trees, then compare with
upstream HEAD separately. Differences from a newer release are upstream changes, not evidence
of Sjel customization. The following findings were checked on 2026-10-20. No migration of these
five projects was performed. The user confirmed local ownership of all five after this audit;
retain the measured differences below and improve the source here.

| Project and purpose | Actual customization | Ownership decision |
|---|---|---|
| pi-subagents: Pi delegation, agent types and workflow UI | Against `e955e29`, 56 of 64 files match; 6 differ and 2 are local packaging files. Only `src/model-resolver.ts` changes runtime behavior; `src/output-file.ts` changes a comment. The remaining differences are checks, dependencies, formatting and a replacement regression-test file. | Keep and improve locally. Preserve provider-scoped resolution; investigate cancellation, results and tool-permission handling. Keep per-machine model choices outside the source. |
| pi-web-access: Pi web search, fetch, PDFs and video | Against `72c6e67` (0.35.0), 83 of 86 files match. Only `.gitignore` and `package.json` differ; `bun.lock` is local. No TypeScript source differs. Upstream HEAD is 0.43.0. | Keep and improve locally. Review provider routing, extraction quality, source attribution and overlap with Interceptor/Defuddle. Keep credentials and machine defaults outside the source. |
| Anatomy: interactive physical SVG explainers | Against `e948829`, 141 of 144 files match. Changes are `SKILL.md`, eval assertions, and an extracted `references/examples.md`. Drawing code, examples and verification scripts match. Newer HEAD `1482e84` adds printing; those differences are upstream evolution. | Keep and adapt locally for Sjel and Obsidian output. Preserve the geometry kit, examples and checks; add local integration through a focused reference. |
| human-voice within human-writing: prose linter and writing guidance | Of 17 copied script/data files present locally, 12 match `9bcba2f` and 5 differ. Changes add 14 patterns, flag a single em dash, remove inline suppressions, correct a reference path and annotate an import for Semgrep. The composed skill also draws from oberskills and design-tells. Current upstream reorganizes the linter extensively. | Keep the composed skill, linter and references in Sjel. Upgrade them locally and review upstream changes selectively; preserve existing behavior unless explicitly revised. |
| vibecoded-design-tells within unslop: code/UI scanners and review guidance | All 4 mapped reference files match `f7c4aef`. Both scanners differ only in displayed reference paths. The skill combines the code and UI workflows and adds Sjel ownership boundaries. | Keep the combined skill, scanners and references in Sjel. Improve them locally; use upstream as a reviewed source of changes. |

The pi-subagents routing fix still matters: with an unavailable
`anthropic/claude-haiku-4-5` pin and two available gateway entries, Sjel's resolver selects
`opencode/claude-haiku-4-5`; upstream HEAD `ca93a52` selects
`openrouter/anthropic/claude-haiku-4.5`. This was reproduced by importing each resolver against
the same synthetic registry. Do not drop the patch just because an upstream version is newer.

Configuration cannot reproduce every human-writing delta. Current human-voice supports
`.humanvoicerc` thresholds and weights, but its em-dash check also hardcodes a minimum count of
three, and inline suppressions run unconditionally. A threshold override alone preserves
neither Sjel's single-dash rule nor its ban on inline suppressions. Preserve those policies during local upgrades unless the user explicitly revises them.

### Local Pi behavior

The local source now preserves these contracts:

- Web summary, query-rewrite and page-answer model selection checks the enabled scope before
  choosing between native and routed models. A disabled first router cannot hide an enabled one.
- HTML extraction resolves relative links against the fetched document or its declared base,
  preserves useful short fallback text, and retains an extracted article when an optional
  fallback fails. Incomplete content stays labeled as incomplete. Search snippets retain
  clickable source links. Failed fetches also point to the existing Interceptor browser workflow
  for JavaScript or signed-in pages; this does not create a second browser integration.
- Subagents resolve canonical exact pins before fuzzy matching and reject empty or malformed
  pins. Cancellation before startup or resume cannot start a prompt. Cancelling a result wait
  leaves the child available; later rejection is still observed. A provider abort is reported
  as a failed result with partial output, while a user-stopped record keeps its stopped status.
- Existing extension-tool narrowing, denied tools and prior tool-call guards remain enforced
  when extensions register tools after startup.

`bun test tools/pi-local-adaptations.test.ts` exercises these contracts without live providers,
credentials or model spend. CI installs both packages' locked dependencies before collecting
these tests. Upstream remains a source of reviewed fixes and ideas; preserve local regression
cases when adopting changes.

### pi-web-access (vendored, detached)

[nicobailon/pi-web-access](https://github.com/nicobailon/pi-web-access), MIT, revision `72c6e67`,
version 0.35.0 (2026-10-01). Full grant text in this Pack's [`LICENSE`](LICENSE).

The web search, page fetch, PDF extraction and video analysis every session uses, across roughly
thirty provider adapters. It arrived here as `npm:pi-web-access`, an opaque version we could not
patch when a provider changed shape. Re-vendored 2026-10-01 from 0.29.0 (`192ac18`); six releases
landed in between, three of them on this machine's own defaults — OpenCode session headers without
which `fetch_content` answer mode against `opencode-go` returns `400 MissingSessionID` (0.30.0,
0.32.0), and `toolActivation: "auto"`, which stops turning web tools on from resending the
conversation and missing the prompt cache on DeepSeek-class models (0.34.0).

| | |
|---|---|
| Vendored | `pi-packages/pi-web-access/` — the root `*.ts` sources (81 files), `package.json`, `tsconfig.json`, `LICENSE` |
| Not copied | `dist/` (the 996K precompiled bundle), `test/` (1.0M, 82 files), `pi-web-fetch-demo.mp4`, `banner.png`, `CHANGELOG.md`, `README.md`, `SECURITY.md`, and the upstream lockfile |
| Not committed | `node_modules/` (installed in place); the local `bun.lock` is committed |
| Local deltas | FOUR, all in `package.json`. (1) `pi.extensions` is `["./index.ts"]`, not upstream's `["./dist"]`: since 0.30.0 the published package ships a precompiled bundle and points its manifest at it, while a source checkout loads the TypeScript directly — `prepublishOnly` rewrites the field to `./dist` and `postpublish` rewrites it back. This tree is a source checkout, so it keeps the source shape. (2) `"overrides": { "undici": "^8.11.2" }`, originally added 2026-09-29 for GHSA-3wwx-pv8p-q78v; upstream now depends on `^8.11.2` itself, but the vendored dev copy of `@earendil-works/pi-coding-agent` pins `undici` **exactly** (0.99.2 pins 8.10.2), so the override is what keeps one copy at 8.11.2. `osv-scanner scan --lockfile=bun.lock` reports no issues across 369 packages. (3) The three pi SDK devDependencies are pinned to `0.99.2` rather than `1.0.0`: the global 24h adoption hold in `~/.bunfig.toml` blocks 1.0.0 until 2026-10-02T19:15Z, and the point of the pin is to typecheck against the SDK we run. Bump it to 1.0.0 once the hold clears. (4) `typebox` stays a `"*"` peer with no devDependency, where upstream 0.35 lists `typebox@1.3.27` as one; adding it installs a second copy beside Pi's, which is what 0.32.0 changed to stop the duplicate-module startup warning. `tsconfig.json` is this tree's own, kept from the first vendoring so a customization can be typechecked before a restart |
| Owner | Sjel, **detached**: no `.git`, no remote. Materialized as a source checkout, not an installed package, so the vendored devDependencies exist only for `bun run typecheck` |

## Why this shape: the flip conditions

`suggest-skills` is worth keeping while it rejects. A run that proposes three skills every time it
is asked is pattern-matching on enthusiasm; the rejection table, with the test each candidate
failed, is the artifact that proves it did the coverage read. Two consecutive runs with an empty
rejection table and it has stopped checking.

`trim` flips on the deterministic half. If the dead-pointer and duplicate checks keep finding
something, the always-on set is drifting and the skill is paying for itself before any judgment
call. If they come back empty three runs running while the files keep growing, the growth is
load-bearing content and the honest answer is that these files are the right size — at which point
the skill's remaining value is the measurement, and a one-line script would do.
