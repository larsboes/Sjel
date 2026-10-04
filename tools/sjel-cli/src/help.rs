//! Help text for `sjel help [command]`, moved verbatim from the bash launcher on 2026-10-02.
//! `sjel search` matches against USAGE line by line, so a line here is also a search target.

pub const USAGE: &str = r#"Usage: sjel <command> [arguments]

Discover Sjel:
  help [command]                 Show commands or one command's usage
  search <words...>              Search commands, tools, capabilities, and Packs
  doctor                          Check this Sjel installation
  context with [capability]      Emit bounded current operating context
  context on [unit-or-path]      Emit bounded current repository context
  storage <report|apply|target|prune>
                                 What is filling this disk, and what is safe to reclaim
  update [--json|--offline]      Software installed outside this checkout, and what is stale
  update apply [--only <class>]  Move what Sjel owns, and delegate the rest

Check a change before pushing it:
  gates                          Run CI's repo gates here, from CI's own definition
  test                           Run CI's bun test and every tools/*.test.sh
  cargo <args...>                Run cargo against a scratch target dir and a scratch overlay

Operate capabilities:
  capability list                List registered HTTP capability surfaces and health
  capability health              Probe registered capability health endpoints
  capability url <capability>    Print a registered capability's HTTP base URL
  capability call <name> <get|post|put|patch|delete> <path> [body] [curl-args...]
  capability ingest <url>        Ingest a URL through comms
  capability feed [days]         Read the comms feed
  capability mail [status]       Read mail triage without Secret rows; pseudonymized for an agent
  agent enroll                   Create or rotate the agent token (Keychain) and its server hash
  mcp                            Serve the capabilities as MCP tools on stdio, under each one's agent mode
  mcp register [harness]         Register that server with pi or Claude Code, then verify it answers
  claude [apply|check]           Write or check THIS device's Claude Code settings

Manage Packs:
  pack list [harness]            List available Packs, optionally for one harness
  pack status <harness> [pack|--all]
  pack deploy <harness> <pack>...
  pack sync <harness> <pack|--all>
  pack remove <harness> <pack>...

Run 'sjel help <command>' for focused help."#;

pub const CAPABILITY: &str = r#"Usage: sjel capability <command>

  list | health | url <capability>
  call <name> <get|post|put|patch|delete> <path> [body] [curl-args...]
  ingest <url> | feed [days]

health polls each capability's ready_path where it declares one and its health_path
otherwise, which is what sjel-status judges availability on. Four states:

  up       answered 200
  down     should be answering and did not. The only state that exits non-zero
  off      this machine does not autostart it, so not running is not a fault
  unknown  declares neither a health nor a ready path; nothing can answer for it

An external capability is polled at its resolved endpoint, not on loopback: it has no
port here because a port is a fact about the host that binds it. It is never `off`
either — the registry blanks its autostart, because how another host runs a capability
is that host's declaration to make, not a claim this machine may read as permission."#;

pub const CLAUDE: &str = r#"Usage: sjel claude [apply|check] [--force] [--dry-run]

  (no arguments)   Merge Sjel's baseline into this device's Claude Code settings file,
                   existing-wins, so your own values always survive and re-running is safe
  check            Report how the settings file has drifted from the baseline; exits 3 on drift
  --force          Overwrite the values the baseline declares, to restore the floor
  --dry-run        Say what would change and write nothing

Target: $CLAUDE_CONFIG_DIR/settings.json, else ~/.claude/settings.json.
Baseline: tools/templates/claude-code/settings.base.json, laid over with this deployment's own
<overlay>/config/claude-code/settings.fragment.json when one exists.

The security floor used to be a root-owned managed policy deployed with sudo. The principal's
ruling of 2026-10-02 moved it here: one file, no root. What that gives up is that the file is
writable by the agent sessions that run as you, which is why `check` exists — it is the only
thing in this repository that would notice the floor being lifted."#;

pub const MCP: &str = r#"Usage: sjel mcp [register|unregister [<harness>...]]

  (no arguments)              Serve the capabilities as MCP tools on stdio, one tool per route,
                             under each capability's agent mode
  register [<harness>...]     Write Sjel's server into pi, or drive `claude mcp add`, then speak
                             MCP to it and report the tool count it answers with
  unregister [<harness>...]   Remove it again

Harnesses: claude and pi. Codex supports MCP but has no measured path in this repository, so
it is reported rather than guessed at. The agent token stays in the login Keychain; no
registration writes a secret.

Claude Code is governed by a managed allowlist that filters MCP servers. When the deployed
policy predates this server, `register claude` fails and names the deploy that fixes it."#;

pub const PACK: &str = r#"Usage: sjel pack <command> <harness> ...

Harnesses: claude, codex, opencode, pi
Commands: list [harness], status, deploy, sync, remove, use <profile>"#;

pub const STORAGE: &str = r#"Usage: sjel storage <command>

  report [--json]   free/used/total, every policy class, what is flagged
  apply  [--json]   run each applicable class's reclaim command
  target [--json]   the Cargo target dir against PRD §9's R6 debug/release ratio
  prune [--incremental] [--target] [--node-modules] [--dry-run]

Policy: <overlay>/config/storage-policy.toml. See tools/storage/README.md."#;

pub const UPDATE: &str = r#"Usage: sjel update [report|apply] [--json] [--offline] [--only <class>...]

  report [--json]   what is installed outside this checkout, who owns moving each
                    class, and what has gone stale; --offline reads receipts and
                    installed versions without asking a registry anything
  apply             move what nothing else owns (cargo binaries, npm -g packages)
                    and delegate the rest to its owner: brew/uv/rustup go through
                    tools/host-patch.sh, the two integrations through their verb

Classes for --only:
  brew uv rustup containers graphify interceptor checkout cargo npm vendor

Report exits 1 when something is stale. Apply exits 1 when there was nothing to do.
The --json payload is the stable surface a UI reads; see src/updates/ in this crate."#;

pub const GATES: &str = r#"Usage: sjel gates | sjel test

Both replay one job of .github/workflows/ci.yml here, reading the step list out of that
file. No gate is named here on purpose: a second copy of the list is a copy that goes
stale, so `tools/ci-local list` prints what each job will actually run.

  gates   the repo-gates job — the file-based checks, in CI's order
  test    the bun-tests job — `bun test` from the root, then every tools/*.test.sh

Every step runs even after one fails. Exit 1 means a step failed.
Rust is deliberately not here: run `sjel cargo test --workspace --locked`.
`tools/ci-local list` also says why CI's remaining jobs are refused on this machine."#;

pub const CARGO: &str = r#"Usage: sjel cargo <args...>

cargo, with the four things a run on this machine has to be protected from:

  a scratch CARGO_TARGET_DIR   the repository's target/release/<bin> IS the binary a
                               live service is running, and replacing it kills it
  --release refused            for the same reason
  a scratch overlay            SJEL_* is cleared, then pointed at a throwaway root
  scratch projection roots     trips, finance, interior and comms export notes into
                               the Obsidian vault from roots SJEL_DB_PATH never touches

  --print-env    show the environment it would use, and run nothing

Set CARGO_TARGET_DIR yourself to reuse a warm one; anything inside the repository
is refused."#;
