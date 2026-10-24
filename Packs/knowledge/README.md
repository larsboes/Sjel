# knowledge

Knowledge work on the Obsidian vault. Deployed per harness where the vault is that harness's
surface: pi today, any other harness when a profile selects it. `teach` and question rounds in
`crystallize` degrade to plain text without the pi `quiz` extension — see the degraded paths
below.
The pack is the purpose; `obsidian` is the tool inside it. Eight skills collapsed to
three on 2026-08-22.

Vault-coupled, never vault-hardcoded: the vault root is read from the overlay
(`config/knowledge.toml`, `vault_root`) at runtime, so nothing personal is baked
into a skill.

## Skills

- **`obsidian`** — the single entry point for vault work. Resolves the vault,
  gates every write on `Projects/Soma/Vault-Contract.md`, routes to the syntax it
  needs, and carries the wiki layer (ingest · synthesize · lint) plus `recall`
  over past sessions. Syntax lives in `references/` — `markdown`, `bases`,
  `canvas`, `cli`, `wiki` — read at the point of need, never loaded up front.
- **`teach`** — one-to-one teaching at the measured edge of the student's
  understanding: probe with graded quizzes, plan a dependency path, teach one
  reasoning step at a time. Every write routes through `obsidian`. The graded
  quizzes come from the pack's `quiz` pi extension; without it (other harnesses,
  non-interactive runs) the skill falls back to numbered text questions.
- **`defuddle`** — clean markdown out of a web page. Not vault-coupled itself,
  but it is what the wiki's ingest path calls, and the Obsidian Web Clipper
  feeds the same `Clippings/` folder by another route.

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
always offered. Used by `teach` to find the edge and to check each step.

## Attribution

The syntax references started from [kepano/obsidian-skills](https://github.com/kepano/obsidian-skills)
(MIT, see `LICENSE`) and are edited freely from there — flattened into one skill, frontmatter
dropped so they cannot win a trigger, and rewritten against this vault as they get touched. They
are inspiration and starting material, not a pinned vendor copy: there is no re-sync path and no
obligation to track upstream. `references/wiki.md` is not upstream at all.

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
