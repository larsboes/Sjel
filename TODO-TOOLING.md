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
- **Mail event outcomes by sender.** To choose the next ticket reader, the agent read
  `/triage/events/status` 20 ids at a time over all 892 mails and grouped the receipts by sender
  domain with an ad hoc Python script. Only 19 mails had a receipt, because the sweep scans new
  mail only. Smallest change: `sjel capability mail events report`, which groups receipts and
  candidate metadata by sender domain and state, and prints counts only.
- **Inference role probe.** To test what the light model returns for a fixed prompt, the agent
  wrote a `curl` call against the backend port and could not reach oMLX without its key. Smallest
  change: `sjel inference probe <role> <prompt-file>`, which uses the configured key and never
  prints it.
- **Installed-software route for the dashboard and `ask`.** `sjel storage installed` walks every
  app bundle and its `~/Library` data and takes about 30 s on this machine, so it stays CLI-only —
  the same reason `report --where` is not served. Smallest change: sample it daily into the store
  `capabilities/sjel-status/src/storage_history.rs` already owns, and serve the last snapshot from
  `/api/sjel-status/storage-installed`, so the panel and an agent read a number instead of waiting
  for a walk.
- **Local inference server admin.** To cap oMLX's SSD cache at 4 GB, lower `max_concurrent_requests`
  and clear 26 GB of stale cache, the agent posted to `/admin/api/global-settings` and
  `/admin/api/ssd-cache/clear` through an ad hoc Python script that read the key out of
  `~/.omlx/settings.json`. `sjel model-check --local` reads that backend but changes nothing, and
  a hand-edit of the JSON loses to the app rewriting it on exit. Smallest change:
  `sjel inference admin get|set <key>=<value>` and `sjel inference admin clear-cache`, with the
  same key handling as the probe above, so the settings a deployment depends on are readable and
  durable from a `sjel` command.

- **Real names in tracked files.** A test fixture carried a real contact's full name, and no gate
  noticed. The agent compared tracked files against the people registry with an ad hoc Python
  script. Smallest change: the publication hygiene check in `tools/doctor` flags two adjacent
  registry tokens in a tracked file, and prints only the file and a count.
- **Broken relative links in docs.** When the plans moved to `docs/plans/`, the agent checked
  every relative Markdown link with a shell loop. No gate checks that a link target or its
  `#anchor` exists. Smallest change: a `tools/check-doc-links.sh` beside
  `check-doc-self-references.sh`, in the same CI repo gates.

## Done

- `sjel capability mail events [replay] <id>...` reads safe per-mail event scan states and
  replays them.
- `tools/service-runner.sh stop|restart` stops an orphaned copy of the capability's own binary
  that holds its port. Before, it refused and asked for a manual `kill`.
