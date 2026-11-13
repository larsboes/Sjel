# The Sjel agent

External source decisions and integration boundaries live in
[`agent-upstream-integration.md`](../../research/agent-upstream-integration.md).
The Octet runtime trial is complete; it remains an idea source. Intelligent UI remains
renderer prior art, with no plugin hooks adopted.

A coding and operations agent written in Rust. It runs on the Mac, and the user controls it
from the GPUI app on the Mac or the paired iPhone. The loop is `crates/sjel-agent`. The capability that
serves it is `crates/agent`. The reasoning for that shape is in
[`crates/agent/README.md`](../../crates/agent/README.md).

Agent expansion is Priority 3 in [`PLAN.md`](../../PLAN.md). Existing protections remain
in force. The local order below applies after higher-priority maintenance and daily workflows.

Order:

1. The native terminal in the GPUI app (§6).
2. Tiers and command policy (§2, §3).
3. Live validation on Mac and iPhone (§7).
4. Plugins and the Pi port, all web providers (§4).
5. The measured comparison with Pi (§8).

## 1. What exists

- **Loop and CLI.** `crates/sjel-agent`: the core tools (`read`, `write`, `edit`, `bash`, `grep`,
  `find`), sessions in the overlay, steering, cloud calls with a data-class check.
- **Extensions in `crates/sjel-agent/src/ext/`.** `guard` (secrets, on by default), `web`, `subagents`,
  `mcp`, `skills`, `rust`, `graph`, `sjel`, `compaction`.
- **Capability.** `crates/agent` serves sessions, prompts, steering, a resumable event
  stream, approvals and stop, on `Reach::Loopback`. The phone reaches it through the paired-device
  listener.
- **Approval gate.** `crates/agent/src/gate.rs`. The command/write policy selects Allow, Ask or
  Block. An approval card shows a real diff for `edit` and `write`. A decision is a file in
  `secrets/agent/approvals/`, so the Systems page, the Mac and the phone show one record. A
  session grant is keyed by tool and exact arguments. An unanswered call is denied after 15
  minutes.
- **Clients.** One seat, `AgentSeat` (`apps/gpui/ui/src/agent_seat.rs`), holds the run on both
  devices over `Host` (`apps/gpui/core/src/agent.rs`): the Mac's shell or the phone's
  `DeviceClient`. It lists and creates sessions, prompts, steers, follows the stream with resume
  after `truncated`, approves, denies and stops. The Mac finder's `Agent` mode and the phone's
  edge each add only a text field. The seat offers the tier (`sjel | project | account`), and a
  blocked call shows its rule ID, a red border and one override button (no session-wide grant).

**Gap.** The capability assembles only `fs, sjel, sql, guard, compaction`
(`crates/agent/src/plugins.rs`). `web`, `subagents`, `mcp`, `skills` and `rust` exist in
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

Built (bc1ec6bc): `crates/agent/src/tier.rs` holds the three tiers, each naming its plugin
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
| B1 | Block | `rm` with `-r`, `-R` or `--recursive`, or any `rm` whose target is the tier root, `$HOME`, `/`, or outside the tier scope. `find … -delete`. |
| B2 | Block | Discarding uncommitted work or history: `git commit --amend`, `git reset --hard`, `git clean` with `-f`, `git checkout -- <path>` and `git restore` on the worktree, `git push` with `-f` or `--force`, `git push --delete`, `git branch -D`, `git tag -d`, `git filter-repo`, `git reflog expire`. |
| B3 | Block | Privilege: `sudo`, `su`, `doas`, `osascript` with `administrator privileges`. |
| B4 | Block | Download and run: a pipe or process substitution into `sh`, `bash`, `zsh`, `python`, `node` or `ruby` (`curl … \| sh`, `bash <(curl …)`). |
| B5 | Block | Disks and system state, except explicit diagnostic subcommands in R1: `dd`, `mkfs*`, `diskutil`, `fdisk`, `shred`, `csrutil`, `spctl`, `nvram`, `launchctl`, `systemctl`, `crontab -r`, `defaults write`, `chmod -R`, `chown -R`. |
| B6 | Block | Secrets: the existing guard (`crates/sjel-agent/src/ext/guard.rs`), plus `security find-*-password -w` and `security delete-*`. |
| B7 | Block | Irreversible publishing: `cargo publish`, `npm publish`, `bun publish`, `gh release create`, `gh repo delete`. |
| A1 | Ask | A write through the shell or the `write` and `edit` tools whose target is under `$HOME` but outside the tier scope. |
| A2 | Ask | Git changes to existing history, branches or remotes: `push` (no force), `pull`, `merge`, `rebase`, `cherry-pick`, `revert`, `am`, `apply`, `stash` (save, pop, apply), `reset` (soft or mixed), `switch`, `checkout` of a branch, `branch` and `tag` create, rename or delete, `worktree`, `submodule`, `remote` changes, `config` writes, `gc`, `prune`, `notes`, `bisect`, `update-index`. |
| A3 | Ask | Package changes and download-and-run: `brew`, `npm`/`pnpm`/`yarn`/`bun` `install`, `add`, `remove`, `update` and `link`, `pip install` and `uninstall`, `pipx`, `uv` `add`, `remove`, `sync`, `pip` and `tool`, `cargo install`, `add`, `update` and `remove`, `rustup` changes, `apt`, `dnf`, `snap`, `flatpak`, `gem install`, `go install` and `go get`, and `npx`, `bunx`, `uvx`. |
| A4 | Ask | Network writes and remote shells: `curl` or `wget` that send data (`-d`, `--data*`, `-F`, `-T`, a `-X` other than `GET` or `HEAD`), `ssh`, `scp`, `sftp`, `rsync` to a remote, `nc`, `telnet`, `ftp`, and `gh` write subcommands. |
| A5 | Ask | Processes and containers: `kill`, `pkill`, `killall`, and `docker`, `podman` and `orb` commands other than reads. |
| A6 | Ask | Opaque input (step 3): a program name built at run time, `eval`. |
| R1 | Allow | Reading: `ls`, `cat`, `rg`, `grep`, `find` without `-delete`, `curl` and `wget` reads, Docker/Podman read subcommands including `system df`, and explicit disk/service diagnostics (`diskutil list`, `launchctl print`, `systemctl status`, `csrutil status`, `spctl --status`). Argument substitutions retain their own classification. |
| R2 | Allow | New local commits without `--amend`, Git reads and local index changes: `status`, `diff`, `log`, `show`, `blame`, `branch` (list), `remote -v`, `fetch`, `add`, `rm`, `mv`, `restore --staged`, `clone`, `init`. |
| R3 | Allow | Build and test: `cargo` other than A3, `bun test`, `bun run`, `npm run`, `tools/doctor`, `sjel` read verbs. |
| R4 | Allow | Every program without a rule, interpreters running a named script or `-c` code, here-documents, `source`, `sed -i` and `perl -i`, non-recursive `rm` of a file inside the tier scope, including `-f`, and writes per step 4. |

### Where it lives and how it is tested

- `crates/sjel-agent/src/ext/policy.rs`: the classifier, so the CLI and the capability share one rule
  set. `crates/agent/src/gate.rs` reads the class: Allow skips the card, Ask shows it, and
  Block shows the separate card with the rule ID.
- `Packs/security/extensions/command-policy/cases.toml`: the one table both implementations are
  held to — a case per rule ID, the evasions that must never be Allow (`r''m -rf x`, `\rm -rf
  x`, `env rm -rf x`, `command rm -rf x`, `$(echo rm) -rf x`, `bash -c "rm -rf ~"`, `echo x |
  xargs rm -rf`, `find . -exec rm -rf {} +`, `ls; rm -rf x`), and the `[[write]]` cases. Each is
  written against a fixture scope — a synthetic `$HOME`, a project directory inside it, and the
  temp directory writable — so a case means the same on every machine.
  `tools/command-policy.test.ts` runs it against the pi extension,
  `crates/sjel-agent/tests/policy_cases.rs` against the Rust one.
- Done when every case passes, the gate shows the rule ID on each card, and one week of real use
  gives a prompt rate (below).

The Pi implementation and Rust classifier share the case table (`b0d94cae`); tests are
`tools/command-policy.test.ts` and `crates/sjel-agent/tests/policy_cases.rs`.

Pi-specific session file scopes and 60-second permission/decision deferral are implemented
(commit pending). The current interfaces and limits live in
[`Packs/security/README.md`](../../Packs/security/README.md#pi-session-permissions) and
[`Packs/knowledge/README.md`](../../Packs/knowledge/README.md#extensions).
No classifier model or account-wide bypass is introduced. The Rust gate remains unchanged.
Checked: 65 focused permission, question and Pack-extension tests pass, including the installed
Pi typecheck; one non-applicable fallback test is skipped. Live acceptance remains: reload Pi,
exercise one scoped backup, one unanswered approval, and one partially answered decision round.
RPC asks currently defer immediately because the editor handoff has no cancellation API.
Live TUI/RPC acceptance and full-workspace checks have not run.

Decided 2026-11-08: ordinary local work and diagnostics run without approval; destructive loss,
privilege, secrets and external writes keep their guards. New local commits and non-recursive
in-scope `rm -f` run; amendments and recursive deletion remain Block. Nested container reads and
explicit system diagnostics are covered by the shared table, including the compound storage
inventory regression. Container config rendering remains Ask because output options can write
files. Package changes remain Ask and must be resolved before unattended work.
The native Pi headless procedure lives in the `sjel` skill's `on-unattended.md` reference:
approved local tasks only, blocked-task deferral, persistent sessions and evidence checkpoints.
A hard wall-clock or spending supervisor is not implemented; an enforced limit or explicit
approval for an uncapped run is required before launch. No unattended job has been started.
Checked: 14 Pi tests, 4 shared-table Rust tests and 6 Rust policy unit tests pass; the table
contains 167 command cases and 5 write cases. Full-workspace checks did not run.

A local small model may classify A6 commands later, only after a measurement shows it lowers the
prompt rate without a missed Block. `Packs/harness/extensions/clm-classifier.ts` is the prior art.

The capability gate logs the selected rule and implements Allow/Ask/Block (`6622b21e`).
Open: the separate Block card in §5, A6 classification of unknown programs, and the measures below.

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
`crates/sjel-agent/src/ext/guard.rs` makes for secret paths. A command can write outside the root
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
- Later: a run answers with a view document, not only text, under
  [`gpui.md` Generated views](gpui.md#views-from-the-web-catalog).

## 6. Terminal from the phone

Decided 2026-10-25: the phone gets a real terminal on the Mac over the tailnet. Claude
Code, Codex, Pi or any other CLI agent then runs on the Mac and is driven from the phone. The Sjel
agent keeps its native UI. Every other agent uses the terminal.

Steps:

1. **PTY sessions on the Mac** in `crates/agent` (A5), behind the routes below. Built
   (`833c571d`): `crates/agent/src/terminal/mod.rs`, with `[terminal.presets]` in
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

### Contract

[Terminals](../../crates/agent/README.md#terminals) owns routes, events, presets,
environment, scrollback and limits. `cwd` must name an existing directory under `$HOME`.

### Security

A terminal is a full shell with the user account's rights. The command policy in §3 cannot apply
to interactive keystrokes, so access is the control.

- The agent token never reaches these routes. `AgentRoutes::closed`
  (`crates/sjel-server/src/agent_policy.rs`) is checked in the gate before the token is admitted, so a
  closed route answers 401 with the same shape an unknown token gets. `crates/agent` lists
  every terminal route there, and `every_terminal_route_is_closed_to_an_agent_token` reads the
  manifest instead of restating the list, so a later route added without closing it fails the test
  rather than opening a shell.
- The capability stays on `Reach::Loopback`. The phone reaches it only through the paired
  listener and a device signature.
- The phone asks for Face ID (`LocalAuthentication`) before it opens or reattaches a terminal,
  once per foreground session of the app.
- One audit line per terminal start and end in `<overlay>/data/agent/terminals.jsonl`:
  `{at, id, preset, cwd, client, event, exit_code}`. No input and no output is logged.

### Client and shell integration

- Mac and phone share `Host`, the terminal seat and resume handling (`e1b19960`, `3ef11ee6`).
- Tabs, the preset/folder menu, selection, clipboard and appearance controls are built
  (`a4e29630`); client behavior lives in `apps/gpui/core/src/terminal.rs` and
  `apps/gpui/ui/src/terminal/`.
- Shell command/exit marks are implemented (`34185597`). Live zsh and phone validation remain open.
- The phone opens a shell behind Face ID/passcode and clears its unlock in the background.
  Native paired-phone acceptance remains open below.

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
`cargo check --manifest-path crates/agent/Cargo.toml --locked --offline`. CoreSimulator was
unavailable and no phone was connected, so nothing ran on a device. The desktop release build is
blocked, see [`gpui.md`](gpui.md#open).

1. **Desktop.** `⌘K` → `Agent`. Create and reopen a session, stream a reply, approve once, grant
   for the session, deny, stop. The Systems approval view and the seat show the same decision.
2. **iPhone.** Pair through the QR flow. The setup page offers the Mac's tailnet address beside
   the Same Wi-Fi name, so the phone can pair for use away from home.
   `apps/gpui/ios/run-device <udid>` installs a development build. Not run yet: a phone scan of
   the tailnet code. The tailnet ACL from the phone to port 8443 has not been tested. The seat
   lists saved and active sessions. Start a
   session on the phone and send a follow-up. The Mac shelf shows the same transcript.
3. **Cross-device.** Both clients run the same seat code, so a defect found on one is fixed on
   both. Follow a Mac run on the phone. Cut and restore the connection, then check
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
   evaluator's proxy tools attest c0 (`Tool::data_class`, `crates/sjel-agent/src/lib.rs`).
4. Local candidates (Gemma E2B/E4B, Granite, Apple Foundation Models) each get an
   `upstreams.toml` row and a measurement on one job.

The gate to retire Pi (decided 2026-10-24): `sjel-agent` beats Pi on one household task and one
coding task, each measured against the existing path.

### Pi daily use and managed configuration

Pi stays the daily agent during the evaluation. `sjel pi plan|apply|check|rollback`
([`crates/sjel-cli/src/pi_config.rs`](../../crates/sjel-cli/src/pi_config.rs), `6eef2c79`) owns
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
| A1 | What is the hard boundary for `project` and `account`? | The static classifier first. Then a macOS sandbox profile that limits writes to the tier scope, or `tools/agentbox` for untrusted work. In place before `account` ships. |
| A2 | Which web search providers to port from `pi-web-access`? | All of them. DuckDuckGo stays the keyless default. |
| A3 | Can a tier change inside a session? | No. A new session per tier. |
| A4 | Can the phone start a `project` or `account` session? | Yes. Every Ask and Block decision still needs a card. |
| A5 | Where do PTY sessions live? | `crates/agent`, because it already has the session, stream and device plumbing. Split into a `terminal` capability only if the two grow apart. |
