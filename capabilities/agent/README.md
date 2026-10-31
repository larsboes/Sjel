# agent

The coding agent as a capability: one loop from `libs/agent`, a plugin tool set, and the
sessions, stream and approvals a front end needs. Loopback only. Every caller goes through the
same gates every other capability uses — the agent token, the owner's per-capability mode, the
approval files and the call log (`libs/sjel-server/src/agent*.rs`).

A Mac client (`apps/gpui`, finder ⌘K) and a phone both drive the same run.

## Why this shape: a capability, not a route on `assistant`

[`libs/agent/README.md`](../../libs/agent/README.md) states the rule this capability exists to
respect: the file tools are not confined to a root, `bash` can reach any path the process can, and
*"the assistant must never get these tools."* The assistant answers questions with read-only Sjel
tools. This is a second front end with its own token scope, its own mode and its own port, and the
assistant's tool list does not move.

The alternative — linking the loop into `apps/gpui` — was considered and declined. It would put
`bash` in the app process, which is the process the operator is looking at, and it buys nothing:
the tools run on this machine either way, so the HTTP hop carries a prompt and a stream, never file
contents. What it costs is a second copy of everything `libs/sjel-server` already owns, written
only for the phone.

## Why this shape: the loop is unchanged

`Agent::run(&self, &mut Vec<Message>, on: impl FnMut(Event<'_>))` is blocking and streams
`Event::{Text, Reasoning, ToolCall, Usage, ToolResult}`. That five-variant enum is the stream
protocol; the SSE handler maps it one to one and invents no vocabulary. `Agent::run` runs under
`spawn_blocking` and the closure pushes into a channel.

Per-tool-call approval needs no change to the loop either. The gate is
`Extension::tool_call(&self, &ToolCall) -> Verdict`: synchronous, and it runs before the arguments
are parsed, so a card can name a command without trusting its JSON. A gate that blocks on a human
is legal because that thread is already blocking.

## Why this shape: the plugins are the extension contract

The `[agent]` section of `<overlay>/config/agent.toml` names the set, and `Extension`
(`libs/agent/src/extension.rs`) has the five hooks that carry them. Nothing here is a new plugin
system.

| Plugin | Tools | Where |
|---|---|---|
| `fs` | `read write edit bash grep find ls` (`tools::coding(root)`) | In-process ripgrep, 30 KB output budget, 500-entry listings, read-only calls in parallel |
| `sql` | `sql` | `src/sql.rs`: one read-only query on the shared database, instead of `bash` + `sqlite3` |
| `sjel` | `sjel_health`, `calendar_entries`, `projects_list`, `project_records`, `trips_plans` | `libs/agent/src/ext/sjel.rs`, the set `capabilities/assistant` passes |
| `guard` | `vault_exec`, `vault_keys` | `libs/agent/src/ext/guard.rs`, on by default |
| `compaction` | — | `libs/agent/src/ext/compaction.rs` |

`fs` and `sql` are the two that cannot live in `libs/agent`: `fs` is the core tool set rather than
an extension, and `sql` would pull `rusqlite` into every consumer of the library, including the
CLI. `sjel` is pure HTTP over `sjel-http`, which the library already carries.

The root is session data, not the capability's identity: a session names the working directory it
was started in, `~/Developer/Sjel` by default, and its conversation is filed under
`<overlay>/data/agent/sessions/<root>/`.

## Routes

Every session route is under `/api/agent/sessions`; the shell proxies the capability as `/agent/…`.

| Route | Purpose |
|---|---|
| `POST /api/agent/sessions` | Start a session: `root` (absolute, `~/…` expanded) and `plugins` |
| `GET /api/agent/sessions` | Every session on this machine, newest first |
| `GET /api/agent/sessions/{id}` | One session: root, plugins, turns, whether a run is active |
| `GET /api/agent/sessions/{id}/transcript` | The stored turns |
| `POST /api/agent/sessions/{id}/prompt` | One prompt; the run starts behind the stream |
| `GET /api/agent/sessions/{id}/stream?from=<seq>` | Server-sent events, resumable by sequence |
| `POST /api/agent/sessions/{id}/approve` | `approval_id`, `allow`, `scope` = `once` or `session` |
| `POST /api/agent/sessions/{id}/steer` | A message for the next model request |
| `POST /api/agent/sessions/{id}/stop` | Ends the turn and keeps the conversation |

A terminal is a separate surface, under `/api/agent/terminals`, because it is a shell rather than
a run of the loop. See [Terminals](#terminals).

One event per frame, because a server-sent event carries one `data:` line and batching JSON
objects into one would be cut at the first newline by every client that reads the protocol.

A tool call that needs a decision pauses the run and emits `needs_approval` with an id. The record
is written by `PolicyFiles::request_approval` under `secrets/agent/approvals/`, so it appears on
the Systems page beside every other approval, and `decide` is the same code path. The gate **polls
the file** rather than waiting on a channel: an owner who presses Allow on the Systems page and one
who presses it in the app write the same record, and a resumed run reads the file instead of holding
state a restart would lose. A read-only call never asks — without that exemption a five-file turn
would be five prompts, and an operator who answers five prompts an hour learns to press Allow
without reading.

Three consequences the front end carries: the stream stays open while a human decides (a keep-alive
comment every interval, `KeepAlive::default()` in axum), a resumed stream asks for `from=<seq>` and
gets `truncated` with the oldest retained `first_seq` when its cursor falls behind, and a
session-wide grant is keyed by tool **and arguments**, so allowing `cargo test` does not allow
`cargo publish`.

## Terminals

A real pty on this Mac, so Claude Code, Codex, Pi or any other CLI agent runs here and is driven
from the phone. The transport is the session stream's: JSON requests, and Server-Sent Events for
output, signed by the paired device.

| Route | Body or query | Answer |
|---|---|---|
| `POST /api/agent/terminals` | `{preset, cwd, cols, rows}` | `{id}` |
| `GET /api/agent/terminals` | | one row per terminal: `id, preset, title, cwd, started_at, last_output_at, alive, exit_code, clients` |
| `GET /api/agent/terminals/options` | | `{presets, folders}`: preset names, then `$HOME` and the folders in `~/Developer`, newest first |
| `GET /api/agent/terminals/{id}/stream?from=<offset>` | | events `output` `{offset, data}`, `truncated` `{first_offset}`, `exit` `{code, signal}` |
| `POST /api/agent/terminals/{id}/input` | `{data}`: base64, at most 64 KiB | `204` |
| `POST /api/agent/terminals/{id}/resize` | `{cols, rows}` | `204`; the last resize wins |
| `DELETE /api/agent/terminals/{id}` | | `204` after SIGHUP to the process group, SIGKILL after 5 s |

- **Presets** come from `[terminal.presets]` in `<overlay>/config/agent.toml`, where the value is
the command line. Absent, the five are `shell` (the login shell), `claude`, `codex`, `pi` and
`sjel-agent`. Every preset except `shell` runs inside `$SHELL -l -c "<command>"`, so `PATH`
matches an interactive terminal.
- **Environment** is an allowlist: `HOME`, `USER`, `LOGNAME`, `PATH`, `LANG`, `LC_ALL`, `TMPDIR`,
`TZ`, `SHELL`, `SSH_AUTH_SOCK`, plus `TERM=xterm-256color` and `COLORTERM=truecolor`. The builder
inherits the whole process environment by default, and the capability's carries the overlay root
and the agent's tokens, so it is cleared first.
- **Scrollback** is a 2 MiB ring of raw output per terminal, with a byte offset on every frame. A
reader resumes from its last offset; an offset older than the ring gets one `truncated` frame with
the oldest offset still held, and the stream stops there, so a client cannot keep drawing on a
grid it never cleared.
- **Ending.** A `DELETE` signals the session's process group with SIGHUP, waits up to five seconds,
then SIGKILLs the group. A terminal with no client attached and no output for twelve hours is
closed the same way. At most eight are alive at once.
- **The audit** is one line per start and per end in `<overlay>/data/agent/terminals.jsonl`:
`{at, id, preset, cwd, client, event, exit_code}`. No input and no output is recorded.

### Why an agent token cannot reach a terminal

A keystroke is a command, so there is no argument for the policy to classify and no card a human
could read first. The gate therefore refuses the agent token on every terminal route — 401, as if
the token were unknown here (`AgentRoutes::closed`, `libs/sjel-server/src/agent_policy.rs`) — and
`every_terminal_route_is_closed_to_an_agent_token` reads the manifest rather than restating the
list, so a terminal route added without closing it fails the test instead of opening a shell.

## Run

`service.toml` declares port 8108 and `target/release/agent-server`. `tools/service-runner.sh
start agent` runs it, and `restart agent` picks up a new build; a restart ends every terminal. It
needs a local `coding` role in the overlay's `config/inference.json`, or `/ready` answers 503 and
says which role is missing.

## What is built

The capability, its routes, the sessions, the stream with resume, the approval gate and the plugin
set. `apps/gpui` drives it: `core/src/agent.rs` is the desktop client, `src/agent_run.rs` holds the
run state, and the finder's fourth mode (`Auto | Ask | Search | Agent`) draws the transcript and
approval card, including `write` and `edit` diffs. The docked desktop seat keeps the run visible
after the finder closes. The paired iPhone can create or select sessions, prompt or steer, follow
and resume streams, inspect and answer approvals, and stop runs through its signed device client.

The terminal has a client on the Mac. Pty sessions, the routes, the ring, the resize, the group
kill and the audit are here (`src/terminal.rs`); a Mac shell has been driven through them end to
end; and the app's half is `apps/gpui/core/src/terminal.rs` (the wire client, with the resume
offset) and `apps/gpui/ui/src/terminal.rs` (the view: `alacritty_terminal` for the grid, keys as pty
bytes, the key row a phone needs). On the Mac, `⌘K › Terminal` or `sjel://terminal` opens the newest
live terminal — or a login shell — in the right edge.

The phone cannot use that client yet: it reaches the Mac through `DeviceClient`, not `Shell`, so its
seat says so rather than opening a shell it could not follow. `docs/plans/agent.md` §6 names what
that costs: `streaming_get` already exists, `DELETE` does not, and the Face ID gate has no binding
in `ios/host`.

The join between the agent UI and the capability is not tested: each part has its own tests, and a
run has never been driven end to end through the capability. `docs/plans/agent.md` (Live
validation) lists what to do in which order, and what a session picking it up needs to know first.

## What is not built

- **The phone's terminal.** Its client is written against `Shell` and the phone speaks
  `DeviceClient`, so the seat reports that instead of opening one. The Face ID gate before a
  reattach has no `LocalAuthentication` binding in `ios/host` yet.
- **Live integration validation.** Work Package 22 still needs a real desktop run and paired-phone
  checks for start, follow, reconnect, transcript recovery, approval from both clients, deny, stop,
  and re-pair. A host-target compile is not evidence that the phone flow works on-device.
- **The container runtime.** `capabilities/agentbox` behind the same session API, once the
  model-endpoint question its README records is answered.

## Ownership boundary

Runs coding tools on the paths it is given, so it is the one capability whose tools are not limited
to Sjel's own data. `Reach::Loopback` always: the phone arrives through `tailscale serve` identity
or a paired device, never through a wide bind. Sessions are c3 — file contents, commands and their
output — so they are never sent to a cloud role; `Agent::run_public` refuses tools outright, and
that is the only reason this surface cannot leak a private value to a provider.

## Considered and declined

- **A coding route on `capabilities/assistant`.** The stream, steer and stop plumbing is already
  there, which is what makes it tempting. It contradicts the README rule directly and puts `bash`
  behind the port the dashboard already proxies.
- **The loop inside `apps/gpui`.** Declined above: no phone path, and a second copy of the gates.
- **`capabilities/agentbox` as the runtime.** A container is a stronger fence than the guard, and
  agentbox already runs one. It also records the price — `--network none` takes the model with it,
  so the box needs a route to the host's model server, which is the boundary it was adopted to
  remove. Available as a later runtime behind the same session API; not the first slice.
- **Spawn `sjel-agent` as a child process.** Reuses the CLI unchanged, but replaces one stream with
  a stdio protocol plus a supervisor, and the phone would need both hops.
