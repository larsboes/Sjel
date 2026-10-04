---
project: sjel-agent
type: isa
phase: climbing
progress: 20
principal_stated_goal: "pi agent inspiration rust implementation of slim core and official extensions from start but opt in"
---

# ISA · agent

Lib-scoped state of record for `libs/agent` (crate `sjel-agent`) and the extensions built on it.
Repo-wide items stay in the root `ISA.md`. The idea, the why and the scope are in the private
vault, `Projects/Sjel/Agent/PRD sjel-agent.md`. This file holds the testable claims.

## Problem

The coding agent on this machine is pi 1.0.1 (`@earendil-works/pi-coding-agent`). Measured
2026-10-03, a pi session here loads four extensions from `Packs/*/extensions/`, four packages
(three vendored under `Packs/harness/pi-packages/`, about 70,000 lines of TypeScript between
accordion, pi-subagents and pi-web-access, non-test `.ts` and `.js` counted with
`git ls-files | wc -l`, plus `ponytail` from git), 29 skills and one MCP server (`~/.pi/agent/settings.json`, `~/.pi/agent/mcp.json`).

Two things follow. All of it loads in every session, whether the session needs it or not. And
pi's extension model is TypeScript in-process with 12 integration points (the table in pi's
`docs/extensions.md`, "Choose an integration point"), which Sjel would have to track release by
release to keep the vendored packages working (`Packs/harness/README.md`, the Accordion deltas).

`libs/agent` started 2026-10-03 as the minimum that runs: one loop, six tools, sessions, and a
Ctrl-C that stops one turn. This file plans what comes next without letting the core grow.

## Vision

A bare `sjel-agent` run is the slim core and nothing else. Every capability beyond it is an
official extension: a first-party Rust crate, compiled into the binary, that does nothing until
the operator names it. The coding CLI and `capabilities/assistant` build on the same loop and
choose different extensions.

## Out of Scope

- **Third-party extensions.** `README.md`, "Scope": "Only first-party extensions run. There is
  no extension store." No plugin loading, no WASM host (`upstreams.toml [extism]`, rejected
  2026-10-01 on the same line).
- **Feature parity with pi.** Session trees and branching, themes, RPC mode and prompt templates
  are pi features this plan does not copy. pi stays installed and in use beside sjel-agent.
- **A second wire format.** OpenAI chat completions only, decided 2026-10-03 (Decisions). Every
  backend in the overlay's `config/inference.json` speaks it.

## Principles

- **The core does not grow for an extension.** An extension uses a hook the core already has. A
  new hook is a core change and needs a decision here first.
- **Off until named.** Nothing beyond the core runs unless config or a flag names it. The one
  exception is the secrets guard (D3).
- **Consume Sjel's contracts, do not copy them.** PRD Axon §7.2 (frozen 2026-09-26) put skills
  in `Packs/` and tool contracts in `schemas/` and the capability manifests, so a harness swap
  stays cheap. Extensions read those sources. They define no format of their own.
- **A speed claim carries its measurement.** Otherwise it says "not measured".

## Constraints

- **C1** — `cargo test -p sjel-agent` and `cargo clippy -p sjel-agent --all-targets` pass
  before a claim below is marked done. Each extension crate adds its own `cargo test -p`.
- **C2** — No `unsafe` in any agent crate (workspace `unsafe_code = "deny"`, `Cargo.toml`). A
  need for it goes through a reviewed crate with an `upstreams.toml` row, as `signal-hook` did.
- **C3** — HTTP is blocking, through `libs/sjel-http`. An async caller uses `spawn_blocking`
  (`libs/sjel-http/Cargo.toml`, the dependency comment).
- **C4** — A session file can hold any file the model read. Sessions and extension state live
  in the private overlay, mode 0600, never in this repository.

## Goal

A daily coding session on this machine runs on sjel-agent with the five wave-1 extensions
enabled (skills, guard, MCP, compaction, Rust diagnostics), and a bare run still behaves exactly
like the core shipped 2026-10-03.

## Features

### F0 · Slim core

Built 2026-10-03. The baseline every later claim is measured against.

- [x] AGT-1 — the loop streams a completion, runs the tool calls, and repeats until the model
  answers without one. Evidence: `loop_runs_parallel_tool_calls_and_streams_the_final_answer`
  (`src/lib.rs`). Live: qwen3:4b on Ollama found and fixed a bug with grep, read and edit,
  2026-10-03. Falsifier: a run that ends while the last message still carries tool calls.
- [x] AGT-2 — a turn whose calls are all read-only runs them on scoped threads, and results keep
  call order. Evidence: the same test, two `echo` calls. Falsifier: results out of call order, or
  a turn with `write`, `edit` or `bash` running in parallel. The speed gain is not measured.
- [x] AGT-3 — `grep` and `find` respect `.gitignore` and the glob, in-process, with no `rg`
  process. Evidence: `grep_and_find_respect_gitignore_and_glob` (`src/tools.rs`). Falsifier: a
  match from an ignored path.
- [x] AGT-4 — Ctrl-C stops one turn and leaves a conversation that can be sent again. Evidence:
  `a_stop_leaves_a_conversation_that_can_be_sent_again`, `bash_stops_when_the_flag_turns_true`.
  Live: a real SIGINT during `sleep 30` exited 130 in under a second, and `--continue` resumed,
  2026-10-03. Falsifier: a tool call with no tool result after a stop.
- [x] AGT-5 — sessions append one message per line, mode 0600, without the system prompt.
  Evidence: `append_then_load_round_trips_without_the_system_prompt` (`src/session.rs`).
  Falsifier: a session file readable by group or other.

### F1 · The extension contract

The one core change wave 1 needs. Everything after it uses this contract and nothing else.

```rust
trait Extension: Send + Sync {
    fn name(&self) -> &'static str;
    fn tools(&self) -> Vec<Box<dyn Tool>> { vec![] }
    fn system(&self, _prompt: &mut String) {}
    fn before_request(&self, _messages: &mut Vec<Message>) {}
    fn tool_call(&self, _call: &ToolCall) -> Verdict { Verdict::Allow }
}
```

Four hooks, because wave 1 needs exactly these: tools (skills, MCP, diagnostics), system prompt
(skills), request context (compaction) and the tool-call gate (guard). pi's event bus, renderers
and commands are not copied.

- [ ] AGT-6 — `Agent` takes a list of extensions. Their tools join the core tools, `system` runs
  once per run, `before_request` before every completion, `tool_call` before every call. A
  `Verdict::Deny(reason)` becomes the tool result and the call does not run. Falsifier: a denied
  call whose tool `run` executes. Probe: a test extension that denies `echo` and counts runs.
- [ ] AGT-7 — a bare run is the F0 core. With no extensions named, the request body and the tool
  list equal F0's byte for byte. Falsifier: any difference. Probe: a test that compares the two
  request bodies.
- [ ] AGT-8 — `<overlay>/config/agent.toml` `[agent] extensions = [...]` names what runs.
  `--ext none` runs nothing beyond the core, `--ext +name` adds one and `--ext -name` removes one
  for this run. An unknown name is an error at startup, not a silent skip. Falsifier: a run that
  starts with a misspelled extension name.
- [ ] AGT-9 — two extensions that register a tool with the same name stop the startup with both
  names in the error. Falsifier: a run where one tool silently shadows another.

### F2 · Guard (on by default, D3)

A port of `Packs/security/extensions/secrets-guard.ts` (572 lines) to the `tool_call` hook. Its
header lists what it blocks: secret paths on `read` and `edit`, secret patterns in
`grep` and `find`, and `cat`, `bw get`, `env` and similar on `bash`.

- [ ] AGT-10 — every block and allow case listed in the header of `secrets-guard.ts` is a test
  case in the Rust port, with the same verdict. Falsifier: a case where the two disagree. Probe:
  a table test over the header's examples.
- [ ] AGT-11 — the guard runs unless `--ext -guard` or config removes it, and the startup line
  says so when it is off. Falsifier: a run with the guard off and no line saying so.

### F3 · Skills from `Packs/`

- [ ] AGT-21 — sjel-agent is a harness in `tools/lib/harness-registry.ts` with the `registry`
  model. Activating a profile writes its skill paths into `<overlay>/config/agent.toml`, and
  `tools/harnesses status` shows a sjel-agent row per skill (D7). Falsifier: a profile switch
  that leaves `agent.toml` unchanged. Probe: `sjel harnesses use <profile> --harness sjel-agent`,
  then read the file.
- [ ] AGT-12 — the system prompt carries each enabled skill's `name` and `description` from its
  `SKILL.md` frontmatter, and nothing else of it. A `skill` tool returns the body on demand.
  Falsifier: a skill body in the first request. Probe: count `SKILL.md` body bytes in request 1.
- [ ] AGT-13 — a skill directory that fails the frontmatter parse is reported with its path at
  startup and skipped. The run continues. Falsifier: a crash, or a silent skip.

### F4 · MCP client

The one door to tools in another process. Its first target is `tools/sjel-mcp`, which already
carries root `ISA.md` F8–F10 (capability search, a read-only token, pseudonymized reads). So the
agent reaches Sjel's data with no data path of its own.

- [ ] AGT-14 — the MCP servers named in config start over stdio, and their `tools/list` entries
  become tools named `<server>__<tool>`. Falsifier: a listed tool the model cannot call.
- [ ] AGT-15 — a call waits up to the server's configured timeout, default 120 s, because an
  ask-mode write in `sjel-mcp` polls that long while the owner decides (`tools/sjel-mcp/README.md`).
  Falsifier: an ask-mode call that times out before 120 s.
- [ ] AGT-16 — a Ctrl-C during an MCP call returns a stopped result within one second and leaves
  the server running for the next call. Falsifier: a hung turn, or a dead server after a stop.

### F5 · Compaction

- [ ] AGT-17 — when the conversation estimate exceeds the role's `max_input_tokens`
  (`libs/inference`, `ResolvedRole`), the oldest complete turns become one summary message
  before the request. A turn is never split between its tool calls and their results.
  Falsifier: a request with a tool result whose call was compacted away.
- [ ] AGT-18 — the session file records the summary, so `--continue` after a compaction sends
  the summary and not the full history. Falsifier: a resumed request larger than the compacted
  one it continues.

### F6 · Rust diagnostics

- [ ] AGT-19 — a `cargo` tool runs `check`, `clippy`, `test` or `deny` with `--message-format=json` and
  returns one line per diagnostic, `path:line:col level[code] message`, instead of rendered
  compiler output. Falsifier: a diagnostic in the JSON that the tool result omits. Probe: a fixture
  crate with one known error and one known warning.
- [ ] AGT-22 — `stream::fold` holds under `proptest`: for any split of a valid event stream into
  chunks, and for arbitrary bytes, it returns a message or an error and never panics (D8).
  Falsifier: a shrunk input that panics or folds two chunkings differently.
- [ ] AGT-20 — the tool result is smaller than the rendered output for the same build.
  Falsifier: a fixture where it is not. The size is measured on the fixture, not assumed.

## Not yet specified

- **MCP client: `rmcp` or hand-written.** The official Rust SDK is async. The core is blocking
  (C3). Measure `rmcp`'s dependency count against the size of `initialize` + `tools/list` +
  `tools/call` over stdio before choosing.
- **Wave 2.** Named so they are a later decision, not an omission: subagents (pi-subagents,
  20,943 lines today), web access (pi-web-access, 30,925 lines), a full-screen TUI, line editing,
  and `capabilities/assistant` adopting the loop with read-only Sjel tools and no `bash`.

## Test Strategy

| isc | type | check | threshold | tool | anchors_to |
| --- | --- | --- | --- | --- | --- |
| AGT-1 | command | `cargo test -p sjel-agent loop_runs` | pass | cargo | F0 |
| AGT-2 | command | same test, result order | call order | cargo | F0 |
| AGT-3 | command | `cargo test -p sjel-agent grep_and_find` | pass | cargo | F0 |
| AGT-4 | command | `cargo test -p sjel-agent stop` | pass | cargo | F0 |
| AGT-5 | command | `cargo test -p sjel-agent append_then_load` | mode 0600 | cargo | F0, C4 |
| AGT-6 | command | deny extension, count `run` calls | 0 | cargo | F1 |
| AGT-7 | command | bare request body vs F0 request body | equal | cargo | F1 |
| AGT-8 | command | start with `--ext typo` | non-zero exit | cargo | F1 |
| AGT-9 | command | two extensions, one tool name | non-zero exit, both names | cargo | F1 |
| AGT-10 | command | table test over the `secrets-guard.ts` header cases | all equal | cargo | F2 |
| AGT-11 | command | run with `--ext -guard`, read startup line | says off | cargo | F2, D3 |
| AGT-12 | command | `SKILL.md` body bytes in request 1 | 0 | cargo | F3 |
| AGT-13 | command | broken frontmatter fixture | reported, run continues | cargo | F3 |
| AGT-14 | command | fixture MCP server, call each listed tool | all answer | cargo | F4 |
| AGT-15 | command | fixture tool that sleeps 90 s | answers | cargo | F4 |
| AGT-16 | command | stop during a call, then call again | < 1 s, second call answers | cargo | F4 |
| AGT-17 | command | compact a conversation with tool calls | no orphan result | cargo | F5 |
| AGT-18 | command | resume after compaction, compare request sizes | resumed ≤ compacted | cargo | F5 |
| AGT-19 | command | fixture crate, count diagnostics | equal to JSON | cargo | F6 |
| AGT-21 | command | switch profile, read `agent.toml` and `tools/harnesses status` | paths match profile | bun, cargo | F3, D7 |
| AGT-22 | command | `cargo test -p sjel-agent stream` with proptest | no panic, chunking-invariant | cargo | F6, D8 |
| AGT-20 | command | bytes of tool result vs rendered output | smaller | cargo | F6 |

## Anti-claims

- [ ] A1 — no extension runs without being named, except the guard. Falsifier: a bare run whose
  tool list or system prompt differs from F0 (AGT-7 is the probe).
- [ ] A2 — the assistant capability never gets `bash`, `write` or `edit`. Falsifier: any of them
  in the assistant's tool list once it adopts the loop.
- [x] A3 — no session data is written inside the repository. Evidence: the only session path is
  `<overlay>/data/agent/sessions/` (`src/main.rs`), 2026-10-03. Falsifier: a `.jsonl` under the
  repository after a run.

## Decisions

- **2026-10-03 — the slim core's shape** (built in this session). OpenAI chat completions only.
  Blocking HTTP through `sjel-http`. `grep` and `find` in-process on ripgrep's crates, not a
  subprocess (`upstreams.toml [ripgrep]`). Ctrl-C through `signal-hook`, because the workspace
  denies `unsafe` (`upstreams.toml [signal-hook]`). Linear JSONL sessions, no tree.
- **2026-10-03 — D1: extensions are Rust crates behind one trait** (principal's call, crystallize
  round). Each official extension is a workspace crate implementing `Extension` (F1), compiled
  into the binary. Chosen over a subprocess protocol, because that reopens the third-party door
  `README.md` closes and charges IPC on every hook. Chosen over modules inside `libs/agent`,
  because the assistant would link every extension. MCP is the one door to out-of-process tools.
- **2026-10-03 — D2: compiled in, off until named.** One binary. `agent.toml` and `--ext` decide
  what runs. Not cargo features alone, because changing the set should not need a rebuild.
- **2026-10-03 — D3: the guard is on by default.** The single exception to D2. Its absence can
  send a credential to a cloud model, and opt-in means the first session without it is the one
  that leaks. It stays visible: AGT-11.
- **2026-10-03 — D4: wave 1 is skills, MCP, compaction, Rust diagnostics, plus the guard.**
  Subagents, web access and a TUI wait for wave 2 (Not yet specified).
- **2026-10-03 — D5: a Rust-specialist coding agent** (PRD Q5). The loop stays general. The
  product is the Rust discipline the extensions add: the `effective-rust` skill through F3 and
  clippy findings through F6. Miri is not a focus, because the workspace denies `unsafe`.
- **2026-10-03 — D6: `tools/sjel-agent/` holds the binary, extensions are its modules** (PRD
  Q7). Forced in part: a binary inside `libs/agent` that depends on extensions which depend on
  `libs/agent` is a cycle Cargo rejects. Not a `sjel-cli` subcommand, because that crate has no
  network stack and the launcher rebuilds it in release mode. Extensions start as
  `src/ext/*.rs`, per `on-placement.md` (`libs/` needs more than one consumer), and move to
  `libs/` when the assistant needs one. Refines D1's layout, not its substance.
- **2026-10-03 — D7: sjel-agent is the fifth harness, registry model** (PRD Q8). AGT-21.
- **2026-10-03 — D8: F6 uses installed tools plus `proptest`** (PRD Q9). `check`, `clippy`,
  `test`, and `cargo deny` once a `deny.toml` exists. No `toolchain.toml` change. AGT-22.
- **2026-10-03 — this plan lives here, not in the Axon PRD.** `PRD Axon.md` in the vault was frozen
  2026-09-26: "Open work lives in the repo's `ISA.md` files. Nothing new is recorded here." The idea got
  its own vault PRD the same day (header), because an ISA records claims, not why they exist.

## Log

- 2026-10-03 · Built the slim core: loop, six tools, streaming, parallel read-only calls,
  sessions, Ctrl-C. 13 tests, clippy clean, three live runs against qwen3:4b on Ollama.
- 2026-10-03 · Planned F1–F6 in a crystallize round. D1–D4 recorded above.
