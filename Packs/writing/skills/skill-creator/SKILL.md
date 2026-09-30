---
name: skill-creator
description: Authors, audits, evolves, and evaluates open-standard agent skills (SKILL.md with references/scripts/assets) and the Sjel Packs that bundle them, per the agentskills.io spec, current Claude Code skill features, and Claude-5-era authoring rules. Use when creating a new skill or pack, auditing or refactoring an existing one for progressive disclosure, metadata, or trigger quality, writing skill evals, de-prescribing a skill written for older models, or deciding which pack a skill belongs in. Do not use for prose or README docs (human-writing), non-agentic library code, or MCP-server implementation.
allowed-tools: Read, Write, Edit, Bash
---

# Skill Creator

Build a skill a zero-context executor — a mid-level engineer, or the weakest model that will
deploy it — can load and run correctly on the first try. Discoverable metadata, lean body,
deterministic scripts, evals that prove the skill earns its context cost. Ground truth only:
verify every path and command against disk before writing it down.

## Before you scaffold
Write a skill when the same instructions get pasted into chat repeatedly, or a CLAUDE.md
section has grown into a procedure rather than a fact — a skill's body loads only when
triggered, so it costs nothing until used. Don't write one for a single unrepeated task, or
for a fact with no procedure attached (that's a `references/` note in an existing skill, or
CLAUDE.md itself). Then place it: an existing pack whose domain fits > a new skill in a new
pack > a loose personal skill. Splitting vs merging skills inside a pack:
`references/patterns.md` §Pack routing.

## Variables — instantiate before writing
| var | fill with | constraint |
|-----|-----------|------------|
| `<name>` | the skill id | `^[a-z0-9]+(-[a-z0-9]+)*$`, ≤64, == directory basename, no `anthropic`/`claude` |
| `<desc>` | the trigger sentence | 3rd person, ≤1024, **what + "Use when …" + "Do not use for …"**, triggers only — never a workflow summary |
| `<when_to_use>` | optional extra triggers | combined with `<desc>` ≤1536 chars, key use case first |
| `<dir>` | `Packs/<pack>/skills/<name>` (pack) · `.claude/skills/<name>` (project) · `~/.claude/skills/<name>` (personal) | pack skills deploy via adapters |
| `<freedom>` | `low` \| `medium` \| `high` per step | fragile/safety → low · preferred-pattern → medium · open-ended → high |
| `<body_max>` | `500` | hard line ceiling for SKILL.md |
| `<ref_depth>` | `1` | references stay this many hops from SKILL.md |
| `<folders>` | `scripts references assets evals` | the subdirs YOU create, each one level deep |

A deployed skill may also arrive with nothing you did not put there. **A skill must be able to
run from its own directory** — no shared data directory, no paths into a sibling skill's tree.
The distinction that matters, and the one a pack-level `shared/` mechanism was tried and removed
for violating:

> **Run from itself. Point at a sibling by NAME for what it merely reads.** A run dependency that
is not inside the skill is a skill that is not deployable; a *named* read dependency degrades
gracefully, because the local text still says what to do whether or not the sibling is present.

So `slide-deck` naming `human-writing` is fine — and copying a 500-line catalog into
`slide-deck` to make it "self-contained" would be far worse than the dependency it removes.

When two skills must hold the *same* fact, and neither can own it for the other, that is a
gate rather than a file: see `tools/check-presentations-theme-contract.sh`, which fails if a
palette role one skill requires is not one the other's themes define. Verified invariants beat
shared files, because a shared file makes the source skill incomplete while a gate does not.

## Loading model — earn every token
| level | loaded | budget | holds |
|-------|--------|--------|-------|
| 1 metadata | always | ~100 tok/skill, shared listing budget | `<name>`+`<desc>`, the trigger |
| 2 body | on trigger, **stays for the session** | `<<body_max>` lines | workflow + the few rules the executor can't infer |
| 3 `<folders>` | on read/run | ~∞ (scripts: only stdout costs) | schemas, long docs, deterministic tools, eval cases |

Bulk lives in Level 3; SKILL.md is its table of contents. Full model, surface differences,
and the listing-budget failure mode: `references/architecture.md` +
`references/claude-code-extensions.md`.

## Procedure
```
- [ ] 1. <name>+<desc> pass validate_metadata.py
- [ ] 2. <dir>/<folders> scaffolded, name==dir
- [ ] 3. body written: imperative, <ref_depth>-deep refs, <<body_max> lines, current-model rules
- [ ] 4. every fragile step → a scripts/ CLI (args in, stdout/stderr out)
- [ ] 5. evals: baseline RED captured, evals/evals.json written, A/B delta positive
- [ ] 6. every references/checklist.md item passes
```

**1 · Metadata — the highest-leverage field.** `<desc>` is how a model picks this skill from
100+. State what it does, the concrete phrases a user would say, and the negative trigger +
which sibling to use instead. Never sketch the workflow in it — a summarized workflow becomes
a shortcut the model follows instead of reading the body. Gate it:
`python3 scripts/validate_metadata.py --name "<name>" --description "<desc>" --dir "<dir>"`
→ SUCCESS proceeds; any NAME/DESC/STYLE/DIR/DISCOVERY line → fix that field, re-run.
Discovery patterns + worked examples: `references/best-practices.md` §Discovery.
Restricting tools? `allowed-tools:`; the full frontmatter surface (`when_to_use`, `paths`,
`context: fork`, hooks, lifecycle): `references/claude-code-extensions.md`.

**2 · Scaffold.** `<dir>/{<folders>}`, each one level deep. No README/INSTALLATION/CHANGELOG
inside a skill — a pack's README carries the human-facing story instead.

**3 · Body.** Start from `assets/SKILL.template.md`. Third-person imperative. Pick
`<freedom>` per step: `low` → exact command + "do not modify"; `medium` → parameterized
script; `high` → goal only, trust the model. Over `<body_max>` lines or carrying a schema?
Move it to `references/` and command the read at point of need, never deeper than
`<ref_depth>`. Two rule sets a smart model still needs stated:
- **Current-model rules** — de-prescription (cut what a current model gets right unaided;
  keep safety gates, verified gotchas, tool contracts, output contracts), never instruct
  reasoning-echo (refusal risk), ground progress claims in long runs, damping blocks for
  bounded tasks: `references/fable-5-authoring.md`.
- **Ambiguity path** — if the task can arrive underspecified, gate it: one batched round of
  numbered questions with concrete options, escape-hatch defaults documented in output,
  answers persisted as workflow artifacts: `references/question-gates.md`.
Workflow shape (sequential, coordination, iterative, decision-tree, domain-intelligence) and
pack-level routing: `references/patterns.md`.

**4 · Scripts.** One fragile task (regex, parsing, multi-step, safety-critical) per CLI.
Takes args. **Solves, never punts** — handle the error in the script, don't fail to the
model. No magic constants. Specific stdout on success, specific stderr on failure so the
agent self-corrects. State intent in the body: "Run `x.py`" (execute) vs "See `x.py` for the
algorithm" (read).

**5 · Evals.** Before padding instructions: run the task with **no skill** in a fresh
subagent, capture the exact failure, write only enough to close it. Author
`evals/evals.json` (2–3 realistic cases, one edge/ambiguity case), run the with/without A/B
from clean contexts, grade assertions with quoted evidence, read the benchmark delta —
pass-rate gain vs token/time cost. Near-miss rule for trigger tests, grading discipline,
the full loop, and the `skill-creator@claude-plugins-official` automation:
`references/evaluation.md`.

**6 · Audit.** Every item in `references/checklist.md`, including the current-model section.
For an existing skill, the audit IS the entry point: run the checklist, then de-prescribe
against the baseline A/B.

## When NOT to use
Not for README/user docs, library code, or MCP-server code. For a domain runbook, don't
start from scratch — copy the closest sibling skill's shape from its pack.

## Bootstrapping a whole library
Different job: a project with *no* skill library, and the ask is the whole set (10+ skills).
Discover-then-parallel-author-then-adversarial-review workflow:
`references/bootstrap-library.md`.

## Provenance and maintenance
Supersedes `writing-skills`, which this skill carries forward. The promotion is done: the
skill moved out of the personal `meta` staging pack into `Packs/writing` on 2026-09-11, and
that pack now lists `writing-skills` in `retired_skills`. Nothing further is owed here.
Re-verify when sources drift — they are versioned and this is a snapshot:
- validator green on this skill: `python3 scripts/validate_metadata.py --file SKILL.md --dir "$(pwd)"`
- `references/best-practices.md`, `architecture.md`, `checklist.md`: Anthropic's
  [Agent Skills overview](https://platform.claude.com/docs/en/agents-and-tools/agent-skills/overview)
  + [authoring best practices](https://platform.claude.com/docs/en/agents-and-tools/agent-skills/best-practices)
  + the [agentskills.io](https://agentskills.io) spec — absorbed 2026-07-09, extended 2026-08-09.
- `references/claude-code-extensions.md`: [Extend Claude with skills](https://code.claude.com/docs/en/skills),
  re-snapshotted 2026-08-09.
- `references/evaluation.md`: [agentskills.io evaluating-skills](https://agentskills.io/skill-creation/evaluating-skills)
  (2026-08-09) + the baseline-first testing idea from obra/superpowers (MIT, idea-only, no text retained).
- `references/question-gates.md`: telekom/better-code `demo/one-modernizer` (MIT, idea-only), 2026-08-09.
- `references/fable-5-authoring.md`: [Prompting Claude Fable 5](https://platform.claude.com/docs/en/build-with-claude/prompt-engineering/prompting-claude-fable-5)
  + [prompting best practices](https://platform.claude.com/docs/en/build-with-claude/prompt-engineering/claude-prompting-best-practices)
  + Anthropic's Claude-5 context-engineering results, 2026-08-09.
- `references/patterns.md` + `bootstrap-library.md`: a third-party PDF skill-authoring guide
  plus a "bootstrap a whole skill library" mega-prompt, both encountered without provenance
  (unknown author and license — mined for ideas only, rewritten from scratch, no text
  retained); §Pack routing from the Sjel pack schema (`schemas/pack.toml.example`) and the
  2026-07 unslop merge history.
