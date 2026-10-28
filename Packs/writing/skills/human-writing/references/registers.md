# Register profiles

"Human" is not one voice. A technical report stays professional, marketing copy addresses the
reader directly, fiction has a narrator. Infer the register, then write the way a skilled
human writes *in that genre*.

Pass the matching `--register` to the scanner so it mutes the checks that do not apply. The
universal core below is fixed everywhere; only warmth, address, contractions, hedging, and
structural strictness flex.

## The universal core (fixed in every register)

Vacuity, fabrication, the rule-of-three reflex, bold-bullet listicles, puffery, vague
attribution, uniform sentence rhythm, terminology and dialect drift, restatement, and the
"not X, it's Y" template. These are AI in every genre. Fix them regardless of what is being
written.

## The profiles

| Register | Voice | Allowed here that is not elsewhere | Still wrong |
|---|---|---|---|
| `technical` (default) | Professional, direct, present tense | nothing extra, this is the strictest | warmth, selling, hype |
| `business` | Professional with a little warmth | a brief courteous opener or closer | gush, filler, hedging |
| `marketing` | Conversational, addresses the reader | direct address, contractions, light enthusiasm | **puffery and hype**, which is this genre's specific failure mode, and fake stats |
| `academic` | Formal, measured | measured hedging, passive voice, citations | unsourced "studies show", clichés |
| `casual` | Personal, conversational | contractions, first person, rhetorical questions, fragments | listicle padding, meta-commentary |
| `creative` | Narrative voice | em dashes, fragments, wide cadence, any vocabulary that serves the voice | clichés, pleonasm, puffery as lazy writing |
| `email` | Brief, courteous, direct | a one-line greeting and sign-off | jargon, padding, burying the ask, chatbot sign-offs |
| `release_notes` | Terse, user-facing, past tense | imperative or past bullets, fragments | marketing hype, vague "various improvements" |
| `ux_microcopy` | Minimal, plain | fragments, dropped articles, terseness | full-sentence padding, cleverness over clarity |
| `tutorial` | Instructional, second person, present | direct address, imperatives, numbered steps | over-explaining the obvious, rhetorical filler |
| `engineering_log` | Terse record of what was changed and measured, past tense | repeating the same fixed term (one term per concept), lists of metrics, fragments, an observation as precise as the author saw it | caveat stacking, trailing "…, not X" contrasts, conclusions the run does not support |

## Detection cues

Code blocks, metrics, or config imply `technical`. A call to action or a product benefit
implies `marketing`. Citations and measured hedging imply `academic`. A first-person anecdote
with casual contractions implies `casual`. A greeting plus sign-off implies `email`. A
versioned, past-tense, bulleted change list implies `release_notes`. Numbered how-to steps
imply `tutorial`. Dated runs with configuration blocks, screenshots and measured rates (experiment
logs, incident notes, investigation records) imply `engineering_log`. When the cues genuinely conflict and the answer changes the voice, ask one
short question rather than guessing.

## Format conventions, not preferences

Some genres impose hard format rules that are not stylistic calls. A commit message uses
imperative mood and a short subject line. A release note is past tense and user-facing
("Fixed a crash when…", never "We refactored…"). UX microcopy is terse and may drop articles.
An email leads with the ask. Honor these.

## When registers blend

A technical blog post is `technical` plus `casual`. The universal core still holds. Resolve
the voice toward the dominant audience and hold one voice rather than switching mid-document.
The calibration test: write as the most respected human author in that genre would, and ask
whether this voice would survive in the publication it is bound for.

Two rules survive every register. Never fabricate, and match rather than fake: do not bolt
slang onto a report or stiff formality onto a blog post. Forced personality in a genre that
rejects it reads as AI exactly as much as the stiffness would.

---

Extracted from the register profiles in
[stephenoffer/human-voice](https://github.com/stephenoffer/human-voice) (MIT), pinned
`9bcba2f`, and condensed out of that skill's SKILL.md to keep this one's body lean.
