# The Sjel agent

A coding and operations agent written in Rust. It runs on the Mac, and the user controls it
from the GPUI app on the Mac or the paired iPhone. The loop is `libs/agent`. The capability that
serves it is `capabilities/agent`. The reasoning for that shape is in
[`capabilities/agent/README.md`](../../capabilities/agent/README.md).

Lane B in [`PLAN.md`](../../PLAN.md), the top priority (2026-10-25). Beside the Sjel
agent, the phone gets a terminal on the Mac (§6), so Claude Code and every other CLI agent run
there too.

Order:

1. The native terminal in the GPUI app (§6).
2. Tiers and command policy (§2, §3).
3. Live validation on Mac and iPhone (§7).
4. Plugins and the Pi port, all web providers (§4).
5. The measured comparison with Pi (§8).

## 1. What exists

- **Loop and CLI.** `libs/agent`: the core tools (`read`, `write`, `edit`, `bash`, `grep`,
  `find`), sessions in the overlay, steering, cloud calls with a data-class check.
- **Extensions in `libs/agent/src/ext/`.** `guard` (secrets, on by default), `web`, `subagents`,
  `mcp`, `skills`, `rust`, `graph`, `sjel`, `compaction`.
- **Capability.** `capabilities/agent` serves sessions, prompts, steering, a resumable event
  stream, approvals and stop, on `Reach::Loopback`. The phone reaches it through the paired-device
  listener.
- **Approval gate.** `capabilities/agent/src/gate.rs`. Every call that is not read-only asks. The
  card shows a real diff for `edit` and `write`. A decision is a file in
  `secrets/agent/approvals/`, so the Systems page, the Mac and the phone show one record. A
  session grant is keyed by tool and exact arguments. An unanswered call is denied after 15
  minutes.
- **Clients.** The Mac finder has an `Agent` mode, and the run lives in the right-edge seat. The
  iPhone lists and creates sessions, prompts, steers, follows streams, approves, denies and stops.
  Both desktop Finder and iPhone agent seat support session tier selection (`sjel | project | account`)
  and annotate gate approval cards with policy rule ID, displaying a distinct red border and single
  override button for blocked commands (no session-wide grant).

**Gap.** The capability assembles only `fs, graph, sjel, sql, guard, compaction`
(`capabilities/agent/src/plugins.rs`). `web`, `subagents`, `mcp`, `skills` and `rust` exist in
the library but no session can use them.

## 2. Access tiers

Decided 2026-10-25. A session has one tier. The tier sets the plugin set and the scope
that a write or a command may touch.

| Tier | For | Plugins | Write and command scope |
|---|---|---|---|
| `sjel` | Questions and actions on Sjel's own data | `sjel`, `graph`, `sql` (read-only), capability actions through their routes | No shell. No file writes. |
| `project` | Coding in one checkout | Above plus `fs`, `bash`, `rust`, `skills`, `web`, `subagents` | The session root (default `~/Developer/Sjel`) |
| `account` | Work across the user account | Same as `project` | `$HOME`. `sudo` stays blocked. |

The tier is set when the session starts and does not change (A3). A higher tier needs a new
session. This keeps a grant from one tier out of another. The phone may start a session in any
tier (A4).

Built (bc1ec6bc): `capabilities/agent/src/tier.rs` holds the three tiers, each naming its plugin
set and the directory it reaches. `POST /api/agent/sessions` takes a `tier`, defaults to
`project`, refuses a name it does not carry, refuses a `project` or `account` root outside
`$HOME`, and accepts `plugins` only as a narrowing of the tier's own set — so a client cannot
widen the session it is given. The tier and the reach it grants are written into the session's
meta, and a session read back from disk never derives a wider one. Open: the picker in §5, and
`rust`, `skills`, `web` and `subagents` join the coding tiers when §4's port reaches them into
this capability; `plugins::KNOWN` names what exists today.

## 3. Command policy

Decided 2026-10-25. Every `bash` call and every write gets one of three classes.

| Class | Behavior |
|---|---|
| Block | Refused. The user can allow one call from a separate card that names the matched rule. A session grant is never possible. |
| Ask | The approval card, as today. A session grant is possible, keyed by tool and exact arguments. |
| Allow | Runs without a card. Logged like every call. |

### How a command is classified

1. **Split.** Lex the command with shell quoting rules. Split it into simple commands at `;`,
   `&&`, `||`, `|`, `&` and newlines. A subshell `( … )` or a group `{ … }` is classified by its
   contents.
2. **Unwrap.** Before a rule matches, strip the wrappers that run another command: `env [VAR=x]…`,
   `command`, `builtin`, `exec`, `nice`, `nohup`, `time`, `timeout <n>`, a leading backslash
   (`
m`), and an absolute path to a known binary (`/bin/rm`). Classify `bash -c`, `sh -c` and
   `zsh -c` strings recursively. `xargs <cmd>` and `find … -exec <cmd>` are classified as `<cmd>`
   with unknown arguments.
3. **Opaque input.** A command whose program name comes from a substitution (`$(…)`, backticks,
   `$VAR`), or `eval`, is at least Ask. If the opaque part reaches a Block program name, it is
   Block. A here-document fed to a shell is classified by its body.
4. **Paths.** Resolve every path argument and every redirect target (`>`, `>>`, `2>`, `tee`)
   against the working directory, with `~` and `..` expanded and symlinks followed. A write
   inside the tier scope (§2) or outside `$HOME` is Allow. A write under `$HOME` but outside the
   tier scope is Ask (A1).
5. **Strictest wins.** The class of a compound command is the strictest class of its parts.
6. **Unknown is Allow.** A program with no rule is Allow (R4). Only the harmful classes above
   ask or block: history, remotes, packages, processes, destruction and privilege.
7. **Log.** Every decision is written to the call log with its class and the rule ID.

### Rules

| ID | Class | Matches |
|---|---|---|
| B1 | Block | `rm` with `-r`, `-R`, `--recursive` or `-f`, or any `rm` whose target is the tier root, `$HOME`, `/`, or outside the tier scope. `find … -delete`. |
| B2 | Block | Discarding uncommitted work or history: `git reset --hard`, `git clean` with `-f`, `git checkout -- <path>` and `git restore` on the worktree, `git push` with `-f` or `--force`, `git push --delete`, `git branch -D`, `git tag -d`, `git filter-repo`, `git reflog expire`. |
| B3 | Block | Privilege: `sudo`, `su`, `doas`, `osascript` with `administrator privileges`. |
| B4 | Block | Download and run: a pipe or process substitution into `sh`, `bash`, `zsh`, `python`, `node` or `ruby` (`curl … \| sh`, `bash <(curl …)`). |
| B5 | Block | Disks and system state: `dd`, `mkfs*`, `diskutil`, `fdisk`, `shred`, `csrutil`, `spctl`, `nvram`, `launchctl`, `systemctl`, `crontab -r`, `defaults write`, `chmod -R`, `chown -R`. |
| B6 | Block | Secrets: the existing guard (`libs/agent/src/ext/guard.rs`), plus `security find-*-password -w` and `security delete-*`. |
| B7 | Block | Irreversible publishing: `cargo publish`, `npm publish`, `bun publish`, `gh release create`, `gh repo delete`. |
| A1 | Ask | A write through the shell or the `write` and `edit` tools whose target is under `$HOME` but outside the tier scope. |
| A2 | Ask | Git changes to history, branches or remotes: `commit`, `push` (no force), `pull`, `merge`, `rebase`, `cherry-pick`, `revert`, `am`, `apply`, `stash` (save, pop, apply), `reset` (soft or mixed), `switch`, `checkout` of a branch, `branch` and `tag` create, rename or delete, `worktree`, `submodule`, `remote` changes, `config` writes, `gc`, `prune`, `notes`, `bisect`, `update-index`. |
| A3 | Ask | Package changes and download-and-run: `brew`, `npm`/`pnpm`/`yarn`/`bun` `install`, `add`, `remove`, `update` and `link`, `pip install` and `uninstall`, `pipx`, `uv` `add`, `remove`, `sync`, `pip` and `tool`, `cargo install`, `add`, `update` and `remove`, `rustup` changes, `apt`, `dnf`, `snap`, `flatpak`, `gem install`, `go install` and `go get`, and `npx`, `bunx`, `uvx`. |
| A4 | Ask | Network writes and remote shells: `curl` or `wget` that send data (`-d`, `--data*`, `-F`, `-T`, a `-X` other than `GET` or `HEAD`), `ssh`, `scp`, `sftp`, `rsync` to a remote, `nc`, `telnet`, `ftp`, and `gh` write subcommands. |
| A5 | Ask | Processes and containers: `kill`, `pkill`, `killall`, and `docker`, `podman` and `orb` commands other than reads. |
| A6 | Ask | Opaque input (step 3): a program name built at run time, `eval`. |
| R1 | Allow | Reading: `ls`, `cat`, `rg`, `grep`, `find` without `-delete`, `curl` and `wget` reads, and the like. |
| R2 | Allow | Git reads and local index changes: `status`, `diff`, `log`, `show`, `blame`, `branch` (list), `remote -v`, `fetch`, `add`, `rm`, `mv`, `restore --staged`, `clone`, `init`. |
| R3 | Allow | Build and test: `cargo` other than A3, `bun test`, `bun run`, `npm run`, `tools/doctor`, `sjel` read verbs. |
| R4 | Allow | Every program without a rule, interpreters running a named script or `-c` code, here-documents, `source`, `sed -i` and `perl -i`, plain `rm` of a file inside the tier scope, and writes per step 4. |

### Where it lives and how it is tested

- `libs/agent/src/ext/policy.rs`: the classifier, so the CLI and the capability share one rule
  set. `capabilities/agent/src/gate.rs` reads the class: Allow skips the card, Ask shows it, and
  Block shows the separate card with the rule ID.
- `Packs/security/extensions/command-policy/cases.toml`: the one table both implementations are
  held to — a case per rule ID, the evasions that must never be Allow (`r''m -rf x`, `\rm -rf
  x`, `env rm -rf x`, `command rm -rf x`, `$(echo rm) -rf x`, `bash -c "rm -rf ~"`, `echo x |
  xargs rm -rf`, `find . -exec rm -rf {} +`, `ls; rm -rf x`), and the `[[write]]` cases. Each is
  written against a fixture scope — a synthetic `$HOME`, a project directory inside it, and the
  temp directory writable — so a case means the same on every machine.
  `tools/command-policy.test.ts` runs it against the pi extension,
  `libs/agent/tests/policy_cases.rs` against the Rust one.
- Done when every case passes, the gate shows the rule ID on each card, and one week of real use
  gives a prompt rate (below).

**Pi has it now (2026-10-26).** `Packs/security/extensions/command-policy/` implements this section
for pi in TypeScript, with the case table in `command-policy/cases.toml` (every rule ID and every
evasion case above). It implements the rule table above: cards are reserved for harmful classes,
so writes inside the scope, unknown programs and interpreters run without one.

**The Rust classifier is the same table.** `Policy` reads a fixture's home directory and writable
roots (`with_home`, `with_writable`) instead of the machine's, and
`libs/agent/tests/policy_cases.rs` loads the shared table, so neither language has a copy of the
rules to drift from. All 115 command cases and 5 write cases pass in both. The second table
(`libs/agent/tests/policy_cases.toml`) is deleted. `capabilities/agent/src/gate.rs` says what the
three classes now cost a run — a read, a build and an in-scope write run without a card — and its
approval fixture asks through a redirect whose target is computed at run time (A1), which is the
write that still reaches a card.

A local small model may classify A6 commands later, only after a measurement shows it lowers the
prompt rate without a missed Block. `Packs/harness/extensions/clm-classifier.ts` is the prior art.

Built (6622b21e): `libs/agent/src/ext/policy.rs` holds the classifier and the rule table, and
`capabilities/agent/src/gate.rs` reads the class: Allow runs without a card, Block is refused
before any card, Ask is the card as before, and each decision is logged with its rule ID on the
call log. Open: §5's separate Block card that can allow one call, the A6 classifier for unknown
programs, and the measures below, none of which have run.

**Measures for the policy.**

- **Prompt rate.** Approvals in `secrets/agent/approvals/` divided by lines in
  `secrets/agent/calls/<YYYY-MM>.jsonl` over one month. Confirmations must stay rare enough to be
  read (Akhawe and Felt, cited in `research/`). Set the threshold after the first month.
- **No way around the gate.** An agent session never reads unredacted data or secrets except
  through the gate. Open check: in a new session, a `Read` of `secrets/**` is refused by the
  user-level configuration floor.
- **Cloud fields.** A cloud request carries only the fields its task needs (Staab et al.). The test
  `cloud_derivative_carries_only_fields_task_needs_answering_staab_et_al` covers it.

**The classifier is not a sandbox.** Shell indirection is unbounded, the same argument
`libs/agent/src/ext/guard.rs` makes for secret paths. A command can write outside the root
without naming the path. The hard boundary for `project` and `account` is decision A1 in §10.


## 4. Plugins and the Pi port

The goal is to stop depending on Pi's TypeScript packages for daily work. Pi stays until the
measured gate in §8 passes (decided 2026-10-24).

| Pi package | Rust today | Work |
|---|---|---|
| `pi-web-access` (97 files, about 15 search providers, page curation, PDF extraction) | `ext/web.rs`: DuckDuckGo search and `web_fetch`, SSRF guard | Port all of them (A2, §10). DuckDuckGo stays the keyless default. |
| `pi-subagents` | `ext/subagents.rs` | Expose in the capability. Check parity with the agent files in `~/.pi/agent/agents/`. |
| `secrets-guard.ts` | `ext/guard.rs`, with `vault_exec` and `vault_keys` | Done |
| `inference-keys` | `sjel provider` and keychain credentials | Check parity |
| `clm-classifier.ts` | none | Input for §3 rule 3 |
| `questions.ts` | none | Decide if the agent needs it |

Then add `web`, `subagents`, `mcp`, `skills` and `rust` to `plugins.rs` `KNOWN`, gated by tier.

## 5. Phone and Mac experience

- A tier picker when a session starts.
- The Block card looks different from the Ask card and needs a deliberate action.
- A notification when a run waits for approval. It carries no tool output and no file contents.
- Later: the run drives the main column, see [`gpui.md`](gpui.md#5-finder-and-right-edge).

## 6. Terminal from the phone

Decided 2026-10-25: the phone gets a real terminal on the Mac over the tailnet. Claude
Code, Codex, Pi or any other CLI agent then runs on the Mac and is driven from the phone. The Sjel
agent keeps its native UI. Every other agent uses the terminal.

Facts (2026-10-25): Tailscale SSH is off on this Mac. The node advertises no SSH host keys, and
`capabilities/host-net` only reports the `RunSSH` flag. tmux is not installed. No PTY or
terminal-emulator crate is in the workspace; `portable-pty` is now (see `upstreams.toml`). Decided
2026-10-25: build the native terminal first. The SSH path is the fallback, not a first step.

Steps:

1. **PTY sessions on the Mac** in `capabilities/agent` (A5), behind the routes below. Built
   (`833c571d`): `capabilities/agent/src/terminal.rs`, with `[terminal.presets]` in
   `<overlay>/config/agent.toml` and the audit in `<overlay>/data/agent/terminals.jsonl`. What is
   left from this step: nothing.
2. **One terminal view for Mac and phone**, specified below. Built for the Mac: `⌘K › Terminal`
   or `sjel://terminal` opens the newest live terminal — or a login shell — in the right edge
   (`Seat::Terminal`), and the view wires keys, the wheel, resize and the End button. Built for
   the phone: the same seat opens the same terminal through `DeviceClient`, behind Face ID. Not
   tried on a paired phone yet.
3. **Fallback.** If the native terminal does not work, use an SSH path instead: Tailscale SSH or
   macOS Remote Login reachable only over the tailnet, tmux on the Mac so a session outlives the
   connection, and [Blink Shell](https://github.com/blinksh/blink) (GPL-3.0) on the phone with a
   Secure Enclave key, so the private key cannot leave the phone.

Why not an SSH client in the app: it needs its own key on the phone and its own server on the
Mac. The paired-device listener already authenticates the phone, and PTYs held on the Mac give
reattach without tmux.

### Routes

Same transport as the agent's session stream: JSON requests, and Server-Sent Events for output.
The phone signs every request with `DeviceClient` through the paired listener.

| Method and path | Body or query | Answer |
|---|---|---|
| `POST /api/agent/terminals` | `{preset, cwd, cols, rows}` | `{id}` |
| `GET /api/agent/terminals` | | `[{id, preset, cwd, title, started_at, last_output_at, alive, exit_code, clients}]` |
| `GET /api/agent/terminals/options` | | `{presets, folders}`: preset names, then `$HOME` and the folders in `~/Developer`, newest first |
| `GET /api/agent/terminals/{id}/stream?from=<offset>` | | SSE, events below |
| `POST /api/agent/terminals/{id}/input` | `{data}`: base64 bytes, at most 64 KiB | `204` |
| `POST /api/agent/terminals/{id}/resize` | `{cols, rows}` | `204`. The last resize wins. |
| `DELETE /api/agent/terminals/{id}` | | `204` after SIGHUP to the process group, SIGKILL after 5 s |

SSE events:

- `output`: `{offset, data}`. `offset` is the byte position of `data` in the terminal's output,
  and `data` is base64.
- `truncated`: `{first_offset}`. The requested `from` is older than the scrollback. The client
  clears its grid and reads again from `first_offset`.
- `exit`: `{code}`. The stream ends after it.

Every route declares its request schema (`schema_of::<T>()`), as `tools/check-route-schemas.sh`
requires.

### Behavior

- **Presets.** `shell` runs the user's login shell (`$SHELL -l`). `claude`, `codex`, `pi` and
  `sjel-agent` run that program inside the login shell, so `PATH` matches an interactive
  terminal. The commands live in `<overlay>/config/agent.toml` under `[terminal.presets]`, because
  install paths are machine facts; the field is the command line, and the five names are the
  default when the section is absent. `cwd` must be an existing directory under `$HOME`.
- **Environment.** `TERM=xterm-256color`, `COLORTERM=truecolor`, `LANG` from the user's
  environment. The PTY never inherits the capability's own secrets or tokens.
- **Scrollback.** A 2 MiB ring of raw output per terminal, held in memory. A client that
  reattaches reads from its last offset. A restart of the capability ends every terminal.
- **Limits.** At most 8 terminals. A terminal with no attached client and no output for 12 hours
  is closed like a `DELETE`. These are starting values.
- **Several clients.** The Mac and the phone can attach to the same terminal at once. Both see the
  same output, and either can type.

### Security

A terminal is a full shell with the user account's rights. The command policy in §3 cannot apply
to interactive keystrokes, so access is the control.

- The agent token never reaches these routes. `AgentRoutes::closed`
  (`libs/sjel-server/src/agent_policy.rs`) is checked in the gate before the token is admitted, so a
  closed route answers 401 with the same shape an unknown token gets. `capabilities/agent` lists
  every terminal route there, and `every_terminal_route_is_closed_to_an_agent_token` reads the
  manifest instead of restating the list, so a later route added without closing it fails the test
  rather than opening a shell.
  Recorded because the first draft of this section was wrong: `serve_local_with` *does* admit the
  agent token on this capability, which is why "does not call `admit_agents`" was never the
  mechanism.
- The capability stays on `Reach::Loopback`. The phone reaches it only through the paired
  listener and a device signature.
- The phone asks for Face ID (`LocalAuthentication`) before it opens or reattaches a terminal,
  once per foreground session of the app.
- One audit line per terminal start and end in `<overlay>/data/agent/terminals.jsonl`:
  `{at, id, preset, cwd, client, event, exit_code}`. No input and no output is logged.

### Client

- `apps/gpui/core/src/terminal.rs`: the wire types and the client. Built: `Host` over the Mac's
  shell or the phone's `DeviceClient`, the list row, open, attach-or-open, the stream with a
  resume offset, input, resize, close, and [`Position`], which is what stops a
  replayed frame from being drawn twice and what moves the reader to `first_offset` after
  `truncated`.
- `apps/gpui/ui/src/terminal.rs`: the view. Built: `alacritty_terminal` 0.26 (Apache-2.0, the
  licence check this plan asked for; its `upstreams.toml` row records the dependency cost and what
  it does not do) parses the stream into a grid, and GPUI draws it as runs of cells that share a
  look. Keys become pty bytes in `key_bytes`, which is the one path the Mac keyboard and a phone
  key row can share; the wheel moves through the parser's history; the cursor is the inverted
  cell.
- Mac: `⌘K › Terminal` or `sjel://terminal` opens the right edge on `Seat::Terminal`, focused, and
  ends a finished shell by offering a new one rather than a dead grid. The grid measures itself
  and resizes the pty; the edge's left border drags its width, kept in `sidebar.json`. The page's
  single-key bindings use `Workspace && !Terminal`, so Enter, J, K, G and Esc reach the program.
  A drag selects (double-click a word, triple-click a line, shift extends), ⌘C copies, ⌘V
  pastes with bracketed paste when the program asks for it, and ⌘A selects the scrollback.
  Settings › Appearance › Terminal sets the font (Auto picks an installed Nerd Font), size, tone
  and opacity; a trackpad pinch steps the size. ⌘T opens the terminal from anywhere; inside it
  ⌘T opens `+ New`, and ⇧⌘] / ⇧⌘[ (or ⌃Tab / ⌃⇧Tab) step the tabs. Built.
- Both: the seat is a deck of tabs (`ui/src/terminal/deck.rs`), one per live terminal, and
  `+ New` picks a preset and a folder from `GET /api/agent/terminals/options`. A tab's × ends its
  program. A resize reaches the pty only once the grid's size has held for 150 ms. Built; the
  options route needs the agent capability restarted to answer.
- Phone: `Seat::Terminal` asks for Face ID (`sjel_ios_unlock` in `ios/host/App.swift`, with the
  passcode as fallback), then attaches the newest live terminal or opens a login shell at 48 × 28.
  The view runs in touch mode: the key row (Esc, Tab, arrows, `|`, `~`, `/`, Return, Ctrl-C) sits
  above the grid, and a tap on the grid raises the software keyboard. Entering the background
  drops the view and the unlock, so a reattach asks again. Built and linked for the simulator.
  The key row carries copy and paste, Settings has the same four Terminal rows as the Mac, and a
  pinch steps the font size (`UIPinchGestureRecognizer` in `ios/host/App.swift`, because the iOS
  port reports no pinch). Not tried on a paired phone.
- Both: the grid is one canvas in Monokai (`theme::MONOKAI`). Each row is one shaped line with
  every glyph on the cell's advance (`force_width`) and one quad per painted background, so a
  fallback-font icon or a wide character keeps its columns.

### Done when

1. On the Mac, the `shell` preset runs `ls`, a resize redraws `htop` correctly, and scrollback
   reaches the first line. Partly: `ls` ran through the real service and a resize reached the
   kernel; `htop` and a full scrollback pass are untested.
2. On the phone, the `claude` preset starts in `~/Developer/Sjel` and accepts a prompt. After the
   phone is locked for 2 minutes, reopening the app reattaches and replays the missed output. Open:
   the phone opens `shell` only, and nothing has run on a paired phone yet.
3. The Mac and the phone attached to one terminal show the same output. Open: both clients can
   attach, but this has not been tried with two.
4. A request with the agent token to each terminal route answers 401 (test). Met, for every route
   in the manifest.
5. `DELETE` ends the process group, and the audit file holds a start and an end line with no
   input or output. Met.

## 7. Live validation

Work Package 22 is code-complete. Host checks pass: `sjel cargo check --manifest-path
apps/gpui/ios/Cargo.toml --locked`, the `aarch64-apple-ios-sim` check, the desktop check, and
`cargo check --manifest-path capabilities/agent/Cargo.toml --locked --offline`. CoreSimulator was
unavailable and no phone was connected, so nothing ran on a device. The desktop release build is
blocked, see [`gpui.md`](gpui.md#open).

1. **Desktop.** `⌘K` → `Agent`. Create and reopen a session, stream a reply, approve once, grant
   for the session, deny, stop. The Systems approval view and the seat show the same decision.
2. **iPhone.** Pair through the QR flow. The seat lists saved and active sessions. Start a
   session on the phone and send a follow-up. The Mac shelf shows the same transcript.
3. **Cross-device.** Follow a Mac run on the phone. Cut and restore the connection, then check
   sequence resume and the transcript reload after `truncated`. Decide from each client in turn,
   and check that the other clears its card. Check deny, stop, session grants and re-pairing.
4. **Boundary.** Every phone request is signed by `DeviceClient` and goes through the paired
   listener. The capability stays loopback-only. Coding sessions stay local. A change that widens
   `Reach` or names a cloud role is the point to stop and ask.

Facts a new session needs: `EventLog` keeps 4,000 events per turn, and a reader behind that gets
one `truncated` frame and re-reads `/transcript`. The gate polls every 250 ms. The build script
supplies `SJEL_BUILD_COMMIT` and `SJEL_BUILD_TIME`.

## 8. Evaluation against Pi

### Sequence: plan now, compare with Pi after the evaluation contract exists

Decided 2026-10-24: the first evaluation profiles are coding and autonomous research. Build the
fixtures and independent checks first, then compare with Pi.

- Development suite: 12 cases (C01–C04, R01–R04, D01–D04) in
  [`research/benchmarks/agent-harness`](../../research/benchmarks/agent-harness/README.md), with
  the offline evaluator ([`tools/agent-eval.ts`](../../tools/agent-eval.ts)) and live adapters for
  both harnesses. Labels have had agent review. Human review is pending.
- Comparison model: DeepSeek V4.1 Flash, c0 only, role `agent_eval_deepseek` in the overlay.
  Budget ends 2026-11-30.
- Runs so far: cells A and C on DeepSeek, 24 attempts, all finished, quality not scored.
- Final-test tasks, the adjudicator, editable components and promotion policy are not decided.

| Cell | Harness | Retrieval | Isolates |
|---|---|---|---|
| A | Pi | existing workflow | baseline |
| B | Pi | Sjel context tools | retrieval, harness fixed |
| C | Sjel | equivalent existing tools | harness, to the extent tools match |
| D | Sjel | Sjel context tools | retrieval in Sjel. D vs B tests the harness. |

Next, in order:

1. Human review of the `review.md` files in the overlay.
2. Cells B and D with vector retrieval. Only a measured answer gain justifies a retrieval
   capability on libSQL.
3. Tools that attest their data class, so cloud roles can do real work. Today only the
   evaluator's proxy tools attest c0 (`Tool::data_class`, `libs/agent/src/lib.rs`).
4. Local candidates (Gemma E2B/E4B, Granite, Apple Foundation Models) each get an
   `upstreams.toml` row and a measurement on one job.

The gate to retire Pi (decided 2026-10-24): `sjel-agent` beats Pi on one household task and one
coding task, each measured against the existing path.

### Pi daily use and managed configuration

Pi stays the daily agent during the evaluation. `sjel pi plan|apply|check|rollback`
([`tools/sjel-cli/src/pi_config.rs`](../../tools/sjel-cli/src/pi_config.rs), `6eef2c79`) owns
the keys in [`tools/templates/pi/settings.base.json`](../../tools/templates/pi/settings.base.json)
plus an overlay fragment. Credentials stay in the keychain and in oMLX's own settings. Open:
model and keybinding keys.

### Broader harness scope and factual precision

One runtime, with tool permissions and completion checks per profile: coding, research and
experiments, precise writing, legal and administrative work, productivity and system operations.
A material claim carries its source and the passage that supports it. A retrieved assertion is
not an established fact. Where evidence is incomplete, narrow the conclusion or abstain. The
profile table and the reasoning are in
[`research/assistant-harness.md`](../../research/assistant-harness.md).

### Two improvement loops, with different authority

The inner loop: observe, hypothesize, test, act, verify. A failed prediction interrupts the
dependent actions. The outer loop changes harness code on versioned candidates in an isolated
workspace. Evaluators, final-test data, permissions and egress rules stay outside the candidate's
write boundary. Autonomous testing does not authorize autonomous changes to policy or to the
evaluation. Sources: Meta-Harness ([arXiv:2603.28052](https://arxiv.org/html/2603.28052v1)) and
Schema, both in
[`research/assistant-harness.md`](../../research/assistant-harness.md#can-the-harness-improve-through-code-and-experiments).

## 9. Context and retrieval

### Agent data backbone: Turso and graph integration

Authoritative records stay with their capability. A graph or vector index is a rebuildable
projection. Agent databases hold isolated run state, cursors and caches. Results carry the source,
revision, observation time, data class and access scope. Authorization applies to the seed and to
every expansion. C3 never enters an index.

### Integration candidates to narrow together

Measured 2026-10-24: SQL edges cover the coding consumer. On Graphify's 37,200-node graph, SQL and
Grafeo both answered all 60 queries exactly, and a BFS in code over SQL edges beat Grafeo about 4×
on open-ended paths ([graph-store spike](../../research/benchmarks/graph-store/README.md)). Grafeo
and IndraDB are `reject` in `upstreams.toml`, with conditions to revisit.

### Sjel questions over documents: a small GraphRAG

Measured 2026-10-24: graph expansion found 18 of 24 required sections, vector search alone 17, at
the same 10-hit budget. Every added section was a neighbouring heading in the same file
([result](../../research/benchmarks/agent-harness/README.md#retrieval-alone-2026-10-24-graph-expansion-does-not-beat-vector-search)).
Keep vector search with same-file section context. Drop cross-document edges until a larger
question set shows evidence that only a link reaches.

Turso agent features (AgentFS, AI memory, code indexing, content caching) are candidates in
[`storage.md`](storage.md#6-agent-databases).

## 10. Decisions

All decided 2026-10-25.

| # | Question | Decision |
|---|---|---|
| A1 | What is the hard boundary for `project` and `account`? | The static classifier first. Then a macOS sandbox profile that limits writes to the tier scope, or `capabilities/agentbox` for untrusted work. In place before `account` ships. |
| A2 | Which web search providers to port from `pi-web-access`? | All of them. DuckDuckGo stays the keyless default. |
| A3 | Can a tier change inside a session? | No. A new session per tier. |
| A4 | Can the phone start a `project` or `account` session? | Yes. Every Ask and Block decision still needs a card. |
| A5 | Where do PTY sessions live? | `capabilities/agent`, because it already has the session, stream and device plumbing. Split into a `terminal` capability only if the two grow apart. |
