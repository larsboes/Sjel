

# harness pack

Every other Pack does work. This one maintains the thing that does the work — and the repository
that holds it: what skills should exist, what should stop being loaded into every session, and how
to operate Axon itself.

- **`axon`** operates and changes the Axon repository: capability discovery, service and feed
  operations, architecture placement, focused change. It carries the `axon` CLI
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
  Axon (`promote`, `accept`) are manual because an edit made inside a harness is a decision.
- **`trim`** measures what actually loads every session — the user and project `CLAUDE.md`, their
  `@`-imports, the memory index — removes what is provably dead, then makes each judgment call in
  front of the user, one at a time, under a gate that refuses any edit which loses a directive.

The three maintenance skills put a deterministic script in front of the model.
`collect-signals.ts` gathers and counts; `measure-context.ts` measures and resolves. Neither
judges, and neither writes. Two runs over the same files return the same corpus, so a proposal can
be argued with instead of trusted, and the model spends its judgment on the part that needs
judgment. `axon` is the exception and is not that shape: it is a runbook over an existing CLI.

**Why `axon` is here.** It was a one-skill Pack. Every profile that held it also held this one, and
both answer the same question from the same direction — the agent system looking at itself. A Pack
is a deployment unit, not a category, so co-deployment is the test; the skills keep their names, so
nothing changed about what loads. Note the consequence for profiles: `productive` held `axon` but
not this Pack, so it now selects `skills = { "harness" = ["axon"] }` rather than dragging in three
maintenance skills it never loaded.

## The tool and the hooks

`tools/harnesses` is the cross-harness front end over the per-harness `packs-*` adapters:

```bash
tools/harnesses list                     # which harnesses exist, and which are installed here
tools/harnesses status [<pack>]          # one matrix: every Pack skill x every harness
tools/harnesses drift [<pack>] [--diff]  # per-file detail, with the diff
tools/harnesses sync <pack>|--all        # one-way Axon -> harness
tools/harnesses promote <skill> --pack <p>   # a harness skill Axon does not own
tools/harnesses accept <pack> <skill>        # an edit to a skill Axon already owns
```

It exists because no adapter ever asked whether its harness was installed. On 2026-09-07 this
machine had three Packs materialized into `~/.agents/skills` for a Codex that is not installed,
while pi — installed, and one of the three harnesses actually in use — carried one Pack of
fourteen and had no row in `tools/doctor`. Deployment tracked adapters that exist rather than
harnesses that are installed. `tools/lib/harness-registry.ts` is now the one place that knows the
difference, and it also names what is NOT supported (Antigravity: no adapter, no verified skill
format, nothing installed to measure against) so a report can say so instead of omitting it.

Two Claude Code hooks call `tools/pack-drift-hook` and report without ever blocking:

```jsonc
// ~/.claude/settings.json
"hooks": {
  "SessionStart": [{ "matcher": "startup",
    "hooks": [{ "type": "command", "command": "<bun> <axon>/tools/pack-drift-hook.ts", "timeout": 15 }] }],
  "FileChanged":  [{ "matcher": "SKILL.md",
    "hooks": [{ "type": "command", "command": "<bun> <axon>/tools/pack-drift-hook.ts", "timeout": 15 }] }]
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
| How do I operate or change Axon itself? | `axon`, here |

`suggest-skills` hands its shortlist to the authoring skill and stops. The authoring skill never
decides what to build. Keeping the two apart is the point: the same agent doing both will propose
what it feels like writing.

The authoring skill is `skill-creator` (`Packs/writing`), promoted into Axon 2026-09-11 from the
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
                                             #   skills, and the vendored pi package by path
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
| The voice notification, the JSONL execution log, the CUSTOMIZATIONS preamble | Removed, for the reasons the `deliberation` Pack's README already records — they address a LifeOS layout Axon does not run. |

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
| Not copied | `test/`, `docs/`, `examples/`, `.github/`, `media/`, and the upstream README and CHANGELOG |
| Not committed | `node_modules/` (installed in place), and upstream's own lockfile (the local `bun.lock` is committed) |
| Local deltas | THREE. (1) One doc comment in `src/output-file.ts`: its POSIX path example became a placeholder so it stops reading as a workstation path to `tools/check-publication-hygiene.sh`. No behavioural change. `tsconfig.json` is upstream's, added to the vendored set so a customization can be typechecked before a restart. (2) `"overrides": { "undici": "^8.10.2" }` in `package.json`, added 2026-09-29 for GHSA-3wwx-pv8p-q78v / CVE-2026-85024: undici's WebSocket client kills the whole process on a malformed permessage-deflate block, fixed in 8.10.2. An update cannot reach it, because the vendored dev version of `@earendil-works/pi-coding-agent` pins `"undici": "8.9.0"` **exactly** — so the resolution sat one patch below the fix. `bun install` resolved the lockfile to 8.11.2 and `osv-scanner` reports no issues. (3) Host-provided `@sinclair/typebox` and `typebox` moved from runtime dependencies to `"*"` peers; the three pi peers also use `"*"`. This follows Pi 0.99's extension-loader contract and removes duplicate-module warnings. |

| Owner | Axon. Upstream is a source to re-read, not a dependency that updates itself — `pi update` does not touch a local-path package |

This reverses, for these three packages, the convention every other Pack README states: that a
third-party tool is "driven through its own install/update tooling, never vendored" (see
`tools/agent-integrations.sh`). That convention is right for anything we merely consume. It is
wrong for the extension that carries our agent types, because the alternative is a package that
updates under the deliberation Pack without either of them knowing — and because the ability to
change a subagent's behaviour is the reason this Pack exists at all.

### Local CLM classifier (experimental Pi extension)

`extensions/clm-classifier.ts` is an opt-in adapter for the real CLM System One API, not a substitute for its encoder or projection heads. `/clm-probe` is registered either way — with the gate shut it names the variable to set rather than probing — but the classifier itself is not registered unless `SJEL_CLM_ENABLE=1`, so a session that has not opted in holds no provider that could reach the service. It never connects to anything unless `SJEL_CLM_ENABLE=1`; even then its endpoint is fixed to `127.0.0.1:8700`, so a model override cannot send a prompt off-machine. Try it without deploying the Pack:

```bash
SJEL_CLM_ENABLE=1 pi -e ./Packs/harness/extensions/clm-classifier.ts
# In pi: /clm-probe
```

`/clm-probe` checks that `/health` reports a working encoder, the `clm-latest` head and no mock mode, then asks one synthetic question. Only **after** a real service passes that probe can Pi scripts call `models.getModelOfType("classifier", "sjel-clm", "clm-latest")` and `models.classify(...)` (enable codemode with `--tools read,bash,edit,write,codemode` for a one-off session). A passing synthetic probe checks the wire contract, **not** decision quality: compare labeled Sjel routes against the current keyword baseline before activating any product route. `tools/pi-clm-classifier.test.ts` checks the adapter against synthetic responses without claiming a model was run.

The CLM v0.1 release is Apache-2.0 (checkpoint and code, <https://huggingface.co/Contrastive-LM/CLM-v0.1-8B>); it needs the matching Qwen3-8B last-token pooling encoder plus its trained heads. An Ollama `/api/embed` cosine of Qwen3-8B vectors is **not CLM**. The reference `contrastive-lm` package currently installs vLLM, and its documented serving recipe targets an NVIDIA GPU. vllm-metal documents experimental LAST pooling on macOS, but that is not yet an end-to-end validation of this checkpoint. On this Mac the service and Qwen3-8B are not installed; with 35 GiB free, no model download or background service was started. Keep this extension off until an exact encoder/head match and labeled accuracy are measured. Hosted Jev through Pi's built-in `typesafe/jev-latest` is the next, separate step after local validation; it needs credentials and Sjel's egress approval before personal data is sent.

### Accordion (vendored, detached)

[a-Fig/Accordion](https://github.com/a-Fig/Accordion), MIT, revision `8427145`, version 0.1.2
(2026-09-16). Full grant text in this Pack's [`LICENSE`](LICENSE).

A pi extension for context-window management: the Map, folding, and the conductors that decide
what folds between turns. Vendored rather than installed because the two strongest conductors —
thermocline and triptych — ship with the repository and are deliberately absent from the npm
package, so an npm install could never have the part the README calls the proof.

| | |
|---|---|
| Vendored | `pi-packages/accordion/` — `extension/`, `core/`, `conductors/`, and the one module `app/src/lib/live/registry.ts` the extension imports |
| Not copied | `app/` (the SvelteKit + Tauri desktop app) except that one module, `brand/` (24M), `docs/` (6.8M), `.github/` |
| Not committed | `node_modules/`, and `accordion.js`. Upstream does not commit `accordion.js` either — it is built at `prepack` for the npm tarball, which excludes `core/` and `app/` |
| Committed on purpose | `extension/dist/client/` — 2.3M, 54 files. This is the SvelteKit browser build the Map is served from, and it is the one generated artifact that IS committed, because it cannot be rebuilt from this tree: `build-client.mjs` copies `app/build`, and `app/` is not vendored. Upstream ships exactly this directory in its npm package (`files`) while never committing it to git |
| Local deltas | FOUR, all reversible without touching upstream's structure. (1) A root `package.json` beside the vendored trees carrying `pi.extensions: ["./extension/accordion.ts"]`, so pi loads the TypeScript source directly and the extension needs no build — upstream points its manifest at the built `accordion.js`, which is only necessary when `core/` and the shared app module are absent from the tarball. (2) In `session_start`, folding is armed from `ACCORDION_FOLDING_DEFAULT` (default **on**) and a catalog-validated conductor is attached from `ACCORDION_CONDUCTOR_DEFAULT` (default **triptych**), where upstream hardcodes `setFolding(false)`. (3) `session_before_compact` cancels pi's native compaction only while a conductor is attached, where upstream cancels it whenever folding is armed. (4) The same wrapper `package.json` also carries `scripts.test` and a pinned `devDependencies.vitest`, so the byte-identical upstream suite runs under the runner it was written for — see below |
| Owner | Axon, **detached**: no `.git`, no remote, no re-sync path. Upstream was read once, at the revision above. Divergence is expected, not drift |

Rebuilding the browser client, if it ever needs to change — it needs a checkout of upstream,
because `app/` is not vendored:

```bash
git clone --depth 1 https://github.com/a-Fig/Accordion /tmp/acc
cd /tmp/acc/app && npm install && npm run build   # vite build, no Rust
cd ../extension && node build-client.mjs          # → extension/dist/client
cp -R /tmp/acc/extension/dist/client "$SJEL_ROOT/Packs/harness/pi-packages/accordion/extension/dist/client"
```

The extension loads without this and folds context normally; only the Map view is missing, and
it answers `No browser build found` on the HTTP port until the directory exists. That failure is
worth knowing about, because it looks like a broken installation and is actually a missing build.

### Setup, and what is on by default

```bash
cd "$SJEL_ROOT/Packs/harness/pi-packages/accordion"                        && bun install --frozen-lockfile --ignore-scripts   # the test runner
cd "$SJEL_ROOT/Packs/harness/pi-packages/accordion/extension"               && bun install
cd "$SJEL_ROOT/Packs/harness/pi-packages/accordion/conductors/ws/triptych" && bun install
```

The first install exists only to run the vendored suite; the other two are runtime.

### Running the vendored tests

One command, from the package root:

```bash
cd "$SJEL_ROOT/Packs/harness/pi-packages/accordion" && bun run test
```

22 files, 459 tests, all green since 2026-09-17. This is not ceremony: deltas (2) and (3) are
behavioural patches to `extension/accordion.ts`, and this suite is the only thing that covers
them.

It runs under **vitest**, not `bun test`, and that is a deliberate split rather than an
oversight. The upstream tests import `vi` from vitest, and Bun's compatibility shim does not
implement `advanceTimersByTimeAsync` — so `bun test` reported seven failures inside vendored
code that this repository did not write and must not edit, since every upstream file here is
byte-identical on purpose. `bunfig.toml` therefore scopes `bun test` away from
`Packs/*/pi-packages/**`, and the `bun-tests` CI job runs this suite separately with the flags
the install policy requires. The exclusion is structural, so the next vendored package is
covered by it without anyone remembering to edit anything.

The second install is not decoration. Triptych's `web-tree-sitter` and `tree-sitter-wasms` are
declared as `requiredModules` in `core/conductor/registry.ts`, and a spawn conductor whose modules
do not resolve shows in the picker as unavailable with a remediation line. Without it, the
strongest conductor in the repository — the one whose README carries the benchmark — is simply
absent, which reads as a feature that does not exist rather than a dependency that was never
installed. Thermocline needs nothing: its runner is dependency-free by design and its optional
attention probe degrades to an age-based fallback.

**Folding is armed by default, and triptych is attached to do the folding.** Both are env-driven,
which is what makes this reversible without editing the vendored code:

| Variable | Default | Effect |
|---|---|---|
| `ACCORDION_FOLDING_DEFAULT` | `on` | Anything but `0`/`false`/`off`/`no` arms folding at `session_start`. Set it to `off` to go back to opt-in per session |
| `ACCORDION_CONDUCTOR_DEFAULT` | `triptych` | The conductor attached when folding is armed. Validated against the catalog, so a typo warns and attaches nothing rather than breaking session start; `none` is a real choice |

The arm and the attach go together, and that pairing is the point. A conductor proposes folds between
turns only while folding is armed (`serializeWire` is guarded on `foldingEnabled`), so a conductor
with folding off does nothing; and folding armed with **no** conductor is the combination to avoid,
because it is the one where pi's own protection has been switched off with nothing replacing it.

That is what the third local delta fixes. Upstream cancels `session_before_compact` — pi's native
compaction — whenever folding is armed, unconditionally, with no overflow escape valve. Its comment
says that is deliberate: once folding is armed, Accordion's budget is meant to be the only thing
between the session and overflow. That holds while a conductor is actually folding, and it is false
in the window where folding is armed and nothing has folded yet — no conductor attached yet, or one
that crashed or was detached. Folding on by default makes that window the common case, so the cancel
is now gated on `liveHost.activeMeta()`: armed but idle leaves pi's compaction intact.

Set `ACCORDION_FOLDING_DEFAULT=off` and the whole thing returns to upstream's opt-in behaviour,
including the unconditional suppression rule, with no code change.

Configuration is by environment variable rather than a config file, so it is per invocation:
`ACCORDION_HOME` (state directory, default `~/.accordion`), `ACCORDION_DOOR_PORT` (the stable
door), `ACCORDION_APP_PATH` / `ACCORDION_APP_FLAG` to point at a built desktop app, and the
`ACCORDION_PLAN_TIMEOUT_MS` and `ACCORDION_CONTROLLER_*_MS` timers.

### pi-web-access (vendored, detached)

[nicobailon/pi-web-access](https://github.com/nicobailon/pi-web-access), MIT, revision `192ac18`,
version 0.29.0 (2026-09-16). Full grant text in this Pack's [`LICENSE`](LICENSE).

The web search, page fetch, PDF extraction and video analysis every session uses, across roughly
twenty provider adapters. It arrived here as `npm:pi-web-access`, an opaque version we could not
patch when a provider changed shape.

| | |
|---|---|
| Vendored | `pi-packages/pi-web-access/` — the root `*.ts` sources, `package.json`, `tsconfig.json`, `LICENSE` |
| Not copied | `test/` (1.0M, 82 files), `pi-web-fetch-demo.mp4`, `banner.png`, `CHANGELOG.md`, `README.md`, `SECURITY.md`, and the upstream lockfile |
| Not committed | `node_modules/` (installed in place); the local `bun.lock` is committed |
| Local deltas | two, in `package.json`: (1) `"overrides": { "undici": "^8.10.2" }`, added 2026-09-29 for GHSA-3wwx-pv8p-q78v / CVE-2026-85024: undici's WebSocket client kills the whole process on a malformed permessage-deflate block, fixed in 8.10.2. This tree's own `undici` range had already resolved to 8.10.2, but the nested dev copy of `@earendil-works/pi-coding-agent/undici` sat at 8.9.0. `bun install` dropped the nested entry and `osv-scanner` reports no issues. (2) Host-provided `typebox` moved from runtime dependencies to a `"*"` peer to follow Pi 0.99's extension-loader contract. Upstream's manifest already points pi at `./index.ts`, so it still loads with no build |
| Owner | Axon, **detached**: no `.git`, no remote. The npm package was removed from `settings.json` when this landed, so exactly one web-access extension loads |

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
