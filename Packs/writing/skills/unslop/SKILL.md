---
name: unslop
description: >-
  Strips the tells that make source code and web UI read as AI-generated and forces a
  deliberate, project-specific choice instead of the model's default average. It writes
  neither the code nor the design, and has no house style. Code: chat artifacts, placeholder
  comments, emoji, swallowed errors, generic names like process_data, plus the structural
  tells a linter passes (tutorial boilerplate, hallucinated APIs, over-engineering, ignoring
  the surrounding repo). UI: default shadcn/Tailwind, AI-purple gradients, emoji-as-icons,
  the hero-plus-three-cards skeleton, and the newer cream-serif-sage 'tasteful default'.
  Use whenever writing, generating, reviewing,
  refactoring, or auditing code or a web interface, especially when it 'looks AI-generated',
  'reads like a tutorial', is 'too generic' or 'vibe-coded'. Trigger even if the request
  never says AI tell. Do not use for prose a human reads (use human-writing), thesis text
  (use academic-writing), or agent skills (use skill-creator).
allowed-tools: Read, Edit, Bash
---

# unslop

Read this first, because the most common misunderstanding sinks the whole thing.

**This skill writes nothing and has no house style.** It does two narrow things. It removes
the specific cues that make code or an interface read as machine-made, and where the output
reads as AI because nobody ever made a choice, it forces a deliberate one. Correctness,
taste, and architecture stay with the person building. A guardrail is not an engineer and it
is not a designer.

## The trap to avoid (this is the whole point)

Every anti-slop effort fails the same way: it replaces one default with another. In UI the
2024 tell was the purple-to-blue gradient on a dark hero; the 2026 tell is warm cream, a
serif display face, and a sage accent, which is the current Claude house look. In code it is
the model told to "write clean code" over-correcting into defensive checks for impossible
cases, a type on every local, a comment on every block, an abstraction with one caller.
Trying to look senior is itself a tell.

So this skill never prescribes a palette, a font, a layout, or a pattern. Prescribing one is
how a skill becomes next year's slop. **A tell is an unspecified default, not a banned
thing.** If the user genuinely wants purple, or a broad catch at a process boundary, that is
a decision and the skill leaves it alone. The only universal rule: make a deliberate choice
and be able to say why, which is the one thing a default never is.

## The two domains

| | code | UI |
|---|---|---|
| Scanner | `scripts/unslop_code_scan.py` | `scripts/devibe_scan.py` |
| Catalog | [references/tells-code.md](references/tells-code.md) | [references/tells-ui.md](references/tells-ui.md) |
| Build method | [references/fitting-the-codebase.md](references/fitting-the-codebase.md) | [references/choosing-a-look.md](references/choosing-a-look.md) |
| What only this domain has | It executes. Compile and resolve imports before reading anything | Layout coherence, spacing, and overflow are invisible to the scanner |

A styled component is both. Run both scanners and read both catalogs.

## Mode 1: Build (the important one)

Most "looks AI-written" outcomes are a specification problem, not a style problem. An
unspecified prompt gets the average of public code, and the average is the tutorial. Before
generating, establish the brief. Pull it from the user, or state what is being assumed and
why, then proceed. Never generate against a blank slate.

The single highest-value input, in both domains, is **the thing it must resemble**: for code,
the files it will sit next to and the conventions they already follow; for UI, one real site,
brand, or named direction. The most repeated fix in the entire dataset was "make it follow
the existing code instead of guessing the average." Everything else is secondary to that.

The two build-method references carry the rest, written as process rather than prescription
so they do not become the next default: what to establish for code, and the color, type, and
layout decisions for UI. When the user gives no brief, do not produce one median result.
Produce a deliberate one and say what was chosen, or offer two or three genuinely distinct
directions.

## Mode 2: Audit (the guardrail)

Order matters. The scanners are regexes and are blind to the tells that rank highest.

**1. For code, run what only code allows.** This catches the bug-class structural tells,
hallucinated APIs above all, which no regex sees. Python: `python -m py_compile`, `ruff
check`, `mypy`, and import the module so a missing dependency fails loudly. JS/TS: `tsc
--noEmit`, `eslint`, `node --check`, with a real install. Go: `go build ./... && go vet
./...`. Rust: `cargo check && cargo clippy`. If it will not build, or a call resolves to
nothing, that is the loudest tell in the data, found before reading a line.

**2. Run the scanner.** Both take the same flags:

```bash
python3 scripts/unslop_code_scan.py <path>              # full report + slop score
python3 scripts/devibe_scan.py <path> --severity high   # only the strongest signals
python3 scripts/unslop_code_scan.py <path> --json       # machine-readable, for CI
```

Exit code is the high-severity count, so CI can gate on it. The code scanner tags each
finding with a **class**: *bug* (swallowed exception, hallucinated call, `// rest of your
code` stub) or *cosmetic* (emoji, chat artifacts, narrating comments, generic names).
Severity is how loudly it reads as AI; class is whether it is broken. **Fix every bug-class
finding regardless of severity**, and never spend an audit polishing cosmetics while a
swallowed error ships.

**3. Read the diff for what neither step sees:** tutorial shape, over-engineering, repo
mismatch, layout coherence, spacing, overflow. These are the substance tells, and they are
the loudest in the data. The code ranking is blunt about it: boilerplate 18.6%, hallucinated
APIs 11.2%, over-engineering 7.8%, against emoji 3.9% and verbose names 0.4%. Surface cleanup
is the easy 40% of the job, not the job.

**Respecting intentional choices.** A line containing `unslop-ignore` is skipped by both
scanners. Use it when a flagged construct is a real decision, so the audit stays trustworthy.
The sibling `human-writing` linter has no inline ignore directives. Keep scanner configuration
outside the prose files.

## Fixing well

The anchor is *correct, and fits this project*. Do not "fix" `process_data()` by renaming it
`processDataFunction()`, or `bg-purple-600` by swapping in `bg-emerald-700`. Both are just a
different default. Name it the way the rest of the repo names things; apply the project's
actual color, or ask what it should be. **A fix that swaps the model's default for an
invented one is not a fix.** Code has far less aesthetic latitude than UI here: there is
rarely a tasteful choice to make, only "is it correct, and does it match what is already
here."

## What this deliberately does not flag

Grounded in the data, not vibes, because over-flagging trains people to ignore the tool.
Code: left-in debug logging (rejected outright, precision ~0%), reinventing the wheel (mostly
misattributed), over-defensive validation (half the complaints were about the opposite). UI:
mesh/aurora/blob backgrounds (a keyword artifact), bento grids and glassmorphism (low and
contested), dark mode itself (only unprompted glow is a tell), shadcn and Tailwind themselves
(only their untouched defaults are).

## Reporting an audit

Lead with the verdict and the single highest-impact change. Then findings by priority with
`file:line` and the fix. Close with the score, the top three changes, and a reminder that the
structural tells still need eyes. The goal is code that looks like it belongs in this project
and an interface that looks like a person made a decision, which is the one thing the
scanners cannot check.

## Provenance and maintenance

Both halves adapted near-verbatim from `unslop-ai-code` and `unslop-ui` in
[JCarterJohnson/vibecoded-design-tells](https://github.com/JCarterJohnson/vibecoded-design-tells),
pinned `f7c4aef` (2026-06-23), MIT. Code is grounded in a Reddit analysis of 11,906 posts and
11,306 comments across 55 AI, coding, and SaaS subreddits; UI in a 47-subreddit, 3.2M-post
analysis. Merged into one skill 2026-07-28 (history preserved via `git mv`): the two shared an
identical method and split an ambiguous boundary, since a styled component is both. Upstream
second-person clauses were reworded for the third-person discovery convention; no other change.
Full attribution in `Packs/writing/README.md`, verdict in `upstreams.toml [vibecoded-design-tells]`.

Both scanners are stdlib-only (`os re sys json argparse`) — no install step, any Python 3.
Re-verify on drift:

```bash
python3 scripts/unslop_code_scan.py --help && python3 scripts/devibe_scan.py --help
```

Metadata and body-budget validation is **not** run from here. It used to be, as a command
reaching into `skill-creator`'s `scripts/validate_metadata.py` by repo path — a dependency on a
sibling skill's file, which resolves in the Sjel checkout and nowhere else, so a skill copied
into a harness carried a verification step that could not run.
`tools/check-skill-metadata.sh` runs that same validator over every authored SKILL.md on every
push, which is strictly stronger than checking one skill on demand.
