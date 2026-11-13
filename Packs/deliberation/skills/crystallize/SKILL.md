---
name: crystallize
description: Turn unstructured input — a raw brainstorm, a pile of notes, scattered ideas, a half-written PRD — into one precise document where every claim is grounded in the actual repo or vault and every decision is dated with its reasoning. Drives the process through batched multiple-choice questions rather than open-ended prompts, because the user has the judgment and the agent has the ability to measure. Use when asked to think something through, make it precise, turn notes into a spec, PRD or plan, when handed a rambling document with no structure, or when a document and the system it describes have drifted apart and the question is which one is lying. Do not use for producing candidate options where none exist yet (use diverge), for choosing between options already on the table (use council), for attacking one proposal (use red-team), or for investigating a failure (use root-cause).
license: MIT
metadata:
  tags: "PRD, brainstorming, decision records, requirements, documentation, interviewing"
  category: "thinking"
---

# Crystallize

Someone hands you a mess and wants a decision out of it. The mess is usually right about
*what matters* and wrong about *what is true*, because it was written from memory.

Your job is a trade. **They supply judgment. You supply measurement.** Neither of you can do the
other's half, and most bad versions of this task fail because one side tried.

## The one rule

> **Never ask a question you can answer by measuring, and never assert a fact you have not
> measured.**

Both halves fail constantly and they fail in opposite directions. Asking the user what they could
have been told wastes the scarce resource — their attention. Asserting from memory is worse: it
reads as authoritative and it is wrong at a rate that will surprise you.

This is not a warning about carelessness. In a long session working on a real system, claims made
from plausible inference were wrong roughly a third of the time. See
`references/grounding.md` for the catalogue of how — it is worth reading once, because the failure
modes are specific and repeat.

## The loop

1. **Read the target, then read the ground truth.** The document is a claim about a system. Go
   look at the system. Count things.
2. **Ask the ready, independent decisions together.** Give concrete options and a recommendation.
   Ask one if only one is ready.
3. **Write the answers in immediately, dated, with the reasoning.** A decision you did not record
   is a decision you will re-litigate.
4. **Report what the measurement contradicted.** This is where most of the value is.
5. **Repeat until the open-question list is empty**, then hand over what is *work* rather than
   *undecided*.

## Start by measuring, not by reading

Before the first question, get numbers. A document says "most of my notes are a mess"; the
measurement says "42% of one folder is under 40 words and the other folder is at 6%". The second
sentence starts a real conversation and the first one starts a vague one.

`scripts/census.py` does the generic version for a markdown corpus — per-folder counts, stub
rates, frontmatter key usage, orphan detection, dialect drift. Run it before you form an opinion:

```bash
python3 scripts/census.py <root> --frontmatter --stubs
```

For code, the equivalent is reading the actual entry point and running the project's own doctor,
linter, or test suite. Prefer the system's own instruments over your own — a tool the project
already trusts produces numbers the user already believes.

**What you are hunting for:** the gap between what the document claims and what is on disk. That
gap is the deliverable. In practice it is where every genuinely surprising finding comes from.

## Asking questions well

The full craft is in `references/question-craft.md`. The short version:

- **Up to four ready, independent questions per round.** Settle prerequisites first; one can be enough.
- **2–4 options each, mutually exclusive**, with a one-line consequence per option — not a
  description of the option, but what happens if it is chosen.
- **Put your recommendation first and mark it.** You have read the code and they have not.
  Withholding a recommendation to seem neutral just moves work onto them.
- **Use a preview for anything with shape** — a folder layout, a config block, a data model, a
  screen. A rendered example settles in seconds what prose argues about for paragraphs.
- **Never ask a question whose answer changes nothing.** If both branches lead to the same next
  action, decide it yourself and say you did.

The questions that produce the best answers are the ones where you have *already done the work*
and found a genuine fork. "Which of these two things that both exist should win?" beats "what do
you want?" every time.

### The round runs in the `ask` tool

When the `ask` tool is available, the round is one tool call, not prose. Each question takes 2–4
mutually exclusive options, a one-line consequence each, your recommendation marked and given
first, and a preview block for anything with shape. The user picks in one keystroke — the
recommendation is focused, so Enter accepts it — and can attach a note to each decision. **Read
the notes.** A correction to your framing arrives there far more often than in the choice itself,
and the tool tells you which picks overruled your recommendation, which is where the interesting
information lives. `s` skips a question and leaves it open: that is "not sure yet, come back",
recorded rather than lost.

Without the tool — another harness, a non-interactive run — ask the same round as text: numbered
options, your recommendation marked, and room for a note after each.

## Recording a decision

Each answer becomes a durable record at the point in the document where it applies, not in a
changelog at the bottom:

```markdown
> [!done] Q13 — answered 2026-08-23: **one vault-owned profile note.**
> Human-written, loaded by every agent. It survives a harness swap by construction, because it is
> a file rather than a feature of a harness.
>
> It also closes hole H1 in §5.1b — the durable-direction kind has had no owner since the Aim row
> was retired.
```

Three things make this work, and `references/document-shape.md` explains why each matters:

- **The date.** A decision without one cannot be re-opened intelligently later.
- **The reasoning, not just the choice.** The next reader is often the same person, having
  forgotten.
- **The consequence.** What this closes, breaks or obliges elsewhere.

Keep a numbered index of open questions in one place, and keep it honest — derive the counts from
the document rather than typing them. `scripts/integrity.py` checks that every question id has
exactly one record, every cross-reference resolves, and the index matches the body.

## Editing a long document safely

Once a document is a few hundred lines, hand-editing it breaks references silently. Two habits
prevent almost all of it:

- **Anchor on exact text and fail loudly when the anchor is missing.** A replacement that silently
  matches nothing is how a document ends up half-edited. `scripts/docedit.py` applies a batch of
  replacements atomically: if any anchor is absent, nothing is written and it tells you which.
- **Re-check integrity after every batch.** Section numbers, question ids and internal links drift
  the moment you insert a section.

## Correcting yourself

You will be wrong, because you are making claims about a system large enough that nobody holds it
in their head. When the measurement disagrees with something you said earlier, say so plainly and
move on:

> Correction: postgres was never down. It has run since 2026-08-21 — `pg_isready` isn't installed,
> so the check that "proved" it was down was measuring the wrong thing.

No apology, no re-litigation, no dwelling. One sentence for what was wrong, one for what is true,
and — where it earns it — one for *why the check failed*, since that is usually the reusable part.

Correct the document too, not only the conversation. A PRD carrying a claim you have disproven is
worse than one that never made the claim.

## When to stop

Stop when every question is answered, and then do one more thing: **separate what is decided from
what is merely unbuilt.** A handover that mixes "we have not decided" with "we have decided and
not done it yet" forces the next session to re-derive which is which.

```markdown
### Next session — the open points
Every question in this document is answered. What follows is work, not undecided design.

| # | Item | Why it is sized this way |
```

## What this is not for

- **Creative or exploratory writing**, where the point is voice rather than a decision.
- **A question the user has already answered.** Re-asking to seem thorough is the fastest way to
  lose their patience — read the conversation first.
- **Small, obvious calls.** Convention exists so nobody has to decide everything. Take the default,
  state that you took it, and spend the question budget on the fork that matters.

## Reference files

- `references/question-craft.md` — building a question round, option design, previews, and the
  failure modes of each
- `references/grounding.md` — the verify-before-claiming discipline, with a catalogue of real
  failures and the check that would have caught each
- `references/document-shape.md` — decision records, open-question indexes, supersession tables,
  and handovers

## Scripts

- `scripts/census.py` — measure a markdown corpus before forming an opinion
- `scripts/docedit.py` — atomic anchored edits, loud on a missing anchor
- `scripts/integrity.py` — cross-reference and question-id integrity for a long document
