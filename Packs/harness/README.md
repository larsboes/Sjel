

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

`tools/harnesses` is the cross-harness front end over the per-harness `packs-*` adapters and
materialized harness destinations, including GitHub Copilot CLI's `~/.copilot/skills`:

```bash
tools/harnesses list                     # which harnesses exist, and which are installed here
tools/harnesses status [<pack>]          # one matrix: every Pack skill x every harness
tools/harnesses drift [<pack>] [--diff]  # per-file detail, with the diff
tools/harnesses sync <pack>|--all        # one-way Sjel -> harness
tools/harnesses promote <skill> --pack <p>   # a harness skill Sjel does not own
tools/harnesses accept <pack> <skill>        # an edit to a skill Sjel already owns
```

GitHub Copilot CLI settings are managed separately from Pack contents:

```bash
sjel copilot apply                         # autopilot + assisted permissions + sandbox, and the guard hook
sjel copilot check                         # report drift from the settings baseline and the guard
sjel copilot apply --dry-run               # say what would change, write nothing
tools/copilot                             # launcher: warns when the guard hook is missing, then execs copilot
```

The baseline enables Copilot's sandbox, disables its bypass and keychain access, masks GitHub
authentication through the sandbox proxy, keeps custom agents local, disables cross-session
memory, and denies direct access to common Git credential files.

Copilot persists no tool-deny setting — `--deny-tool` is a session flag — so the blocks on `rm`,
`git commit` and `git push` live in `~/.copilot/hooks/sjel-guard.sh`, a command `preToolUse` hook
that `sjel copilot apply` writes and `sjel copilot check` patrols. Copilot loads user hooks for
every session, and a command `preToolUse` hook is fail-closed: a crash or a non-zero exit denies
the tool call. Verified 2026-10-24 — `copilot -p '… rm -f /tmp/x' --allow-all`, run without the
launcher, was denied by the hook and the file survived.

Three limits, stated because each one is a place the floor is absent:

- `tools/templates/copilot/hooks/sjel-guard.sh` matches a blocked command as the first word of a
  shell segment. `xargs rm`, `find -delete`, `bash -c 'git push'` and git aliases are not matched.
  It is a guard against the obvious command, not a sandbox.
- A session that sets `disableAllHooks`, or a user who deletes the hook files, has no blocks.
  `check` notices the deletion; it cannot notice the setting if nobody runs it.
- The non-bypassable layer is a root-owned hook in `/etc/github-copilot/policy.d/`, which Sjel
  does not install. That needs sudo and is a machine-wide decision, not a per-user one.

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
improve their behavior. Human-writing and unslop are also maintained in their Packs.
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

[tintinweb/pi-subagents](https://github.com/tintinweb/pi-subagents), MIT, revision `v0.22.0`
(2026-10-24; 0.19.0/`e955e29` was the previous pin). Full grant text in this Pack's [`LICENSE`](LICENSE).

These are the three things in this pack that are somebody else's code, and this is why each is
here rather than left to its own installer: pi has no subagent tool without pi-subagents,
`deliberation`'s agent files are inert on pi until it loads, and the two below are the context
manager and the web access every session uses.

| | |
|---|---|
| Vendored | `pi-packages/pi-subagents/` — `src/`, `package.json`, `tsconfig.json`, `LICENSE` |
| Not copied | `docs/`, `examples/`, `.github/`, `media/`, and the upstream README and CHANGELOG. `test/` was not copied either — the one file in it now is ours |
| Not committed | `node_modules/` (installed in place), and upstream's own lockfile (the local `bun.lock` is committed) |
| Local deltas | FOUR source files differ from upstream `v0.22.0`, plus packaging. (1) `src/model-resolver.ts` scopes a `provider/modelId` query to that provider, added 2026-10-03 with `test/model-resolver.test.ts` as its regression — upstream never took it. OpenRouter publishes the upstream provider inside the model id, so its `anthropic/claude-haiku-4.5` normalizes to exactly the query `anthropic/claude-haiku-4-5`, scored 100 in the fuzzy pass, and beat every real candidate — which routed the built-in `Explore` agent's pin to a gateway the config never named, where the request was refused on that account's balance. An unreachable pin now falls through to the bare id on another provider (step 3, as documented) or reports itself unresolved (step 4), instead of silently choosing a different gateway. (2) `src/agent-runner.ts` keeps two guards 0.22.0 dropped: `runAgent` throws on its signal before it consults the parent's system prompt or creates a session, and `finalTurnError` reports a provider `aborted` stop as a failure, leaving a caller-initiated stop to the manager's flag. (3) `src/abortable.ts` observes work already started when its caller stops waiting, so a later rejection is still handled. (4) One doc comment in `src/output-file.ts`: its POSIX path example became a placeholder so it stops reading as a workstation path to `tools/check-publication-hygiene.sh`. No behavioural change. `src/workflow/collisions.ts` needed nothing — 0.22.0 ships the lowercase `workflow` entry the old local patch added. Packaging: `"overrides": { "undici": "^8.12.3" }` (see the CVE note below), host-provided `@sinclair/typebox` and `typebox` kept as `"*"` peers rather than devDependencies with the three pi peers also `"*"`, and the three pi SDK devDependencies pinned to `1.4.0` — the version this machine runs — because their only job is `bun run typecheck`. `tsconfig.json` is ours, kept from the first vendoring so a customization can be typechecked before a restart. |

| Owner | Sjel. Upstream is a source to re-read, not a dependency that updates itself — `pi update` does not touch a local-path package |

**The Pack carried no agent file until 2026-10-03.** It does now: `agents/Explore.md` overrides pi-subagents' built-in `Explore` — the only agent in that registry pinning a model (`anthropic/claude-haiku-4-5`), which is the one thing `tools/lib/pi-agent-file.ts` exists to strip on pi. A deployer cannot know which providers a machine can actually reach, and on this one the pin reached none that serve Haiku: no `anthropic` provider at all, `opencode` out of funds, `github-copilot` refusing the id. The override carries no `model:` line, so Explore inherits the session model, and anything wanting a cheaper tier has to name it on a machine that can reach it. There is no `provider/default-model` mechanism to fall back on: pi declares one default chat model per provider, not a fast or small tier, so "cheap where the provider defines one, inherit otherwise" is "inherit" today.

The file is Claude-native like every other agent source in the repo, but it is deployed to pi only, deliberately: Claude Code is Anthropic-native, so the built-in pin resolves there and there is nothing to fix — deploying it would *remove* a working Haiku tier from that harness. The `agents/` row for `harness` in `tools/harnesses status` therefore stays empty in every column, and that is a recorded state rather than an unfinished one.

That decision was prose until 2026-10-09, and prose does not stop a deploy: `tools/packs-claude sync harness` writes every unit a Pack carries, so running it to ship an edit to `harness-sync` also put `Explore.md` into `~/.claude/agents/harness/` — the exact harm above, done by the routine verb. It is now declared in `profiles.toml`'s `[unit_hold]` table as `claude = ["harness/agents/"]`: `deploy` and `sync` skip the unit and say why, and `tools/doctor` reports it as held back instead of warning that it is missing — and as a violation if it is ever deployed anyway. A held unit is never removed by `sync` either, so the override is reported rather than silently undone.

**Drift closed, measured 2026-10-24.** The pi SDK devDependencies used to pin `0.84.2`, the
version upstream developed against, and raising them to `0.99.2` failed `bun run typecheck` in
`src/mention-clone.ts` — `ExtensionContext` no longer assignable to `ExtensionToolContext`, and
`systemPrompt` read-only. `v0.22.0` rewrote that path, so the pins now sit at `1.4.0` — the
version this machine runs — and `bun run typecheck` passes with no local patch in the
mention-clone path. The measurement above is kept because it is why the pins were left alone for
three weeks, not because it still holds.

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
| pi-subagents: Pi delegation, agent types and workflow UI | Against `v0.22.0` (2026-10-24), 55 of 59 `src/` files match. Four differ: `model-resolver.ts` (provider scoping), `agent-runner.ts` (pre-flight abort, provider-abort failure), `abortable.ts` (observe already-started work), `output-file.ts` (a comment). `workflow/collisions.ts` now matches — upstream took the lowercase `workflow` entry. Packaging: `package.json`, plus this tree's `tsconfig.json`, `bun.lock` and `test/`. | Keep and improve locally. Preserve provider-scoped resolution and the two abort guards; investigate cancellation, results and tool-permission handling. Keep per-machine model choices outside the source. |
| pi-web-access: Pi web search, fetch, PDFs and video | Against `v0.44.0` (2026-10-24), 88 of 92 root `*.ts` files match. Four differ, each restoring a contract 0.44.0 dropped: `summary-model-scope.ts` (enabled-scope filter), `search-answer-formatting.ts` (clickable source links), `extract.ts` + `declared-web-links.ts` (the `document.baseURI` anchor that makes attribute links absolute — measured: `[guide](guide)` without it). Plus `package.json`, `.gitignore`, `bun.lock` and this tree's `tsconfig.json`. | Keep and improve locally. Review provider routing, extraction quality, source attribution and overlap with Interceptor/Defuddle. Keep credentials and machine defaults outside the source. |
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

[nicobailon/pi-web-access](https://github.com/nicobailon/pi-web-access), MIT, revision `v0.44.0`
(2026-10-24; 0.35.0/`72c6e67` was the previous pin). Full grant text in this Pack's [`LICENSE`](LICENSE).

The web search, page fetch, PDF extraction and video analysis every session uses, across roughly
forty provider adapters. It arrived here as `npm:pi-web-access`, an opaque version we could not
patch when a provider changed shape. Re-vendored 2026-10-24 from 0.35.0; nine releases landed in
between. Two of them changed what this tree has to carry: 0.44.0 deleted the separate
`opencode-session-headers.ts` and passes `sessionId` to pi-ai instead, so the OpenCode
`400 MissingSessionID` workaround is now the SDK's (`fetch_content` answer mode against
`opencode-go` is the call it protects), and the HTML pipeline moved inside the private
`extractViaHttp`, taking the exported `extractHtmlContent` seam with it — which is why
`tools/pi-local-adaptations.test.ts` now drives that contract through `extractContent` and a
stubbed `fetch`.

| | |
|---|---|
| Vendored | `pi-packages/pi-web-access/` — the root `*.ts` sources (92 files), `package.json`, `tsconfig.json`, `LICENSE` |
| Not copied | `dist/` (the precompiled bundle the published package points at), `test/`, `scripts/`, `pi-web-fetch-demo.mp4`, `banner.png`, `CHANGELOG.md`, `README.md`, `SECURITY.md`, and the upstream lockfile |
| Not committed | `node_modules/` (installed in place); the local `bun.lock` is committed |
| Local deltas | FOUR source files plus packaging. (1) `summary-model-scope.ts` keeps the enabled-scope filter 0.44.0 dropped: `findModelWithProviderRouting` takes the scoped model list and refuses a registry fallback outside it, so a disabled first router cannot hide an enabled one. (2) `search-answer-formatting.ts` keeps clickable markdown source links (`Source: [title](<url>)`) where 0.44.0 emits a bare `Source: title (url)`, preserving the provider's URL verbatim. (3) `extract.ts` anchors linkedom's `document.baseURI` to the declared `<base>`, falling back to the URL the fetch ended at, before Readability runs. linkedom parses with `about:blank`, so without this a relative href in the article stays relative — measured before the patch: `[guide](guide)`. (4) `declared-web-links.ts` exports `resolveHttpUrl` for that anchor instead of copying it. Packaging, all in `package.json`: `pi.extensions` is `["./index.ts"]` rather than upstream's `["./dist"]`, because this is a source checkout and `prepublishOnly`/`postpublish` rewrite the field around publication; no `bin` and no `files`, since `bin` points into a `dist/` this tree does not build; `"overrides": { "undici": "^8.12.3" }` — upstream depends on `^8.11.2` itself, but the vendored dev copy of `@earendil-works/pi-coding-agent` pins `undici` **exactly**, so the override is what keeps one patched copy; the three pi SDK devDependencies pinned to `1.4.0`, the version this machine runs; and `typebox` stays a `"*"` peer with no devDependency, where upstream lists `typebox@1.3.27` as one — adding it installs a second copy beside Pi's, which is what 0.32.0 changed to stop the duplicate-module startup warning. `tsconfig.json` is this tree's own, kept from the first vendoring so a customization can be typechecked before a restart |
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
