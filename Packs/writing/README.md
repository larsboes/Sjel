# writing pack

What an agent produces for a human reader, and how the agent's own skills get authored.

Four skills, four subjects, one pack — merged 2026-09-17 because `profiles.toml` already
deployed them together in every profile that held any of them. A pack is a deployment unit, not
a category, so the test was co-deployment rather than resemblance. The skills keep their names,
so nothing changed about what loads.

- **`human-writing`**: prose that reads as one deliberate human voice instead of the model's
  default register. Drafts, edits, reviews, audits. Three layers. A CI-gateable linter with
  real structural checks (burstiness, lexical diversity, n-gram repetition, SVO monotony,
  over-correction, dialect drift). A catalog of the tells no regex catches, from uniform
  rhythm to sycophancy to saying nothing at length. And a guard against the over-corrected
  "trying not to sound like AI" register that just swaps one default for another. Facts,
  numbers, and citations stay invariant. Optional private voice profiles handle "make it sound
  like me".
- **`academic-writing`**: thesis chapters and conference/journal papers, across two genre
  profiles — empirical-CS/paper and DSR/qualitative-thesis — with an adversarial multi-skeptic
  review pass no third-party academic-writing tool checked during its build actually had. Four
  workflows: `Draft` (genre-aware skeletons and critique, never ghostwrites), `FlowCheck`
  (reverse-outlining, given-new flow, transition audit), `CriticReview` (five critique lenses
  plus the 3-skeptic pass), `Citations` (citation-key coverage, DOI/bib validation, paper
  resolution — Quarto and LaTeX/BibTeX both).
- **`unslop`**: strips the tells that make code and web UI read as AI-generated, and forces a
  deliberate project-specific choice instead of the model's default average. It has no preferred
  style of its own. Two domains sharing one method — **code** (swallowed errors, hallucinated
  APIs, chat artifacts, emoji, narrating comments, plus the structural tells a linter passes)
  and **UI** (default shadcn/Tailwind, AI-purple gradients, emoji-as-icons, the
  hero-plus-three-cards skeleton, and the newer cream-serif-sage "tasteful default"). Both
  scanners are stdlib-only Python, so there is no install step.
- **`skill-creator`**: meta-skill. Authors, audits, evolves and evaluates the skills and the
  Packs that bundle them. Scaffolds `SKILL.md` plus `references/`, `scripts/`, `assets/`,
  `evals/`; validates metadata (name, description, dir-match, reserved words, triggers, and
  the 1,536-char listing cap); enforces progressive disclosure; and runs baseline-first evals
  so a skill only grows to close a failure that was measured first. Supersedes the former
  `writing-skills`, which this pack retires.
- **`agents/`** (Claude Code only, optional): the same five critique lenses and three
  adversarial skeptics from `academic-writing`'s `CriticReview`, as native
  `.claude/agents/*.md` subagents instead of general-purpose dispatches with a prompt brief.
  The real advantage over the skill's own dispatch pattern is hard tool restriction (Read/Grep
  only — Write/Edit are actually blocked, not prompt-asked) and automatic delegation. Carried
  here when `academic-writing` merged in; deliberately not a `pack.toml` field, because a
  Claude-Code-only concept does not belong in the neutral manifest shape.

## Activate
```bash
"$SJEL_ROOT/tools/packs.sh" link writing
# → ~/.claude/skills/{human-writing,academic-writing,unslop,skill-creator}
# → ~/.claude/agents/writing/ (the 8 Claude-Code-native subagents)
"$SJEL_ROOT/tools/packs-codex" deploy writing  # → ~/.agents/skills/… (skills only; agents skipped)
"$SJEL_ROOT/tools/packs-pi" deploy writing     # → registered in ~/.pi/agent/settings.json
```

`retired_skills = ["writing-skills"]` in `pack.toml` is what removes the superseded copy an
adapter already installed. A plain `sync` would otherwise leave it loaded beside its successor.

## The boundary between the skills here

A pack is not a trigger unit, so co-locating these four does not merge their boundaries — the
`Do not use for…` clauses in each description are what route a request, and they still do.

`human-writing` and `unslop` are deliberately adjacent rather than overlapping:
`human-writing` owns **prose a human reads** and `unslop` owns **code and web UI**. Each names
the other as its negative trigger.

One duplication was known and is now resolved, and it is worth recording how, because the fix is a
rule rather than a mechanism: **`human-writing`'s `references/tells.md` is the only place a tell is
defined.** It has the data behind the ranking (a 600-post audited sample out of an 89,239-post pull)
and the linter that implements it. A genre that needs a different *fix* keeps that fix locally and
defines nothing: `academic-writing`'s `references/ai-cadence-tells.md` records the academic handling
of entries 1 (the em dash), 2 (the antithesis cadence) and 18 (the rule of three), and owns only the
four rhythm patterns no one else covers (semicolon splices, the Oxford-comma question, accretive
run-ons, false-hierarchy modifiers).

Those two files had **already drifted before the merge**: the catalog flags any em dash at all
("the rule is simply not to ship one"), while the academic file described only the interruption
shape. A reader applying the weaker rule had no way to know which was current. Both files now say
which one is authoritative, and the em-dash entry in each names the other.

This is the same shape `slide-deck` already used for `academic-writing`, `asd-ste100` and `unslop`:
**name the owning skill, never hardcode a cross-skill path.** A path would be harness-specific —
`~/.claude/skills/x` here, a registry entry in the repo there — so the owner is named and the
owning skill's own references are cited relative to itself. A pack-level `shared/` directory was
tried for exactly this and removed the same day (2026-09-17): it made the source skill incomplete —
the file lived at the Pack root and only the deployer materialized it — and it landed in every skill
of a Pack, so a 500-line catalog would have been copied into `skill-creator`. An invariant two skills
must share is a gate, not a file: `tools/check-presentations-theme-contract.sh` is the worked
example.

## Skill evaluation, and what it measured

`tools/skill-eval` is the executor for the method in `skill-creator/references/evaluation.md`.
`check` validates the suites and runs in `repo-gates`; `run` executes the with/without A/B and is
opt-in, because it costs model calls and CI cannot have a gate that flakes.

The first real run, 2026-09-17 on `skill-creator` case 2 (audit a fixture with planted defects):

| arm | assertions | cost | time |
|---|---|---|---|
| with skill | **6 / 6** | $0.80 | 89s |
| without skill | 4 / 6 | $1.55 | 204s |

The unaided audit missed one thing and invented another: it filed the reasoning-echo instruction
as an output-style directive in the wrong place rather than as the refusal risk it is, and it
reported that the description contained two forbidden forms "verbatim", quoting strings that are
not in the fixture. Both are recorded in the suite's `baseline`.

Two things follow from that, and they are the point of having the tool. The skill measurably
changed what the model did, and it did so while being *cheaper and faster* — the procedure is
a substitute for working it out, not an addition to it. And the delta is two assertions on one
case, which is a demonstration that the loop works, not a benchmark of the skill.

32 of 34 skills still have no suite. That is the outstanding work, and it is now measurable
rather than a matter of opinion.

## Attribution

Full grant text for everything vendored here lives in [`LICENSE`](LICENSE). Verdicts and
licences of record are in `upstreams.toml`, per subject, below.

### human-writing

Merged 2026-07-25 from three upstreams. It supersedes the former `unslop-text` skill, moved
here from `Packs/unslop` with `git mv` so its history survives. Full per-source detail sits in
that skill's `references/maintenance.md` (§ Provenance) and in `upstreams.toml`.

- [stephenoffer/human-voice](https://github.com/stephenoffer/human-voice) (`[human-voice]`,
  MIT, pinned `9bcba2f`) is the substrate. Its linter is vendored wholesale
  (`scripts/human_voice_linter/`, `detect_ai_prose.py`, `ai_prose_patterns.json`), plus four
  reference files verbatim under an attribution header. Two local deltas, both documented in
  the skill's `references/maintenance.md` (§ Local deltas against upstream): the em-dash check now fires on presence rather than upstream's
  `count >= 2` density gate (zero false positives measured across the bundled human and
  ESL-formal corpora), a handful of additive pattern entries, and one corrected user-facing
  string — `patterns.py`'s pattern-file-not-found message named `references/ai-tells.md`, which
  has never existed here, and now names `references/tells.md` (2026-09-17).
- [ryanthedev/oberskills](https://github.com/ryanthedev/oberskills) (`[oberskills]`, pinned
  `5050537`) supplied the architecture: mode router, reader-job taxonomy, voice profiles,
  review protocol. **MIT is declared in `.claude-plugin/plugin.json` only, with no LICENSE file
  at the repo root.** So everything taken is paraphrased and rewritten rather than copied, and
  both derived reference files say so explicitly. Revisit if upstream adds a real LICENSE.
- [JCarterJohnson/vibecoded-design-tells](https://github.com/JCarterJohnson/vibecoded-design-tells)
  (`[vibecoded-design-tells]`, MIT, pinned `f7c4aef`) supplied the doctrine and the ranked tell
  catalog: cited-vs-matched weighting, the density model, the over-corrected-register framing.
  `references/tells.md` and `references/writing-with-intent.md` are still its text.

[hardikpandya/stop-slop](https://github.com/hardikpandya/stop-slop) was evaluated and declined
twice, most recently during this merge. Its distinctive contributions are already pattern
categories in the vendored linter, and its absolutist framing ("kill all adverbs", "no em
dashes ever") manufactures exactly the uniform signature the skill exists to prevent. See
`upstreams.toml [stop-slop]`.

### academic-writing

Built from a source-material sweep of three third-party academic-writing tools plus a fourth
author's (Peng Sida, 彭思达) publicly shared paper-writing notes, all cherry-picked and rewritten
in our own words — no verbatim third-party text retained except
`references/citation-workflows.md`, which its own header records as ported near-verbatim (MIT
source), and no code vendored except the scripts named below.

Licences vary by source, not "MIT throughout" — see each bullet and the linked `upstreams.toml`
entry (`[academic-researcher]`, `[academic-writing-agents]`, `[research-paper-writing-skills]`,
`[pengsida-research-notes]`); the "unknown license" placeholders from the pack's initial
2026-07-11 build were resolved 2026-07-14.

- Citation-workflow content (`references/citation-workflows.md`'s bibliography/claim-evidence-map
  sections) and the three citation scripts (`scripts/resolve-papers.js`, `validate-bib.js`,
  `check-citations.js`) are adapted from
  [SiluPanda/academic-researcher](https://github.com/SiluPanda/academic-researcher) (MIT, pinned
  `e75d70d`) — `check-citations.js` was generalized off a hardcoded path and given Quarto `@citekey`
  support beyond the original's LaTeX-only `\cite{}` handling. See `upstreams.toml
  [academic-researcher]` (verdict `overlay` — real code adapted, not just ideas; retroactive entry,
  still needs a `tools/audit` pass against these three files, and an `upstreams.toml`
  verdict written by hand — no script checks the manifest since 2026-08-28).
- The condensed house style (`references/house-style.md`, sections A/B/D/F) is a rewrite of
  [andrehuang/academic-writing-agents](https://github.com/andrehuang/academic-writing-agents) (MIT,
  a Claude Code plugin) — the five critique lenses in `references/critic-briefs.md` and the matching
  `agents/academic-writing-*.md` files (technical/logic/consistency/bibliography/layout) are also
  adapted from that plugin's 10-agent roster, ported as prompt content rather than the original's
  static-persona files, with its hardcoded author-specific path removed. See `upstreams.toml
  [academic-writing-agents]` (verdict `quarry` — ideas/structure mined, no code retained).
- Flow-diagnostic technique (`references/flow-diagnostics.md` §1–4) is distilled from a university
  writing-center handout encountered without clear authorship attribution — ideas only, no verbatim
  text retained, and still no source to record in `upstreams.toml`.
- `references/genre-empirical-cs.md` condenses Peng Sida's (彭思达) publicly shared paper-writing
  notes — [GitHub](https://github.com/pengsida/learning_research) /
  [Notion](https://pengsida.notion.site/c1a22465a0fa4b15a12985223916048e), no license declared — via
  its Claude-skill repackaging,
  [Master-cai/Research-Paper-Writing-Skills](https://github.com/Master-cai/Research-Paper-Writing-Skills)
  (MIT). Rewritten, not copied, from either. See `upstreams.toml [pengsida-research-notes]` and
  `[research-paper-writing-skills]` (both verdict `quarry`).
- **Not from any external source** — built for this pack specifically: `references/genre-dsr-
  qualitative.md` (no analyzed source covered DSR/qualitative-thesis writing at all), §5 of
  `references/flow-diagnostics.md` (given-new sentence flow + rhythm variance),
  `references/ai-cadence-tells.md` and `scripts/scan-ai-tells.js` (generalized from patterns
  battle-tested in real thesis-editing sessions, not derived from the third-party corpus), and
  `references/adversarial-redteam.md` plus the three skeptic agents (no third-party tool checked had a
  genuinely adversarial pass — every one of them was structurally biased toward finding fixable issues
  rather than arguing for outright rejection).

### unslop

The skill (`SKILL.md`, `references/`, `scripts/`) is adapted near-verbatim from
[JCarterJohnson/vibecoded-design-tells](https://github.com/JCarterJohnson/vibecoded-design-tells),
pinned `f7c4aef` (2026-06-23), MIT — the same pinned commit and the same notice as
`human-writing`'s third upstream above, which is why the two share one section of
[`LICENSE`](LICENSE) rather than carrying two copies of one grant. Content and instructions are
unchanged except as listed here; each `description` had a small number of second-person clauses
reworded (code half: "for you" / "points you at"; UI half: "hand you taste" / "are still yours" /
"you become next year's slop") to clear the third-person discovery convention `skill-creator`
enforces. The 2026-07-28 merge rewrote the shared body prose; `references/` remains
byte-for-byte the pinned commit's, renamed only (`tells.md` → `tells-code.md` / `tells-ui.md`),
and `scripts/` differs by six corrected user-facing strings (2026-09-17): the rename left both
scanners telling the user to consult `references/tells.md`, a path that no longer exists, so each
now names the half it means — `tells-code.md` or `tells-ui.md`.
See the SKILL.md Provenance section for detail, and `upstreams.toml
[vibecoded-design-tells]` for the verdict. The repo's own MIT note applies here too: the
licence covers the code/docs vendored into `skills/`; the raw Reddit harvest (`corpus.jsonl`,
charts, CSVs) was never copied into Sjel, so its separate data terms apply to nothing here.

### skill-creator

Promoted 2026-09-11 out of the personal `meta` staging pack, which held it as an upgraded
public pack awaiting promotion. It carries `writing-skills` forward and reconciles several
sources into one local convention, all distilled into `references/` in our own words. No
verbatim third-party text retained, no code vendored, MIT throughout.

What it adds over the distilled base it inherited, each grounded in the 2026-08-08 source
review: **current-model authoring rules** (`references/fable-5-authoring.md` —
de-prescription as the default direction, the reasoning-echo refusal risk, grounded progress
claims, damping blocks, the cross-model floor for a pack skill that deploys beyond one
harness); **question gates** (`references/question-gates.md` — asking the user as pipeline
machinery, batched concrete-option grammar, escape-hatch defaults that get documented rather
than silently guessed); and **eval-first discipline** (`references/evaluation.md` — baseline
RED runs before writing instructions, the `evals/evals.json` case format, with/without A/B
benchmarking, the near-miss rule for trigger tests). The skill ships its own eval suite,
fixture included.

- Anthropic's [skill best-practices](https://platform.claude.com/docs/en/agents-and-tools/agent-skills/best-practices) plus [Skills overview](https://platform.claude.com/docs/en/agents-and-tools/agent-skills/overview), the [agentskills.io](https://agentskills.io) `skill-creator` spec, and the dense variable-slotted "Fable" skill style. That was the original three-source synthesis.
- Anthropic's [Extend Claude with skills](https://code.claude.com/docs/en/skills) doc, covering the Claude-Code-specific frontmatter and runtime (`context: fork`, dynamic context injection, skill stacking, `skillOverrides`, the description-listing budget), in `references/claude-code-extensions.md`.
- Anthropic's [new rules of context engineering for Claude 5](https://claude.com/blog/the-new-rules-of-context-engineering-for-claude-5-generation-models) (2026): over 80% of Claude Code's own system prompt removed without measurable loss; prescriptive rules replaced by principles, detail moved behind progressive disclosure. Read 2026-07-28; it is why `validate_metadata.py` now enforces the Level-2 body budget this pack had documented but never checked.
- A third-party PDF skill-authoring guide and a "bootstrap a whole skill library" mega-prompt (both unknown author and license, encountered without provenance), mined for *ideas only*: the three-tier eval framework, the five workflow-shape patterns, the discover→parallel-author→adversarial-review pattern. Rewritten from scratch, no text retained. `references/evaluation.md`, `references/patterns.md`, `references/bootstrap-library.md`.

## Considered and declined

Evaluated while auditing this pack, none adopted. Recorded under
`CONTRIBUTING.md#decisions-live-with-their-owner` rather than deleted, so the same sources
do not get re-evaluated from scratch.

- **[ASD-STE100 Simplified Technical English](https://asd-ste100.org)** (ASD, Brussels;
  copyrighted, free official copy on request — never paste it in full). A controlled natural
  language from 1986 aircraft maintenance documentation: one word one meaning, active voice,
  max 20 words per instruction, no semicolons or contractions. Declined as a *skill here*,
  because it strips voice on purpose and `human-writing` exists to specify voice, not remove
  it. Its natural home in this pack is an eleventh `references/registers.md` profile, stricter
  than `technical`, for genuinely voiceless operational text (error messages, runbooks, CLI
  help) — not built yet. **The decline does not cover its other reader.** A skill for strings a
  *machine* must parse, where stripping voice is the point rather than a cost, exists in
  `Packs/cognitive-load` as `asd-ste100` — promoted into Sjel 2026-09-11 and packed beside
  `attention-control` on 2026-09-17, since every language rule it once duplicated there now
  lives only in it. That scope is disjoint from this pack's, so the two do not compete.
- **[woosal1337/blog `ep01-the-cure-for-ai-slop`](https://github.com/woosal1337/blog/tree/main/videos/ep01-the-cure-for-ai-slop)**
  — an STE-condensed skill plus `ste-lint.py`, benchmarked at −74% "violations per 100 words" on
  Claude and −50% on GPT-5.5 against a plain baseline. Declined as a dependency: the linter counts
  the STE rules themselves (sentences over 20 words, semicolons, contractions, passive voice, its
  own banned-word list), so a skill that states those rules scores well by construction. What
  survives the circularity is the *comparison* — a banned-words list moved Claude only 3% against
  STE's 74%, so a writing system beats a word list. That conclusion is already this pack's design.
- **[NVIDIA/skills](https://github.com/NVIDIA/skills)** (`upstreams.toml [nvidia-skills]`, dual
  CC-BY-4.0/Apache-2.0): roughly 230 published skills, mostly GPU and datacenter ML infra with no
  direct overlap here. Declined as a dependency; kept in `upstreams.toml` as the largest real-world
  corpus of skill *structure* available to compare against when scoping a new one.

## Further reading
- Anthropic's live docs. Check these directly when this pack's distillation might be stale (they're versioned, this is a 2026-07-10 snapshot): [Agent Skills overview](https://platform.claude.com/docs/en/agents-and-tools/agent-skills/overview) · [authoring best practices](https://platform.claude.com/docs/en/agents-and-tools/agent-skills/best-practices) · [Extend Claude with skills](https://code.claude.com/docs/en/skills) · [agentskills.io spec](https://agentskills.io).
- [Claude Code subagents](https://code.claude.com/docs/en/sub-agents) — the `agents/` convention above follows this spec directly; re-check it if these agent files ever look stale against a Claude Code release.
- `references/tells-code.md` and `references/tells-ui.md` (in `skills/unslop`) carry the full ranked-tell catalogs with cited-vs-matched data shares and real quotes.
