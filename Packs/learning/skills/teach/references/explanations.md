# Teaching explanations

Read this at Phase 3, step 1, before the first explanation of a session. It sets the shape of one
explanation step. It is not a note template: the terminal turn teaches, and the note keeps only
the clean result.

## Shape of one step

Use the parts that the step needs, in this order.

1. **Heading that states the claim.** "Lag counts unconfirmed messages, not unprocessed ones",
   not "Consumer lag". The learner can check the heading against the body.
2. **The mechanism in one sentence.** Name the whole system before its parts: "A topic is N
   append-only logs, and the key selects the log." Terms come after the learner holds the picture.
3. **Parts as a table, when there are three or more.** One row per part: what it is, what moves
   it, what reads it. A table of moving parts replaces a paragraph of definitions.
4. **The relation as a formula, with a words reading.** `$\text{lag} = \text{LEO} - \text{committed}$`,
   then "lag is the distance from the last written message to the last confirmed one". See
   "Math in the terminal" below.
5. **The consequence.** What the relation makes true that the learner did not expect. This is the
   answer to the step's prediction.
6. **Back to the learner's case.** When the learner brought a real problem, connect the step to it
   as a numbered causal chain, one link per line. Each link uses only established steps.
7. **The same pattern elsewhere.** When the structure repeats at another layer, say so in one
   sentence: "a blocked offset stalls a partition, and a busy task stalls a single-thread
   scheduler queue". This is how the learner transfers the idea.
8. **Limits.** State what the explanation does not cover and where your confidence is lower.
   Correct an earlier step openly when new evidence changes it.
9. **The test that decides.** When two explanations fit the facts, name the one observation that
   separates them and what each result would mean.

A comparison table fits two modes of one mechanism (on/off, eager/cooperative): one row per
property, one column per mode.

## Language

- One idea per paragraph. Short turns, so the learner's thinking arrives often.
- Use the learner's own words for their system, and the field's terms for the concept.
- A strong image is allowed when it carries the mechanism ("removing the thermometer does not cure
  the fever"). Use at most one per step.
- No emoji, no mode announcements, no closing offers.

## Terminal turn and note

| Terminal turn | Concept note |
|---|---|
| Prediction, its resolution, the learner's case | Claim heading, mechanism, table, formula, consequence |
| Questions, hints, feedback | Links to prerequisites, limits, the deciding test |
| Corrections as they happen | Only the corrected statement |

The note never contains the conversation. It keeps the general mechanism. The learner's case
appears in it only as a worked example, and only when the example makes the mechanism clearer.

## Math in the terminal

Pi's TUI renders basic LaTeX as Unicode: inline `$…$` and `\(…\)`, display `$$…$$` and `\[…\]`
on their own lines. Display math stacks fractions, sums and integrals.

- Supported: fractions, sub- and superscripts, `\sum`, `\int`, `\sqrt`, Greek letters, `\text`,
  `pmatrix`, common relations and arrows.
- Lossy: `\mathbb` (drops the blackboard style), `\underbrace` (drops the brace).
- Not supported, shown as raw source: `\color` and other styling commands.
- Inline `$` math must have no space just inside either delimiter and no digit right after the
  closing `$`, or the TUI shows it as plain text.

Give every formula a plain-words reading on the next line. When an expression needs more than the
supported set, write it in the note, where Obsidian renders it in full, and show the source in a
code block in the terminal with the words reading. Other harnesses may not render LaTeX. The words
reading keeps the step readable there.
