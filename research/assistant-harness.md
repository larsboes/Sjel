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

Once the suite exists, use it for the [Gemma 4 E2B trial (#239)](https://github.com/larsboes/Sjel/issues/239)
and [MiniCPM5-2B trial (#240)](https://github.com/larsboes/Sjel/issues/240).

## What the sources do not show

- HarnessTax measures coding tasks. Household tasks with typed tools are not measured.
- The authors note that the models may have seen these benchmarks in training.
- Sjel runs small models on the device first. How a large initial prompt affects a 3B model is
  not measured here.
