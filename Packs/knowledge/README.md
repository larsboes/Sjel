# knowledge

Knowledge work on the Obsidian vault. Deployed per harness where the vault is that harness's
surface: pi today, any other harness when a profile selects it. `teach` and question rounds in
`crystallize` degrade to plain text without the pi `quiz` extension — see the degraded paths
below.
The pack is the purpose; `obsidian` is the tool inside it. Eight skills collapsed to
three on 2026-08-22.

Vault-coupled, never vault-hardcoded: the private overlay's `config/knowledge.toml` selects a
vault profile and may provide `vault_root` for filesystem-only helpers. It may also name a
vault-writing contract with `contract_path`, a vault-relative note path that differs per machine.
The contract is optional; without it, writes follow the vault's existing conventions.

## Skills

- **`obsidian`** — the single entry point for the knowledge-only vault. Resolves
  the active vault, gates writes on its contract, routes to the syntax it needs,
  and carries knowledge ingest, synthesis, lint, and `recall`. Sjel owns
  operational records and unsorted source material. Syntax lives in
  `references/` — `markdown`, `bases`, `canvas`, `cli`, `wiki` — read at the
  point of need, never loaded up front.
- **`teach`** — one-to-one teaching at the measured edge of the student's
  understanding: probe with graded quizzes, plan a dependency path, teach one
  reasoning step at a time. Every write routes through `obsidian`. The graded
  quizzes come from the pack's `quiz` pi extension; without it (other harnesses,
  non-interactive runs) the skill falls back to numbered text questions. Each answer carries
  the learner's thinking and a confidence, each step opens with a prediction, and a session
  closes by extracting claim notes (`type: claim`) with their `## Because` arguments.
- **`practice`** — practice on claims already taught: interleaved problems that never name
  their target, one hint at a time, and a retest of every miss. Reads claim notes, writes only
  the Learning Log line.
- **`defuddle`** — clean markdown out of a web page for review and knowledge
  extraction. It does not write to the vault or create a clipping inbox.

## Why one skill instead of eight

`obsidian-markdown`, `obsidian-bases` and `json-canvas` were pure syntax
references — no procedure, no scripts, no decisions — and each carried a
description broad enough to fire on almost any vault sentence. When one of them
won the trigger instead of `obsidian-ops`, the write happened without the vault
contract ever loading, so the gate that enforces typing and the no-doubling law
was silently skipped. As `references/` they carry the same content at zero
trigger cost, behind the one skill that reads the contract first.

`llm-wiki` was folded and rewritten rather than moved: it prescribed a parallel
`Knowledge/<domain>/llm-wiki/` tree with its own `index.md`, `log.md`,
`sources/`, `entities/` and `concepts/` folders, which contradicts the contract's
placement table and no-doubling law on every point. The vault *is* the wiki.

## Extensions

`extensions/questions.ts` registers the two tools that put a question in front of the
human instead of into the transcript. Both refuse to hang in a non-interactive run:
they return an error the caller degrades from, rather than waiting forever.

- **`ask`** — a batched decision round. Each question carries 2-9 mutually
exclusive options with a one-line consequence each, an optional preview block
shown for the highlighted option, and your recommendation marked ★ and focused,
so Enter accepts it. The user picks one option and can attach a free-text note to
that decision; `s` skips a question and leaves it open. The result reports every
decision with its note and says whether it overruled your recommendation — so a
caller can tell an informed answer from an accepted default, and can carry the
user's own reasoning into the document it writes. Question rounds in
`crystallize` are this tool.
- **`quiz`** — graded multiple-choice, one question at a time, graded on
selection with the correct answer and explanation revealed, and "I don't know"
always offered. Before answering, the learner can type their thinking (`t`) and set a
confidence (`←→`, unset by default); after the reveal, `n` adds a reaction. Used by `teach` to
find the edge and to check each step. In Claude Code, `teach` uses `AskUserQuestion` instead:
the item plus a confidence question per round, the thinking in the option note.

### `obsidian-access`

`pi-packages/obsidian-access` registers Pi-native tools over the installed
`obsidian-local-rest-api` plugin. The plugin remains the vault transport; this package owns the
model boundary. It checks the active `provider/model`, combines Sjel's C0-C3 class with the
optional `agent_access: internal|confidential` frontmatter restriction, applies private overlay
path rules to non-Markdown objects and folder boundaries, and filters collection results before a
filename or search snippet enters model context. Existing `sensitivity: internal` or `restricted`
notes retain their older access boundary. C3 is always refused.

The read surface covers whole or targeted note reads, validated Base reads, outlines, listings,
search, active-file state, backlinks, tags, and access explanations. Draft validators check strict
Markdown frontmatter or Base YAML without writing. Controlled Markdown mutations provide create, append,
whole-note write, targeted patch, copy, move, and delete; destructive operations and policy-field
edits require confirmation, while version preconditions reject concurrent changes. The private
overlay supplies
`config/obsidian-access.toml`; an absent policy admits C0 only. C0–C3 folder decisions use the
local `vault classify` command and therefore the canonical Rust implementation. If that binary is
unavailable, undeclared notes use C2 as a conservative fallback. The package does not configure
the upstream plugin's unrestricted MCP endpoint.

Install its local dependencies before selecting the Pack in Pi:

```sh
cargo build --release -p vault --bin vault
cd "$SJEL_ROOT/Packs/knowledge/pi-packages/obsidian-access"
bun install --frozen-lockfile --ignore-scripts
```

## Attribution

The syntax references started from [kepano/obsidian-skills](https://github.com/kepano/obsidian-skills)
(MIT, see `LICENSE`) and are edited freely from there — flattened into one skill, frontmatter
dropped so they cannot win a trigger, and rewritten against this vault as they get touched. They
are inspiration and starting material, not a pinned vendor copy: there is no re-sync path and no
obligation to track upstream. `references/wiki.md` is not upstream at all.

`teach` started from [amosblomqvist/learn](https://github.com/amosblomqvist/learn). Its
prediction step, claim extraction and the `practice` skill paraphrase ideas from
[amosblomqvist/learnharness](https://github.com/amosblomqvist/learnharness) (AGPL-3.0). No text
is copied, and the verdict is in `upstreams.toml` (`[learnharness]`).

## Interactive figures in notes

The locally maintained `plugins/interactive/` plugin renders `interactive` fenced blocks as
native HTML/CSS/JavaScript inside Obsidian notes. `file:`, `height:` and `title:` select an
asset and presentation. `html-embed` uses the separate iframe renderer.

Native controllers scope DOM lookups to their embed root and register a one-shot
`interactive-dispose` listener there to release animation frames, timers and global listeners.
The renderer sends that event before replacing content, when a note block is removed and when
the plugin unloads. Asset reads that finish after teardown or a newer render are ignored.
[`plugins/interactive/README.md`](plugins/interactive/README.md) owns the embedding guidance for figures.

Build with `bun build Packs/knowledge/plugins/interactive/src/main.ts --target browser
--format cjs --external obsidian --outfile <staging>/main.js`. The installed plugin lives in the
active vault's `.obsidian/plugins/interactive/`; resolve the vault through the Obsidian skill,
retain the previous bundle before replacing it, and reload the plugin to activate the build.
