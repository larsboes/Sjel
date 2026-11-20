# Depth passes

Notes describe what their author thought about. The missing requirements sit in the angles the
author did not take. A depth pass reads the same material again through one named angle and
records only what that angle adds.

## When to run them

Run them after the census and before the first question round, when the notes are a plan, a
spec or a design. Skip them for a factual cleanup, where the census already found the gaps.

## The passes

Pick three to six that fit the material. Run them one at a time, in this order when they apply.

| Pass | The question it asks |
|---|---|
| Constraints | Which parts are hard constraints (physics, law, a contract, a measured limit)? Which are soft constraints (a preference, a convention), or assumptions nobody checked? |
| User | What does the person who uses this do first, and where do they stop? |
| Failure | What happens when each dependency is slow, absent or wrong? |
| Scale and time | What changes at ten times the volume, and after one year of use? |
| Adversary | Who gains from misusing this, and what does it hand them? |
| Operator | Who runs this, how do they see that it is healthy, and how do they undo it? |
| Data | What does it store, which data class is it, and when is it deleted? |

The constraints pass comes first when it applies. An assumption that this pass exposes often
cancels a requirement before the other passes expand it.

## Rules

1. **One angle per pass.** A pass that answers two questions answers neither completely.
2. **Record only the additions.** Each pass writes new requirements, risks or open questions,
   each with the pass that found it. A pass that adds nothing says so in one line.
3. **Stop at saturation.** When two passes in a row add nothing, stop, even if passes remain.
4. **Facts still need measurement.** A pass produces questions and candidate requirements. Ground
   each one by the rules in `grounding.md` before it goes into the document.
5. **Feed the question round.** Each addition that only the user can decide becomes a question
   for step 2 of the loop.
