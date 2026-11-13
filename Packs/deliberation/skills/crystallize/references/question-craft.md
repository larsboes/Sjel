# Question craft

The question round is the engine. Everything else in this skill is preparation for it or
bookkeeping after it.

The framing that makes it work: **the user's attention is the scarce resource, not your effort.**
You can read ten thousand lines. They can answer maybe twenty questions before quality drops. So
every question must be one only they can answer, and it must arrive already narrowed.

## Anatomy of a good round

**Batch up to four ready, independent questions.** A dependent decision waits for its prerequisite.
One question can be a complete round; do not add questions merely to fill it.

**2–4 options each**, mutually exclusive, each with a one-line consequence. The description says
*what happens if you pick this*, not what the option is. "Keep both vocabularies" describes;
"Two vocabularies with a translation table — which is what your own rule forbids" decides.

**Your recommendation first, marked.** You read the code. They did not. A neutral menu is not
neutrality, it is unshared work. Mark it plainly and let them overrule you — they often will, and
the overrule is usually informed by something you had no way to know.

**A preview wherever the choice has shape.** Folder layouts, config blocks, data models, screen
sketches. Prose comparing two structures is slow to read and easy to misread; two rendered blocks
side by side settle it instantly.

**A note line under each decision.** The choice records the decision; the note records the reason,
and the reason is the part the next reader needs. Leave room for it, and read it before you write
the answer down.

## How the round maps onto the `ask` tool

When `ask` is available, the round is one tool call and each beat above is a field:

| Beat | Field |
|---|---|
| The fork's label | `title` ("Data classes") — shown beside the `id`, which is what the record cites |
| The evidence | `context` — what you measured, what disagrees, where it lives |
| The question | `prompt` |
| An option | `options[].label` |
| What happens if chosen | `options[].consequence` — the cost, not a description |
| What it looks like | `options[].preview` — rendered only for the highlighted option |
| Your recommendation | `recommendedIndex`, with that option at index 0 |
| The user's reason | comes back as `note` on that decision |

Send the ready questions together in one call. The tool shows one at a time, but a round is still
one conversation turn, not a session.

## What makes a question worth asking

The best questions come from *already having done the work* and hitting a genuine fork. In
practice they are almost always one of these shapes:

**Two things exist and disagree.** "The document defines four data classes. The code ships three,
and the ratified rule says never re-derive them. Which wins?" The user cannot have known both
sides. Now they do, and the decision is theirs.

**A decision was made without a fact.** "You rejected this technique yesterday. It turns out the
implementation is deterministic, not the guessy version you were rejecting. Does the rejection
stand?" Re-opening a decision with new evidence is a service, not a challenge.

**Cost is invisible from where they sit.** "Web-only ships in weeks. Native is about a year. Which
does v1 require?" They may not have priced it. Price it, then ask.

**Their own rule points somewhere they may not want to go.** "Your rule says volatile state
belongs outside the vault. A reading backlog is volatile state. 83 of these 102 notes are a
backlog." Following someone's stated principle to an uncomfortable conclusion is exactly what they
cannot do for themselves.

## What not to ask

**Anything measurable.** "How many notes have no type field?" is a command, not a question.

**Anything with no consequence.** If both branches lead to the same next action, decide it, say you
decided it, and move on.

**Anything they already answered.** Re-reading the conversation costs you seconds and costs them
trust.

**Open-ended prompts, as the default.** "What do you think about X?" hands the structuring work
back — which is the work they asked you to do. Reserve open questions for the rare case where you
genuinely cannot enumerate the space, and say why you are asking openly.

**Everything at once.** A round is a conversation turn, not a form. If you have twelve questions,
you have not done enough work to narrow them.

## Handling the answers

**Non-answers are answers.** "Not sure yet, let's discover it" means *keep going and come back*.
Record it as open with a marker, do not press. In the `ask` tool this is the `s` key, and the
decision comes back marked skipped rather than answered — never turn a skip into a silent vote for
your recommendation.

**A correction to your framing is the most valuable reply.** When someone says "gear doesn't go in
Knowledge, gear is a database" they have told you your model is wrong, not just their preference.
Rebuild the model before continuing.

**A reversal is information, not a mistake.** "Atlas should be Self" followed by "no Self folder
please" is someone thinking, which is the point. Take the latest, note it superseded the earlier,
move on without commentary.

**When they pick something you argued against, take it fully.** State the concern once, in a
sentence, then implement their choice properly and mitigate what you can inside it. Re-arguing is
the fastest way to stop being useful. Doing a deliberately poor job of their choice is worse.

## After every round

Write the answers into the document **before asking the next round**. Two reasons: an unrecorded
decision gets re-litigated, and writing it usually surfaces the next question, better-formed than
anything you would have guessed.

## Worked example

A weak round:

> 1. How should we handle data privacy?
> 2. What about the vault structure?
> 3. Any thoughts on the dashboard?

Three open questions, no work done, all the structuring pushed back onto the user.

The same round after doing the work:

> **Q1 — Data classes.** §6.1 defines C0–C3. `libs/content-item/src/lib.rs` already ships three
> (`public`/`personal`/`vault`) and `Boundary` §V7 ratifies them with the falsifier *"broken when a
> class is recomputed instead of inherited"*. Your new scheme trips that falsifier.
> - **Inherit the existing three (recommended)** — obeys V7; C3 secrets stay outside the class
>   system since they are blocked at the tool boundary, a different mechanism
> - Keep C0–C3, migrate the code — the document leads; costs a migration across two libs and a
>   live CHECK constraint
> - Map them as aliases — two vocabularies with a translation table, which V7 forbids by name
>
> **Q2 — Atlas is 42% stubs against Knowledge's 6%.** Every rotted folder is named after a *thing*;
> every healthy one after a *statement*. Keep, split, or dissolve?
> …

Each question names a real fork, cites where the evidence lives, and gives a consequence per
branch. The user supplies exactly what only they can: which future they want.

## Calibrating the round to the stage

**Early** — few questions, wide. The goal is to find the load-bearing forks, not settle details.
Expect "not sure yet" and treat it as a map of where to measure next.

**Middle** — the densest rounds. You know the system, they know their intent, and each question
closes something.

**Late** — mostly confirmations and cleanup. If you are still asking wide questions here, something
earlier did not get recorded.

**Never** — questions asked to appear thorough. A round with nothing at stake teaches the user that
rounds can be skimmed, and the next one that matters gets skimmed too.
