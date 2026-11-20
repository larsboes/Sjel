---
name: root-cause
description: Investigates why something already failed and names the systemic condition that allowed it, never the person who touched it last. Picks the method the evidence supports — timeline and change analysis, five whys, is/is-not, fault tree, blameless postmortem — and ends with a cause, the probe that would falsify it, and a fix ranked by whether it removes the whole class of failure. Use when something broke, keeps breaking, regressed, or behaved differently than expected and the question is why — an incident, a postmortem, an RCA, a recurring bug, "it worked yesterday", "this keeps happening". Do not use for attacking a proposal that has not been built yet (use red-team), for choosing between options (use council), or for scanning running software for vulnerabilities.
license: MIT
---

# root-cause

One failure goes in. A named cause, the probe that would falsify it, and a fix ranked by whether
it removes the class come out.

Root-cause explains a failure that already happened. To attack a proposal before it ships, use
`red-team`. To choose between two repairs, use `council`.

Adapted from the RootCauseAnalysis skill in LifeOS by Daniel Miessler
(https://github.com/danielmiessler/LifeOS), MIT.

## Procedure

Copy this checklist and track progress:

```
- [ ] 1. State the failure as an observation, not a diagnosis
- [ ] 2. Build the timeline
- [ ] 3. List what changed
- [ ] 4. Pick the method
- [ ] 5. Generate candidate causes, then try to refute each one
- [ ] 6. Name the cause and its falsifying probe
- [ ] 7. Rank the fixes by what class each one removes
```

**1 — State the failure.** Write what was observed, when it started, and how it was noticed. "The
automations stopped running" is an observation. "The restart wiped them" is a diagnosis, and
writing one here decides the investigation before it starts. Say which of the two you were handed.

**2 — Build the timeline.** Read the logs, the history, the git log, the state file — whatever
records time. Put every dated fact in order, including the boring ones. Mark the last moment the
system is known to have worked and the first moment it is known to have failed. The cause is
between those two marks, and most investigations that fail never drew them.

**3 — List what changed.** In that window: deploys, config edits, upgrades, credentials, network,
hardware, and anything a person or an agent ran by hand. A failure with no change behind it is
possible but rare; when the list is empty, say so explicitly, because it moves the answer toward a
latent condition that was always there and toward what made it visible today.

**4 — Pick the method.** Read `references/methods.md` and pick the one the evidence supports:

| The evidence you have | Method |
|---|---|
| A clean timeline and a suspect change | Change analysis |
| One failure, a chain of mechanism to walk back | Five whys |
| It fails here and not there, now and not then | Is / is-not |
| Several possible paths to the same symptom | Fault tree |
| It already failed more than once | Recurrence pattern |
| An incident that is over and needs a record | Blameless postmortem |
| It came back after a fix, or a fix moved the problem | Systems structure |

Two methods are allowed. Three means the failure is not yet stated precisely enough — go back to
step 1.

**5 — Refute, do not confirm.** Write every candidate cause the evidence permits, including the
boring one and the one that embarrasses somebody. Then, for each candidate, look for the evidence
that would prove it *wrong*. Launch one `cause-hypothesis` agent per candidate, all in one message,
when there are three or more candidates or the check needs files read. A candidate nobody tried to
kill is a guess wearing a report's clothes.

**6 — Name the cause.** One sentence, in the form: *this condition existed, so that trigger
produced this failure.* Then write the probe: the command, the query or the observation whose
result would prove the sentence false. A cause with no falsifying probe is a story.

**7 — Rank the fixes.** Read `references/output-format.md`. Every fix is ranked by what it removes:
this instance, this failure, or this whole class. Say which. A fix that removes only the instance
is still worth shipping, and calling it a class fix is how the same incident returns.

## Evidence rule

- Every fact in the timeline carries its source: a log line quoted with its timestamp, a commit,
  a file path with a line, a command output.
- A claim with no pointer is marked `[unverified]` at the claim and never becomes the named cause.
- "It probably…" is a hypothesis. Write it as one, with the probe that would settle it.
- Never repair the evidence. A log that contradicts the story is the finding.

## Blameless, and what that actually means

A person or an agent doing the obvious thing and getting a failure is a fact about the system, not
about them. Write what the system allowed, not who acted: "a bulk API call disabled every
automation and nothing recorded the caller" is a finding; "someone ran the wrong call" is not, and
it stops the investigation exactly where it should continue. Where identity is genuinely part of
the mechanism — an unattributed action nobody can trace — the finding is the missing attribution.

## Hand off

- The answer is a choice between two or more repairs → `council`.
- The output is a proposal that has not been built yet → `red-team` it before shipping.
- The failure is a live outage → stop. Restore service first. An investigation during an incident
  competes with the repair for the same person.

## Error handling

- **No timeline is recoverable.** Say so and stop at step 3. The finding is that nothing recorded
  it, and the first fix is the record.
- **The evidence supports two causes equally.** Report both, each with its probe, and say which
  probe is cheaper to run. Picking one to look decisive is how the wrong one gets fixed.
- **The failure cannot be reproduced.** That is a property of the failure, not a reason to guess.
  Report the conditions under which it appeared and what instrumentation would catch the next one.
- **The caller wants the person named.** Refuse and name the condition.
