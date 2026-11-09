# knowledge

Knowledge work on the Obsidian vault, which holds learning notes only since 2026-11-08: subject
knowledge and what `teach` and `practice` write. Operational records live in Sjel. Deployed per harness where the vault is that harness's
surface: pi today, any other harness when a profile selects it. The pedagogy (`teach` and
`practice`) moved to [`Packs/learning`](../learning/README.md) on 2026-11-03 and still writes
through `obsidian` and asks through `quiz` here. Question rounds in `crystallize` degrade to
plain text without the pi `quiz` extension.
The pack is the purpose; `obsidian` is the tool inside it. Eight skills collapsed to
three on 2026-08-22.

Vault-coupled, never vault-hardcoded: the private overlay's `config/knowledge.toml` selects a
vault profile and may provide `vault_root` for filesystem-only helpers. It may also name a
vault-writing contract with `contract_path`, a vault-relative note path that differs per machine.
The contract is optional; without it, writes follow the vault's existing conventions.
`local_files = true` selects local vault mode: agents read and edit files under `vault_root` with
local file tools instead of the governed Pi tools. Local access bypasses the access policy below,
so set it only for a vault with no confidential notes.

## Skills

- **`obsidian`** — the single entry point for the knowledge-only vault. Resolves
  the active vault, gates writes on its contract, routes to the syntax it needs,
  and carries knowledge ingest, synthesis, lint, and `recall`. Sjel owns
  operational records and unsorted source material. Syntax lives in
  `references/` — `markdown`, `bases`, `canvas`, `cli`, `wiki` — read at the
  point of need, never loaded up front.
- **`defuddle`** — clean markdown out of a web page for review and knowledge
  extraction. It does not write to the vault or create a clipping inbox.
- **`web-research`** selects web and AI Studio routes for public information. Software
  investigation and repo research reports belong to the upstream `research` skill, when installed.

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
confidence (`←→`, unset by default); after the reveal, `n` adds a reaction. Used by `teach`
(`Packs/learning`) to find the edge and to check each step.

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
edits require confirmation, while version preconditions reject concurrent changes. When these tools are unavailable, the Obsidian skill permits a read-only filesystem fallback
only for an exact local note path supplied by the user and fully admitted as C0 under the active
model's access policy, with effective standard agent access and no stricter session clearance or
egress requirement. Unknown, restricted, C1, C2, and C3 notes remain unavailable; the fallback does not
permit vault discovery, edits, or bypassing an available tool's refusal. The private overlay
supplies
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

`teach` and `practice` carry their own attribution in [`Packs/learning`](../learning/README.md#attribution).

## Interactive figures in notes

The locally maintained `plugins/interactive/` plugin renders `interactive` fenced blocks as
native HTML/CSS/JavaScript inside Obsidian notes. `file:`, `height:` and `title:` select an
asset and presentation. `html-embed` uses the separate iframe renderer.

Native controllers scope DOM lookups to their embed root and register a one-shot
`interactive-dispose` listener there to release animation frames, timers and global listeners.
The renderer sends that event before replacing content, when a note block is removed and when
the plugin unloads. Asset reads that finish after teardown or a newer render are ignored.
[`plugins/interactive/README.md`](plugins/interactive/README.md) owns the embedding guidance for figures.

Build with `tools/obsidian-plugin interactive build`, or build and install into the active vault
with `tools/obsidian-plugin interactive install`. The install keeps the previous bundle in
`.obsidian/interactive-plugin.prev/`. Reload the plugin in Obsidian to activate a build.
`tools/obsidian-plugin.test.ts` fails when the built or the installed copy is older than `src/`.
