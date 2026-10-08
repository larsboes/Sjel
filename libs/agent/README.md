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
crate compiled into this binary, doing nothing until it is named. The contract is five hooks in
[`src/extension.rs`](src/extension.rs) — the tools it adds, the system prompt, the request
context, the gate in front of every tool call, and the rewrite applied to what a tool returned
before the model reads it. That last one touches the model's copy only: the trace on your own
terminal keeps the raw text.

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
than one that does not start.

The guard is compiled in and on by default — the one exception to "off until named", because its
absence is what puts a credential in a request. `--ext -guard`, `--ext none`, or an `agent.toml`
that names a set without it, removes it, and the startup line says so when it is gone. It refuses
secret paths on `read`, `edit`, `grep` and `find`, and any `bash` command that names one unless
it is an allowlisted shape (`source .env && …`, `export VAR=…`, `bw unlock`, writing a reference).
It also scrubs what `bash`, `read` and `grep` returned before the model reads it.

It carries the two tools that make a refusal actionable, because a guard that only says no leaves
a run with no way to do legitimate work: `vault_exec` runs a command with an env file loaded and
replaces every value it loaded with `****` in the result — by value, by shape and by length — and
`vault_keys` lists the names in such a file without the values. Pass `keys` to `vault_exec` to put
only the variables the command needs into its environment; without it the whole environment and
every variable in the file are inherited, which is the risky default and what the tool's own
description says.

## Skills

The `skills` extension is not on by default: it offers what `[skills] paths` names.

```toml
[agent]
extensions = ["guard", "skills"]

[skills]
paths = [
  "~/Developer/Sjel/Packs/coding/skills/effective-rust",
  "~/Developer/Sjel/Packs/writing/skills/human-writing",
]
```

Each named skill contributes its `name` and its `description` to the system prompt — one line
each — and nothing else of its `SKILL.md`. The document arrives when the model calls the `skill`
tool with that name, which is the point: twenty skills cost twenty lines, not twenty documents. A
path that will not read, or whose frontmatter the Pack engine would refuse, is named at startup
and skipped, and the run goes on.

Everything else in wave 1 — MCP, compaction, Rust diagnostics — is still to come.

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

The extension contract is in place (F1), the guard uses all of it including the fifth hook (F2,
D9), and skills run on it (F3). Next: MCP, compaction and Rust diagnostics. What is not done is
letting a profile switch fill `[skills] paths` for you (AGT-21, in `tools/sjel-cli`). Line
editing, a full-screen TUI and subagents are named in [`ISA.md`](ISA.md) as wave 2, not decided.

A Ctrl-C while the model reads a long prompt and streams nothing takes effect at the first
chunk. Press it twice to exit instead.
