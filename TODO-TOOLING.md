# Tooling gaps

Gaps that agents hit while they worked, where Sjel tooling did not cover the task. The rule is in
[`AGENTS.md`](AGENTS.md). Add an entry when you wrote a script or a workaround instead of using a
`sjel` command. When someone picks an entry up, move it to a GitHub Issue or build it, then delete
it here.

Entry format: what was needed, what the agent did instead, and the smallest tooling change that
covers it.

## Open

- **Runtime status for agents.** `sjel runtime status` answers
  `invalid or missing authentication token` from an agent session. An agent cannot see whether
  On-the-go mode deferred a model call. Workaround: read the defer reason from
  `/tmp/sjel-<cap>.err`. Smallest change: send the agent token, or add a read-only agent route
  for the runtime mode.
- **Timestamps in script service logs.** A Rust service stamps its own stderr from
  `sjel_server::serve` on (`libs/sjel-server/src/stderr_stamp.rs`). A `process` service that is a
  script or container still writes lines without a time to `/tmp/sjel-<cap>.err`. Smallest
  change: the runner pipes those through the same stamp.
- **Inference role probe.** To test what the light model returns for a fixed prompt, the agent
  wrote a `curl` call against the backend port and could not reach oMLX without its key. Smallest
  change: `sjel inference probe <role> <prompt-file>`, which uses the configured key and never
  prints it.

- **Real names in tracked files.** A test fixture carried a real contact's full name, and no gate
  noticed. The agent compared tracked files against the people registry with an ad hoc Python
  script. Smallest change: the publication hygiene check in `tools/doctor` flags two adjacent
  registry tokens in a tracked file, and prints only the file and a count.

## Done

- `sjel capability mail events [replay] <id>...` reads safe per-mail event scan states and
  replays them.
- `tools/service-runner.sh stop|restart` stops an orphaned copy of the capability's own binary
  that holds its port. Before, it refused and asked for a manual `kill`.
