---
name: suggest-skills
description: Reads this machine's own prompt history and installed skills, finds the work that keeps recurring and the places the user kept having to correct the agent, and returns a ranked shortlist of skills worth building — each with the evidence that produced it. Read-only and proposal-only — it never writes, edits or installs a skill. Use when the request is "what skills should I build", "am I missing a skill", "skill gap", "based on my recent work, what would help", or when a review of a skill library asks what is absent rather than what is wrong. Do not use for writing or auditing one skill (that is the skill-authoring skill), and do not use it to decide whether an existing skill is any good.
license: MIT
---

# suggest-skills

One question: given what this person actually did, and where they had to repeat themselves, is
there a recurring problem that deserves a skill and does not have one?

It proposes. Somebody else builds. This skill has no capability to create or edit a skill, and
that is deliberate — a proposer that can also build grades its own homework.

Adapted from the SuggestSkills skill in LifeOS by Daniel Miessler
(https://github.com/danielmiessler/LifeOS), MIT.

## Procedure

Copy this checklist and track progress:

```
- [ ] 1. Gather the corpus with the script
- [ ] 2. Read the clusters and throw out the noise
- [ ] 3. Read the friction pairs
- [ ] 4. Dedup against what the skills actually do
- [ ] 5. Test each survivor against the bar
- [ ] 6. Rank, and say what evidence would change the ranking
```

**1 — Gather.** The model does not gather. Run:

```bash
bun scripts/collect-signals.ts --days 30 --json > /tmp/signals.json
```

`--days N` sets the window. `--history PATH`, `--skills DIR` and `--packs DIR` override the
sources; `SJEL_ROOT` adds the Pack registry when it is set. Two runs over the same files return
the same corpus, which is what makes a proposal arguable instead of a mood.

Report every line in `warnings` to the user before using the output. A missing source is not a
detail: it changes what "no gap found" means.

**2 — Read the clusters.** Each cluster is prompts that share vocabulary, with `sessions`,
`prompts`, `spanDays`, the co-occurring words in `with`, and three samples. The grouping is lexical,
so some clusters are junk — a generic verb that survived the stoplist, or one long conversation
that recurs in three sessions because it was interrupted twice. Throw those out by reading the
samples. Keep a cluster only when the samples describe the *same kind of work* rather than the
same words. `dominant` lists the words too common to cluster: they name the corpus, and a proposal
whose whole content is a dominant word is a proposal to build a skill about "everything".

**3 — Read the friction pairs.** `friction` holds prompts that look like corrections, each with
the prompt before it. A cluster with friction inside it outranks a bigger cluster without: a topic
can look covered while the user keeps hitting the same wall inside it, and the wall is what a skill
would remove. Read the pair, not the marker — the regexes over-fire, and a calm "fix it" is not
frustration.

**4 — Dedup against coverage.** For every surviving candidate, find the skills in `registry` that
might cover it, then **read their SKILL.md bodies**. A name match is not coverage. A skill covers a
candidate only when it addresses that failure class, is installed where the work happens, and
would actually trigger on the words the user typed. Write the check down: "`obsidian` covers vault
writes but its description never mentions X, so it does not fire on these prompts" is a finding
either way — it is as likely to produce a description fix as a new skill, and the description fix
is the cheaper answer.

**5 — Test against the bar.** Read `references/judging.md`. A candidate has to pass all five:
recurrence across sessions, a stable procedure, a failure the model gets wrong without help, a
cost that repeats, and no existing owner. A candidate that fails one is reported as rejected, with
which test it failed. The rejects are half the value of the run: they are the reason not to build
the same idea again next month.

**6 — Rank and hand off.** Read `references/output-format.md` for the shape. Rank by
`(sessions × friction) ÷ effort`, argue the top three in prose, and end each with the evidence that
would move it up or down. Then stop. Building is the skill-authoring skill's job, and the user's
call.

## What this cannot see

State these limits in the report rather than letting the reader assume otherwise:

- **Only prompts, never what happened next.** The corpus is what the user typed. Whether the agent
  then did it well is not in there, so this skill can find a recurring *request* and cannot find a
  recurring *failure* except through the friction markers.
- **No satisfaction data.** There is no rating store on this machine. Friction is inferred from
  wording, which over-fires and under-fires.
- **Lexical clusters, not semantic ones.** Two prompts about the same problem in different words
  land in different clusters. A gap the user described three different ways can be invisible here,
  and reading the samples is the only defence.
- **The window hides slow-burn work.** A quarterly task recurs on a schedule this window cannot
  see. Re-run with a longer `--days` before concluding a domain is quiet.

## Error handling

- **No history file.** Say so and stop. There is nothing to analyse and no honest way to guess.
- **Every cluster is noise.** Report that. "Nothing recurred enough to justify a skill" is a
  legitimate and useful answer, and manufacturing three proposals to fill a report is how a skill
  library fills up with skills nobody triggers.
- **The best candidate is covered by an existing skill that never fires.** The proposal is a
  description fix, not a new skill. Say which skill and which words are missing from its
  description.
- **The user asks this skill to build the winner.** It cannot. Hand the shortlist over.
