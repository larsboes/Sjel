# libs/agent

One agent loop. It sends the conversation to a model, runs the tool calls the model asks for,
appends the results, and repeats until the model answers without a tool call.

A shared library with two intended consumers. The coding CLI in `src/main.rs` is a minimal
pi alternative. `capabilities/assistant` is the second consumer and does not use it yet. Each
consumer passes its own tool list. The loop does not change between them.

## Model and wire format

The model comes from a `sjel-inference` role (`libs/inference`). The CLI uses the role
`coding` unless `--role` names another. Requests use streamed OpenAI chat completions with
`tools`, which oMLX, Ollama's `/v1` shim and every hosted backend in the overlay accept. The loop
forwards `chat_template_kwargs` and `request_overrides` from the role, the same pass-through
as `capabilities/assistant/src/main.rs`.

HTTP is blocking, through `sjel-http`. An async caller runs `Agent::run` inside
`spawn_blocking`, which is the pattern `libs/sjel-http/Cargo.toml` states for the workspace.

The response streams. `src/stream.rs` folds the server-sent events into one message and
passes each text fragment to the front end as it arrives. Fragment strings are borrowed from
the line buffer, so most of them reach the terminal without an allocation.

## Coding tools

`tools::coding(root)` returns `read`, `write`, `edit`, `bash`, `grep` and `find`, the same
set pi gives a model. Two choices differ from pi:

- `grep` and `find` run in-process on ripgrep's own crates (`ignore`, `grep-searcher`,
  `grep-regex`). There is no process per call. The walk uses every core, respects
  `.gitignore`, and stops when the 30 KB output budget is spent.
- `read`, `grep` and `find` declare themselves read-only. When every call in one model turn
  is read-only, the loop runs them on scoped threads. A turn that reads five files waits for
  the slowest read, not the sum.

The file tools are not confined to `root`. `bash` can reach any path the process can, so a
path check on the other tools would not stop anything. The assistant must never get these
tools.

## Extensions

The core is one tool set. Everything beyond it is an official extension: a first-party Rust
crate compiled into this binary, doing nothing until it is named. The contract is four hooks in
[`src/extension.rs`](src/extension.rs) — the tools it adds, the system prompt, the request
context, and the gate in front of every tool call.

`<overlay>/config/agent.toml` names the set a run starts with, and `--ext` changes it for one
run, in the order the flags are given:

```toml
[agent]
extensions = ["guard", "skills"]
```

```bash
sjel-agent --ext none "..."       # the core only
sjel-agent --ext +skills "..."    # add one
sjel-agent --ext -guard "..."     # remove one
```

A name without a sign is an error rather than a guess, and a name this binary does not carry
stops the run at startup: a session quietly missing the extension the operator asked for is worse
than one that does not start. Nothing is compiled in yet — `BUILT_IN` in `src/main.rs` is `&[]` —
so every name currently stops the run, and the guard (F2) is the first to land.

## Run it

```bash
cargo build -p sjel-agent
cd <project> && sjel-agent "fix the failing test in foo.rs"   # one prompt, then exit
cd <project> && sjel-agent                                     # one prompt per line, /exit stops
cd <project> && sjel-agent --continue                          # resume the newest session here
```

The answer streams to stdout. Thinking, tool calls and their results go to stderr.

Ctrl-C stops the current turn and keeps the conversation. Text that streamed before the stop
stays, a running `bash` command is killed with its process group, and a tool call that never
ran gets a result that says so. A second Ctrl-C exits with 130. Any other failed turn is
removed from the conversation, so the prompt can be sent again.

## Sessions

Every turn is appended to
`<overlay>/data/agent/sessions/<working directory>/<millis>.jsonl`, one message per line,
mode 0600. A session can hold any file the model read, which is why it lives in the private
overlay and not in the repository. The system prompt is not stored. A resumed session gets
the current one, so an edited `AGENTS.md` takes effect. `--continue` takes the newest session
for the working directory. `--session <file>` resumes or starts a named file.

Add a `coding` role to the overlay's `config/inference.json` first:

```json
"coding": { "backend": "omlx", "model": "gemma-4-26b-a4b-it-4bit" }
```

## Not here yet

The extension contract is in place (F1, [`ISA.md`](ISA.md)); nothing uses it yet. The guard is
first, then skills, MCP, compaction and Rust diagnostics. Line editing, a full-screen TUI and
subagents are named there as wave 2, not decided.

A Ctrl-C while the model reads a long prompt and streams nothing takes effect at the first
chunk. Press it twice to exit instead.
