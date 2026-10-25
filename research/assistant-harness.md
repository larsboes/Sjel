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

## Can the harness improve through code and experiments?

Reviewed 2026-10-24. This section holds the evidence; [the agent plan](../docs/plans/agent.md#broader-harness-scope-and-factual-precision)
owns Sjel's chosen scope, evaluation sequence and unresolved implementation choices.

### Meta-Harness: improve the code around a fixed model

Meta-Harness proposes an outer optimization loop. A coding agent selectively reads previous
candidate source, execution traces and scores, then writes a new harness candidate. The
underlying task-solving model stays fixed. Prior artifacts remain available through a filesystem
rather than being reduced to a single summary or packed into every prompt. These are reported
methods, not measurements made in Sjel
([paper v1, sections 3–4](https://arxiv.org/html/2603.28052v1)).

The evaluation boundary matters. In v1, the classification experiment uses held-out evaluation,
and the math experiment tests a selected retrieval harness on unseen problems and additional
models. The coding experiment searches and evaluates on the same 89 TerminalBench-2 tasks,
which the paper describes as a discovery problem. Its leakage audit does not turn that coding
experiment into an untouched test-set evaluation. Sjel must distinguish search improvement from
generalization when adapting this method
([paper v1, sections 4.1–4.3](https://arxiv.org/html/2603.28052v1)).

The repository supplies classification and terminal-task examples. Its onboarding guide asks for
a domain specification covering the task, editable boundary, baseline, evidence, budget and
evaluation split before implementation. It also keeps final test data/results outside proposer
access. This is a useful preparation checklist; it does not supply Sjel's domain evaluators
([repository](https://github.com/stanford-iris-lab/meta-harness),
[onboarding](https://github.com/stanford-iris-lab/meta-harness/blob/main/ONBOARDING.md)).

### Schema: test an executable hypothesis during a task

Schema gives an agent a persistent program workspace and tools to check its model of an unfamiliar
environment against recorded interactions, search within that model and execute plans under
prediction checks. A mismatch stops the remaining plan and returns evidence for revision. This
is a task-level learning loop, distinct from changing the harness itself
([paper, section 3](https://arxiv.org/html/2609.39140)).

The reported evaluations cover ARC-AGI-3, DiG-bench and MazeBench. The
[original project blog](https://schema-harness.github.io/) uses fallback-pair public-set results;
the [newer paper](https://arxiv.org/html/2609.39140) reports per-model comparisons. Preserve that
distinction when citing a result. RHAE combines completion and action efficiency; it is not a
percentage of factual claims answered correctly. The code is published in the
[official repository](https://github.com/guanning03/Schema).

### What Sjel can take from these sources

The proposed adaptation is to preserve evidence separately from editable hypotheses, make
predictions testable, and save candidate code with the outcomes needed to diagnose failures.
Keep independently controlled checks between a proposal and its execution or promotion. This
is an architectural inference from the methods, not a demonstrated Sjel improvement.

For coding and experiments, executable checks can evaluate behavior, calculations and interface
contracts. For research, precise writing and legal/administrative tasks, the input premises also
need verification: source passages, document versions and unresolved contradictions remain
necessary. A program consistent with previous observations may still predict a new situation
incorrectly. Encoding an assertion in code does not establish its truth.

The [main plan](../docs/plans/agent.md#sequence-plan-now-compare-with-pi-after-the-evaluation-contract-exists)
records coding and autonomous research/experiments as the first evaluation profiles. Other
profiles retain the same factual-precision requirement. The planned Pi/Sjel comparison separates
harness changes from retrieval changes; it is not a new result. Storage choices remain in
[the data-backbone proposal](../docs/plans/agent.md#agent-data-backbone-turso-and-graph-integration).

## Source index

These are canonical public references for this investigation. Local copies of the Meta-Harness
v1 PDF (`2603.28052v1.pdf`) and the operator's Obsidian clipping were supplied as reading routes;
they are not repository dependencies. No private filesystem location is required to follow this
entry.

| Source | Links | Use and limit |
|---|---|---|
| HarnessTax | [Project and paper links](https://harnesstax.github.io/) | Coding-harness cost/success comparisons; does not establish results for Sjel's other profiles. |
| Meta-Harness | [Repository](https://github.com/stanford-iris-lab/meta-harness), [v1 paper HTML](https://arxiv.org/html/2603.28052v1), [v1 PDF](https://arxiv.org/pdf/2603.28052v1), [project page](https://yoonholee.com/meta-harness/), [onboarding](https://github.com/stanford-iris-lab/meta-harness/blob/main/ONBOARDING.md), [terminal-harness artifact](https://github.com/stanford-iris-lab/meta-harness-tbench2-artifact) | Candidate-code search and experiment history; distinguish held-out experiments from coding discovery on the search benchmark. |
| Schema | [Original blog supplied by the operator](https://schema-harness.github.io/), [updated project page](https://guanning03.github.io/schema-webpage/), [paper](https://arxiv.org/abs/2609.39140), [paper HTML](https://arxiv.org/html/2609.39140), [repository](https://github.com/guanning03/Schema), [released ARC traces](https://huggingface.co/datasets/schema-harness/arc-agi-3-schema-traces) | Executable hypotheses and verified action; published game results do not establish factual accuracy in professional work. |
| SQL and agent state | [Turso Database](https://github.com/tursodatabase/turso), [libSQL](https://github.com/tursodatabase/libsql), [AgentFS](https://docs.turso.tech/agentfs/introduction), [Sjel's Turso guide links](../docs/plans/storage.md#6-agent-databases) | Storage candidates and recipes. Keep engine identities separate; storage features do not establish agent quality. |
| Graph engines | [Grafeo](https://github.com/GrafeoDB/grafeo), [Grafeo server](https://github.com/GrafeoDB/grafeo-server), [IndraDB](https://github.com/indradb/indradb), [Sjel's integration alternatives](../docs/plans/agent.md#integration-candidates-to-narrow-together) | Query/storage candidates from the operator's comparison, retained for evaluation; no engine is selected by this entry. |

Meta-Harness and Schema have `inspiration` verdicts in
[`upstreams.toml`](../upstreams.toml). The source index includes supplementary artifacts for
follow-up; listing one does not claim that its code or raw traces have been audited or reproduced.

## First local read-only pilot

This is a smoke test, not a reliability benchmark: one prompt per harness, one model, one source
file, one run each. On 2026-10-21, the same copy of `libs/agent/src/session.rs` was placed in a
temporary workspace for both runs. The prompt was:

> Read `libs/agent/src/session.rs` and explain how recovery works in exactly three bullets. Do not modify files.

| Runner | Model and request | Tool use | Wall time | Outcome |
|---|---|---|---:|---|
| `sjel-agent` | Qwen3.5-4B-MLX-4bit via oMLX 0.7.1; temperature 0, 512-token cap, thinking disabled | One `read` call. The normal coding tools remained available; extensions were disabled. | 9.1 s | Three bullets, but it treated every unterminated final line as truncated. |
| Pi 1.3.0 | Same oMLX model; temperature 0, 512-token cap, Qwen thinking disabled | One `read` call; only the `read` tool was enabled. | 8.1 s | Three bullets, but it also missed the unterminated-valid-record case. |

The code distinguishes three cases: an invalid or incomplete unterminated tail is truncated; a valid
JSON record without a trailing newline is kept and given a newline; a malformed complete line is an
error with its line number. Neither answer made that distinction. Both used the correct tool, so
this run found an answer-grounding failure, not a tool-call failure. The result does not isolate
model from harness: the prompts and model were paired, but the system prompts and tool permissions
were not identical. In particular, the `sjel-agent` run tested compliance with a read-only request,
not enforced read-only access.

A separate check used Ollama `qwen3:4b` with `think:false`. It made one `read` call and returned an
inaccurate explanation after 27.5 seconds. This is not part of the Qwen3.5 harness comparison, and
it does not show that disabling thinking fixes the original failure.

The Qwen3.5 weights occupy about 2.9 GB on disk. Peak memory, temperature, fan noise, token counts,
and repeated-run variance were not measured. Weight size and a short request do not establish that
a model runs cool or quietly.

## Rust agent profiling probe

A focused release-mode probe exercised the `read` tool directly through
`tools::coding`; it did not run a full `Agent::run` turn. Ten consecutive reads of
synthetic text files averaged 0.13 ms for a 1 MiB file, 9.3 ms for a 64 MiB file
at line 1, and 11.8 ms for the 64 MiB file at line 250,000. Cache state was not
controlled. The read tool loads the whole file to count lines, so peak memory was
not measured and may matter more than latency for large files.

An Instruments Time Profiler recording of the streamed-response-fold unit test
was too short and dominated by test/startup work to identify an agent hot spot.
These probes do not justify a latency optimization. Code inspection did find that
a model could request an arbitrary number of parallel read-only tool calls, each
starting a scoped thread. `Agent::call_all` now caps concurrent calls at the
smaller of available parallelism and eight, and a unit test checks the cap and
result ordering. This is a resource bound, not a measured speedup.

## Deferred follow-up: repeatable benchmark

The pilot needs a controlled suite before it can support reliability claims. Its scope is tracked
in [issue #241](https://github.com/larsboes/Sjel/issues/241). That issue covers source-grounded
cases, matched harness settings, enforced read-only access, repeated runs, and resource readings.

The [agent-harness development specification](benchmarks/agent-harness/README.md) now provides
eight cases for the selected coding and research/experiment profiles, with input digests,
expected claims, task permissions and evaluator checks. It includes the session-tail distinction
missed in the pilot. Independent agent review corrected scoring and case gaps; human label
approval remains pending. An [offline evaluator with an OrbStack candidate-code path](benchmarks/agent-harness/README.md#offline-evaluator-and-orbstack-execution)
checks tool boundaries and projection artifacts, with no solver-model results yet. The patching and experiment-tool cases extend beyond issue #241's read-only
scope, so runner work needs an explicit scope decision before implementation.

Once the suite exists, use it for the [Gemma 4 E2B trial (#239)](https://github.com/larsboes/Sjel/issues/239)
and [MiniCPM5-2B trial (#240)](https://github.com/larsboes/Sjel/issues/240).

## What the sources do not show

- HarnessTax measures coding tasks. Household tasks with typed tools are not measured.
- The authors note that the models may have seen these benchmarks in training.
- Sjel runs small models on the device first. How a large initial prompt affects a 3B model is
  not measured here.
- Meta-Harness and Schema do not establish improvements on Sjel's tasks or a need for a graph
  database. Their methods can be prototyped with ordinary files and a run index.
- No reviewed source here demonstrates reliable legal/administrative reasoning or source-grounded
  writing under Sjel's contracts. Each profile needs its own evidence and acceptance checks.
- Sjel has not measured gains from autonomous harness evolution, nor established a promotion
  policy or an optimization budget. The earlier local smoke test remains the local evidence.
