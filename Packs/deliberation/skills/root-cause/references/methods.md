# Methods

Seven ways to walk from a failure to the condition that allowed it. Pick by the evidence you hold,
not by the one you know best. Each entry says what it needs, how it runs, and the way it fails —
every one of these methods has a characteristic wrong answer, and knowing it is most of the skill.

## Change analysis

**Needs:** a timeline with a known-good mark, a known-bad mark, and a list of what changed between.

Take every change in the window and ask two questions: could this reach the failing component, and
does its timing fit? A change that cannot reach it is out however suspicious it looks. Rank what
survives by reach, then test the top one by reverting it in a copy, disabling it, or finding the
log line that shows it acting.

**Fails by:** convicting the most recent change. Recency is not causation, and the deploy that
"broke it" is often the first one to run the code path a latent condition had been waiting in.
Also fails when the change list is built from memory rather than from a log — write down where each
entry came from.

## Five whys

**Needs:** one failure with a mechanism you can walk backwards, and a fact behind each step.

Ask why the observed failure happened. Answer with a fact that has a pointer. Ask why that is true.
Repeat. Stop when the next answer leaves the system you can change, or when the answer would be
about a person rather than a condition.

Five is a rough count, not a rule. Three is a valid depth and eight is valid too.

**Fails by:** becoming a chain of opinions. Every link needs its own evidence; a why with no
pointer makes every link after it fiction. It also fails on failures with several contributing
conditions, because a single chain can only carry one — use a fault tree instead.

## Is / is-not

**Needs:** at least one contrast — it fails here and not there, now and not then, for this input
and not that one.

Build two columns. In IS, write what is true where the failure appears. In IS-NOT, write the
closest neighbours where it does not: the other host, the earlier version, the working account.
Then ask what is different between the columns. The cause has to explain both columns; a cause
that also predicts failure in the IS-NOT column is wrong, however elegant.

**Fails by:** a lazy IS-NOT column. "It works on the other machine" is a start; "it works on the
other machine, which runs the same version, a different network path and an older config" is the
comparison that finds the answer.

## Fault tree

**Needs:** one symptom reachable by several paths.

Write the failure at the top. Underneath, write every immediate cause that could produce it, joined
by AND or OR. Expand each one the same way until each leaf is something you can test. Then test
leaves, cheapest first, and prune whole branches with each result.

**Fails by:** growing without pruning. A tree nobody prunes is a list of everything that could ever
go wrong. Test as you build.

## Recurrence pattern

**Needs:** the same failure at least twice, with dates.

Line up the occurrences and look for what they share: time of day, load, the day of a scheduled
job, a release, a person, a network condition. The shared factor is a candidate, not an answer —
run it through is/is-not against the times the same factor was present and nothing failed.

**Fails by:** stopping at the correlation. Two events sharing a Tuesday is not a mechanism.

## Blameless postmortem

**Needs:** an incident that is over, and its timeline.

Write the record: what happened, in what order, what was tried, what worked, how long each phase
took, and what the system allowed. Separate three things the write-up usually mixes — the trigger,
the condition that made the trigger sufficient, and what delayed detection or recovery. Each one
gets its own fix.

**Fails by:** turning into a narrative with a villain, or into a list of action items nobody sized.
Every item needs an owner-shaped fix, and the write-up says which of the three parts each one
addresses.

## Systems structure

**Needs:** a failure that came back after an event-level fix, or a fix that made something else
worse.

Walk down four layers: the events, the pattern over time, the structure that produces the
pattern (queues, limits, incentives, feedback), and the assumption that keeps the structure in
place. Draw the feedback loops that connect the elements. Mark each loop as reinforcing (it
amplifies change) or balancing (it resists change), and mark every delay. Then match the loops to
a known shape:

| Shape | Sign | Usual lever |
|---|---|---|
| Fixes that fail | The fix works, then a side effect brings the failure back | Trace the side effect before shipping the next fix |
| Shifting the burden | A workaround removes the pressure to fix the cause | Make the workaround expensive or temporary |
| Limits to growth | Growth stalls on a constraint nobody named | Find the constraint, not more push |
| Escalation | Two parties each react to the other | Change one party's goal |
| Drifting goals | The target moves down to meet performance | Fix the target outside the loop |

Push at the deepest layer that the operator controls. A rule, a goal or a feedback path usually
moves behavior more than a parameter does. Trace one round of the intervention's own feedback
before recommending it.

**Fails by:** drawing a diagram instead of a mechanism. Each arrow in a loop needs a fact behind
it, the same as each link in five whys.

## Not covered here

FMEA and other forward-looking methods ask what *could* fail. That is a design question, not an
investigation of a failure that already happened — use `red-team` on the design instead.
