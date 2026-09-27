# How much does the assistant's harness matter?

First written 2026-09-27.

## What the sources show

**The harness moves cost more than success.** HarnessTax ran seven models in three coding
harnesses (Claude Code, Codex CLI and Pi) on 30 SWE-bench Lite and 30 Terminal-Bench 2.0 tasks,
three runs each. The same model often reached a similar success rate at very different costs.
Across shared models, Claude Code cost about 2.0× as much as Pi on SWE-bench Lite. Pi reached the
cost and success frontier on both benchmarks "by providing just four tools: read, write, edit,
and bash". Claude Code's mean initial context was over 10× Pi's
([Pan, Yang, Arabzadeh, Chiang, Stoica and Zaharia, HarnessTax, UC Berkeley and Arena, 2026](https://harnesstax.github.io/)).

**A provider's own harness is not always the best one.** In the same study, a model often did as
well or better outside the harness its provider built for it (same source).

## What the sources do not show

- HarnessTax measures coding tasks. Household tasks with typed tools are not measured.
- The authors note that the models may have seen these benchmarks in training.
- Sjel runs small models on the device first. How a large initial prompt affects a 3B model is
  not measured here.
