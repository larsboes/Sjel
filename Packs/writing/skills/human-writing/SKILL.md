---
name: human-writing
description: >-
  Drafts, edits, reviews, and audits prose so it reads as one deliberate human voice instead
  of the model's default register. Combines a linter for the mechanical tells (em dash, "it's
  not just X, it's Y" cadence, assistant boilerplate, diction memes, bold-bullet listicles), a
  structural pass for the tells no regex catches (uniform rhythm, sycophancy, saying nothing
  at length), and a guard against the over-corrected "trying not to sound like
  AI" register that swaps one default for another. Facts, numbers, and citations stay
  invariant. Use when writing, drafting, editing, rewriting, reviewing, or auditing prose for
  a reader (post, email, essay, README, marketing copy), when text "sounds like
  ChatGPT", "reads like AI", or "is too polished", and for "de-slop this" or "make it sound
  like me". Trigger even if the request never says "AI tell". Do not use for authoring agent
  skills (use skill-creator), thesis or paper prose (use academic-writing), source code or
  web UI (use unslop).
allowed-tools: Read, Edit, Write, Bash, Glob, Grep
---

# human-writing

Read this first, because the most common misunderstanding sinks the whole thing.

**This skill does not supply taste, and it has no house style.** It does two narrow things.
It removes the specific cues that make text read as machine-written, and where the text reads
as AI because nobody ever chose a voice, it forces a deliberate one. Argument and judgment
stay with the author. A guardrail is not a writer.

**Most of the work is mechanical, and the plain fix is usually correct.** Nearly every cited
tell has a fix that needs no taste at all: the em dash becomes a comma or a period, `delve
into` becomes `look at`, the sycophantic opener gets cut, the `as an AI` boilerplate gets
deleted, the "in conclusion" recap goes. Lead with the plain fix. Reserve the voice work for
the genuinely stylistic calls, and for prose that reads as empty because nobody decided what
it was.

## The trap to avoid (this is the whole point)

Every anti-AI-writing effort fails the same way: it replaces one default register with
another. The 2024 tell was the smooth corporate voice. The 2026 over-correction is its mirror
image, the "trying not to sound like AI" voice: staccato fragments, forced lowercase, a
"here's the thing" cold open, a swear dropped in to seem casual, and the truly desperate move
of pasting fake typos to beat a detector. Readers clock the second one just as fast.

**Treat the over-corrected register as its own tell.** Three moves in particular. Conspicuous
em-dash avoidance: the fix for a dash is a comma or a period, not an ellipsis and not a
sentence visibly contorted around the gap, because the bending is as legible as the dash was.
Manufactured casualness: a "honestly", a lowercase "i", a "lol" bolted onto otherwise formal
writing reads as costume. And rhythm that is uniform in a new way: all-short sentences are as
mechanical as all-medium ones. Evenness is the tell, whichever length it settles on.

A tell is an *unspecified default*, not a banned word. If the writing genuinely calls for a
formal register, or the author genuinely loves the em dash, that is not slop. Honor the
escape hatches (see The scanner). Full catalog:
[references/over-correction.md](references/over-correction.md).

## Modes

| Mode | Trigger | Output |
|---|---|---|
| **AUDIT** | "does this sound like AI", "de-slop this", a file to check | Findings by priority, plus the audit block |
| **EDIT** | "rewrite this", "tighten this", a draft to fix | The rewritten text, plus the audit block |
| **REVIEW** | "review my writing", "walk me through it" | Interactive, one issue group at a time |
| **GENERATE** | "write", "draft", a brief rather than a draft | New copy that reads human from the first pass |

AUDIT reports and does not rewrite. EDIT rewrites and reports what changed. REVIEW is the
teaching mode: read the whole piece first, ask one question about audience and purpose, then
present two or three issues at a time and wait. Never open REVIEW with a wall of violations.
GENERATE drafts from a brief and is held to the same invariants, with the brief as the
invariant set.

When editing a file in place, confirm it is git-tracked first, or write a `.bak` copy. Show
the rewrite or a diff before applying it.

## Pin the voice before writing anything

Most "sounds like AI" outcomes are a specification problem, not a wording problem. An
unspecified prompt gets the median of the training data, and everyone's median is the same
smooth voice. Pin three things, in this order:

1. **The reader's job.** What the reader is trying to *do* with the text: follow a story,
   understand why, decide what to believe, decide to act, meet a voice, learn what changed,
   do a task, look something up. The job sets the spine, not the container it ships in.
   Taxonomy and the characteristic AI failure per job: [references/types.md](references/types.md).
2. **The register.** What "plain" means here, and what counts as a tell. A fragment is native
   in casual and a tell in formal; a contraction is right in most registers and wrong in a
   brief. Ten profiles with their conventions: [references/registers.md](references/registers.md).
3. **The speaker and the claim.** One real person with a stake, not "a helpful assistant",
   and the one thing the piece asserts, in a sentence, before the rest gets written. AI prose
   reads as empty because it is fluent with nothing to say. If the claim will not fit in a
   sentence, there is nothing to unslop yet. Method:
   [references/writing-with-intent.md](references/writing-with-intent.md).

When no brief is given, do not produce one median draft. Produce a deliberate one and state
what was chosen, or offer two genuinely distinct voices.

### Matching a specific person

For "write this in my voice" or "make this sound like me", humanizing is table stakes and the
voice is the point. Resolve a profile in order: an explicit path in the request, then
`$WRITE_VOICES_DIR/<name>.md`, then `~/.claude/writing-voices/<name>.md`. Profiles are
private and live outside this skill. If none resolves, say so and offer to build one from
[references/voice-template.md](references/voice-template.md). Never invent a voice profile.

## The five structural moves

These beat surface cleanup by a wide margin, and they are what the linter cannot see.

1. **Lurch.** Vary sentence length hard. Never three consecutive sentences within a few words
   of each other. Monotone length is the most-cited rhythmic tell and the one an ear catches
   before an eye does.
2. **Spike.** Vary information density across paragraphs. Pack one tight, let the next
   breathe. Uniform density is a machine signature even when every sentence is clean.
3. **Wander.** Do not march setup, complication, resolution, reflection. Discourse shape is
   the largest single detection signal and it survives paraphrasing.
4. **Shift register.** Move between precise and plain within a piece. One sustained tone is a
   costume. Match claim strength to evidence: understatement reads as confidence.
5. **Get specific.** Name the particular failure, the actual error code, the unglamorous
   detail. A specific a generic model could not have invented is the strongest human signal
   there is.

Depth, plus the second-dialect tells that appear *after* the obvious slop is gone:
[references/structural-craft.md](references/structural-craft.md). Full ranked tell catalog
with cited-vs-matched data shares and fixes: [references/tells.md](references/tells.md).

## The scanner

```bash
python3 scripts/detect_ai_prose.py <file>                      # report + floor score
python3 scripts/detect_ai_prose.py --register marketing <file> # mute checks the genre allows
python3 scripts/detect_ai_prose.py --register engineering_log <file> # experiment/incident logs: fixed terms, metric lists
python3 scripts/detect_ai_prose.py --dialect american <file>   # spelling drift
python3 scripts/detect_ai_prose.py --baseline <before> <after> # prints the delta
python3 scripts/detect_ai_prose.py --fail-over 5 <file>        # CI gate, exits 1 over 5
python3 scripts/detect_ai_prose.py --fix-dry-run <file>        # preview the autofix
python3 scripts/detect_ai_prose.py --fix <file>                # apply mechanical fixes only
printf '%s' "$TEXT" | python3 scripts/detect_ai_prose.py -     # pasted text
```

`--fix` applies only the unambiguous edits: dash normalization, decorative-emoji removal, 1:1
filler swaps. It never touches code, numbers, or links, and it does not vary the replacement
mark, so the rewrite pass still has to.

**Be honest about what the scanner is.** It is a floor. It computes rhythm, lexical
diversity, and n-gram repetition, but the highest-value tells are structural and it cannot
see most of them: sycophancy, weak stance, a paragraph that is fluent and says nothing. A
clean scan means the lexical layer is clean, not that the writing reads as human. The real
signal is the catalog read against the piece, plus the density number, not the pass/fail.

**Weight by density and concentration, not lone hits.** Six `delve`s in a 200-word paragraph
is slop; six across a 5,000-word essay is how the person writes. Two tells are absolute and
count on a single instance: the em dash, and leftover assistant boilerplate.

## Fix order

Structure beats substance beats vocabulary. Do not start with diction.

1. Cut vacuity. Delete what carries no information. This usually removes 15 to 25% of the
   words and most of the AI feel at once.
2. Fix rhythm. Break uniform sentence length.
3. Dismantle templates: the rule of three, bold-bullet listicles, the antithesis cadence, the
   five-paragraph mold.
4. Cut stance tells: meta-commentary, chatbot scaffolding, fence-sitting, empty conclusions.
   Then sharpen the evaluation. Commit to a recommendation, weight real tradeoffs, lead with
   the verdict, name genuine limits.
5. Unify. One term per concept, one dialect, one heading case, one tense, one voice.
6. Fix diction and mechanics last.

## Never fabricate

Humanizing is exactly when fabrication creeps in: a flat sentence gets rescued with a vivid
invented detail, a hedge becomes a confident false claim, a vague gesture grows a fake
statistic. The rewrite adds *voice*, never *facts*.

Numbers, dates, code, config, links, citations, quotes, defined terms, and proper nouns are
invariant. Vague to concrete is allowed only when the concrete detail is already in the
source. When a sentence is empty, cut it rather than dressing it. When the prose needs a fact
that is not available, emit `[SOURCE NEEDED]` or `[VERIFY]`, list it in the audit, and mark
the output draft-pending. Full protocol, including the claim-inventory diff that runs before
returning: [references/anti-hallucination.md](references/anti-hallucination.md).

## Before returning

Read it aloud. If it sounds like a metronome, vary it. Then check: shortest against longest
sentence, any run of three sharing a shape, whether the register shifted at least once, any
em dash outside a creative register, any sycophancy or "in conclusion" recap or "not X, it's
Y" left, and at least one concrete detail a generic model would not have produced. Diff the
claim inventory against the source. Re-run the scanner and quote both numbers.

Stop at a fixed point, capped at three passes. If tells remain after three, report them
honestly rather than over-correcting into the costume register.

## Reporting

Lead with the verdict and the single highest-impact change. Then findings by priority with
`file:line` and the fix. Close with:

```text
Score: <before> to <after>   (linter floor, not proof)
Words: <before> to <after>
Invariants: numbers ok, code ok, links ok, claims ok   (claim diff: +N added / N strengthened / N weakened / N dropped)
Placeholders left for the author: <list, or none>
Residual risk: <why a skeptical reader might still flag this, or none>
```

## What the scanner cannot see, and you must

- **Listicle shape.** A few headers ("5 ways to", "7 signs") sit in `meta_commentary`, but the
  structural mold is undetected. Check by eye.
- **Standalone bolded lead-ins.** `- **Term:** sentence` fires as `bold_bullets`; the same
  `**Term:** sentence` as its own paragraph does not, and it is a real tell.
- **Short text.** Burstiness, TTR, and n-gram checks report `n/a` under a few hundred words, so
  the score rests on the lexical layer alone. A clean scan there means very little.
- **Documentation that quotes tells to discuss them** scores high by design. This file and the
  references trip it heavily. Not a finding.

**Never game a detector.** They misclassify non-native-English writing as machine written
(Liang et al. 2023). No homoglyphs, zero-width characters, or deliberate typos, ever. The goal
is writing a skeptical human reads as human, not a number.

Upstream deltas, calibration caveats, provenance, and the drift check:
[references/maintenance.md](references/maintenance.md).
