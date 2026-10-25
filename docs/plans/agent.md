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

## 3. Command policy

Decided 2026-10-25. Every `bash` call and every write gets one of three classes.

| Class | Behavior | Examples |
|---|---|---|
| Block | Refused. The user can allow one call from a separate, explicit card. A session grant is never possible. | `rm -rf`, `git push --force`, `git reset --hard`, `git clean -fd`, `sudo`, `curl … \| sh`, `dd`, `diskutil`, `chmod -R`, `launchctl`, a write outside the tier scope, a secret path (the existing guard) |
| Ask | The approval card, as today. A session grant is possible. | File writes and edits, `git commit`, `git push`, package installs, network commands, any command the classifier does not know |
| Allow | Runs without a card | Read-only tools, `ls`, `rg`, `cat`, `git status`, `git diff`, `git log`, `cargo check`, `cargo test` |

Rules for the classifier:

1. Static rules in Rust first. A compound command (`&&`, `;`, `|`, `$(…)`, subshells) gets the
   strictest class of its parts.
2. An unknown command is Ask, never Allow.
3. A local small model may classify unknown commands later, only after a measurement shows it
   lowers the prompt rate without a missed Block. `Packs/harness/extensions/clm-classifier.ts` is
   the prior art in the Pi setup.
4. Every decision goes to the call log with its class and the rule that matched.

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

**Placement, proposed.** The classifier goes in `libs/agent`, so the CLI and the capability use
one rule set. The capability's `ApprovalGate` reads the class: Allow skips the card, Ask shows it,
Block shows the separate card.

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
terminal-emulator crate is in the workspace. Decided 2026-10-25: build the native terminal first.
The SSH path is the fallback, not a first step.

Steps:

1. **PTY sessions on the Mac.** `capabilities/agent` holds the PTYs (A5) and serves them through the
   paired-device listener, with the same signed `DeviceClient` requests as the agent. Output
   streams in frames with a sequence number, like the agent event stream. Input goes up as
   requests. The PTY lives on the Mac, so a dropped phone connection does not stop a running
   Claude Code. The phone reattaches and replays the scrollback.
2. **One terminal view for Mac and phone.** A VT parser and grid (candidate: `alacritty_terminal`,
   which Zed's GPUI terminal uses, after an `upstreams.toml` review). A phone key row with Esc,
   Tab, Ctrl and arrows. A session list. Presets that start `claude`, `codex`, `pi`, `sjel-agent`
   or a plain shell in a chosen folder. Zed's GPUI terminal is the reference to read first. Check
   its licence against Sjel's AGPL-3.0 in the `upstreams.toml` review.

3. **Fallback.** If the native terminal does not work, use an SSH path instead: Tailscale SSH or
   macOS Remote Login reachable only over the tailnet, tmux on the Mac so a session outlives the
   connection, and [Blink Shell](https://github.com/blinksh/blink) (GPL-3.0) on the phone with a
   Secure Enclave key, so the private key cannot leave the phone.

Why not an SSH client in the app: it needs its own key on the phone and its own server on the
Mac. The paired-device listener already authenticates the phone, and PTYs held on the Mac give
reattach without tmux.

**Security.** A terminal is a full shell with the user account's rights. The command policy in §3
cannot apply to interactive keystrokes. Proposed controls: the tailnet and the device signature
(as today), Face ID before a terminal opens on the phone, the capability bound to loopback behind
the paired listener, an idle timeout, and one audit line per session start and end. Keystrokes and
output are not logged.

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
[`storage.md`](storage.md#5-agent-databases).

## 10. Decisions

All decided 2026-10-25.

| # | Question | Decision |
|---|---|---|
| A1 | What is the hard boundary for `project` and `account`? | The static classifier first. Then a macOS sandbox profile that limits writes to the tier scope, or `capabilities/agentbox` for untrusted work. In place before `account` ships. |
| A2 | Which web search providers to port from `pi-web-access`? | All of them. DuckDuckGo stays the keyless default. |
| A3 | Can a tier change inside a session? | No. A new session per tier. |
| A4 | Can the phone start a `project` or `account` session? | Yes. Every Ask and Block decision still needs a card. |
| A5 | Where do PTY sessions live? | `capabilities/agent`, because it already has the session, stream and device plumbing. Split into a `terminal` capability only if the two grow apart. |
