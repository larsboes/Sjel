---
name: practice
description: Runs an adaptive practice session on claims the student already learned, so each claim comes to mind in situations unlike the lesson. Reads claim notes (type claim) and their Because chains from the vault. Poses one problem at a time, gives one hint at a time, and retests every miss before the session ends. Use when the student asks to practice, drill, review, or test themselves on a taught topic. Also use when the student brings an outside problem to work through. Do not use for teaching new material (use teach), for flashcard recall, or for vault work with no practice in it (use obsidian).
---

# Practice

Teaching puts a claim into the student's head. Practice makes the claim come to mind when only
the features of a new situation can trigger it. Change nothing in the material. Change only
when and how reliably it comes to mind.

Read the vault through the `obsidian` skill. Resolve the vault with `scripts/vault root`. The
contract row for claims defines the format: `summary:` holds the statement, and each
`## Because` bullet is one argument whose premises together imply the claim.

## Rules

1. **The target must be inactive.** Do not name the claim in the problem. Do not ask about a
   claim that the last problem or the last feedback just used. Interleave targets: switch to a
   different claim between problems.
2. **Count decision points, not steps.** A decision point is a place where the student must
   select a claim that is not yet in mind. Prefer short problems with two or more decision
   points. Avoid long problems that only test search.
3. **Aim for effortful success.** Target about 85 % success per decision point. Adapt difficulty
   on two axes: the number of claims one problem combines, and how far its surface is from the
   lesson's examples.
4. **Read the thinking.** Ask the student to think out loud in chunks and to give a confidence
   (0–100 %). Judge from the reasoning whether they solved it. Do not demand a written final
   answer when the reasoning already shows it.
5. **Hint one step at a time.** When the student stalls, give the smallest nudge toward the
   next move. Never reveal a step they can still produce. Check their attempt before you
   correct it. A misconception gets one surgical correction, not a lesson.
6. **Solve every problem first.** Before you pose a problem, solve it and look for a shortcut
   that skips the target claims. Use only the material in the selected claims.
7. **The student ends the problem.** A problem ends when the student solves it or asks to end
   it. Mistakes, time, or frustration do not end it.

## Session arc

1. **Select.** Ask which concept, claim folder, or Learning Log entry to practice. Read the
   claim notes and follow their `## Because` links one level down.
2. **Warm up.** Pose one to three easy problems on the weakest claims to calibrate difficulty.
3. **Main body.** Pose interleaved problems. Raise the claim count or the surface distance after
   each success. Lower one of them after a stall.
4. **Retest.** Before the session ends, retest every miss, every problem that needed many
   hints, and every problem the student gave up. Use a new surface, never the original one.
5. **Close.** Append one line to `Knowledge/Meta/Learning Log.md`:
   `- YYYY-MM-DD · practice · <topic> · claims: [[A]], [[B]] · weak: [[C]] · next: <suggestion>`.
   Write no other note.

An outside problem from the student enters at step 3. Solve it, map the claims every correct
solution uses, then run it with the same hints and retest.

## Provenance

The rules paraphrase the practice doctrine of LearnHarness
(<https://github.com/amosblomqvist/learnharness>, `src/agent/prompts/practice/10-doctrine.md`,
AGPL-3.0). This skill copies no text from it.
