# host-net

What this host exposes to the network, read without sudo. Four verbs over the listening
sockets, the application firewall and the tailnet, plus one policy check a scheduled job can
call. It observes and never changes a rule: `capabilities/host-firewall` is the half that
applies.

## Why it exists

`lsof` is the obvious command and, without root, it silently answers a smaller question than
the one you asked. Measured on this Mac 2026-09-02, twice in one session:

| Reading | `netstat -anv -p tcp` | `lsof -nP -iTCP -sTCP:LISTEN` |
|---|---|---|
| with the VPN extension stopped | 29 LISTEN rows, every uid | 27 rows, all one uid |
| with it running | 32 LISTEN rows, every uid | 27 rows, all one uid |

The five rows in the second case are all uid 0, and two of them are wildcard binds on port
443. An earlier mapping pass on this machine used `lsof`, enumerated five loopback ports, and
concluded the host had no wildcard listener at all. It was reading a filtered list that looked
like a complete one, which is the failure mode worth building against: a listener the tool
cannot see is worse than no tool.

So this reads `netstat`, whose one cost is a sixteen-character truncation of the process name,
repaired by joining on pid against `ps` — which reports the full executable path for
root-owned pids without sudo.

## Use

```sh
host-net listen [--json]     # every listening socket and the scope its bind reaches (default)
host-net firewall [--json]   # the application firewall's switches and its stale app rules
host-net tailnet [--json]    # tailnet posture: backend, shields, key expiry, tailnet lock
host-net check [--json]      # wildcard listeners the overlay policy does not account for
```

The CLI is on PATH automatically — `capabilities/shell/init.zsh` sweeps for
`capabilities/<name>/<name>`, so nothing is registered by hand.

**Exit codes**, which `tools/host-watch` and any other caller depend on:

| Code | Meaning |
|---|---|
| 0 | checked, and everything matched the policy |
| 1 | checked, and something is exposed that the policy does not account for |
| 2 | could not check: usage error, no policy file, unsupported platform, or a required command missing or refused |

2 rather than 3 for "a required command is missing". `tools/audit` and
`capabilities/host-firewall/host-firewall` both already spell "usage or setup error" as 2, and
`tools/host-watch` already spells "no policy" as 2. A third meaning invented here would be the
one exit code in the repo that means something else.

## What the scopes mean

`loopback` is 127.0.0.0/8 or ::1 — reachable only from this host. `tailnet` is an address in
100.64.0.0/10 or one on a utun interface. `lan` is a specific address on any other interface.
`wildcard` is `*`, `0.0.0.0` or `::`.

Only `wildcard` is ever a finding, and the reason is in the word "every": a wildcard bind is
reachable from loopback, from the local network, from the tailnet **and** from any container
bridge this host grows later, without the process being restarted or asked. Measured here
today, every non-loopback listener on this Mac is wildcard and none is bound to one interface,
so naming the interfaces a row covers would print the same six names on every row.

## What this does not do

No `apply`, no proposed rule, no probe of another host, and no sudo. It never runs
`socketfilterfw --getappblocked`, which takes a path and might create the entry it is asked
about; a read command that might write is not a read command.

## Machine facts live in the overlay

Every process name is in `<overlay>/config/host-net-policy.toml`; this capability's code
contains none (CONTRIBUTING.md#public-core-and-private-overlays). Shape:
`schemas/host-net-policy.toml.example`.

Entries match on the executable's basename, never on a port. A mesh VPN is assigned fresh
wildcard ports on every start — this host showed a different one an hour apart — so a policy
keyed on port numbers needs editing after each reboot, and a check nobody can keep current is
a check that gets deleted.

Keep the list short. Every name on it is a listener this check can never warn about again.

## How host-watch uses it

`tools/host-watch` runs `host-net check --json` once per pass and folds the result into
exactly one finding keyed `net:unexpected-exposure` — one row for the condition, never one per
port, for the same reason its `cpu:<comm>` findings collapse: the store's partial unique index
allows one open row per key, and ephemeral wildcard ports would mint a new generation every
hour.

It calls the built binary at `target/release/host-net-cli`, never the launcher in this
directory. The launcher builds on first use, and an hourly scheduled job with the ability to
start a `cargo build` is a surprise nobody asked for. A host that has never built it files
nothing and says so once on stderr.

## Why this shape: read-only, netstat-first, no manifest

<!-- asserts-absent: capabilities/host-net/service.toml -->

**No `service.toml`, and the schema proves it is the right answer rather than a shortcut.**
`tools/check-service-tomls.sh` requires a `port` for `kind = "process"` without a `schedule`,
and refuses a `port` alongside one. "A process, no schedule, run on demand" is not expressible,
because it is not a service. Five capabilities already live with no manifest at all —
`capabilities/host-firewall`, `capabilities/cv`, `capabilities/agentbox`,
`capabilities/printing`, `capabilities/shell` — and the shell sweep puts their CLI on PATH with
nothing registered.

**Rust, not the shell its siblings use.** `capabilities/host-firewall` is Bash because its job
is "render one ruleset and hand it to `nft`", and that verdict is the test. This job is four
undocumented text formats — netstat's column layout, `socketfilterfw --listapps`,
`ps`, `ifconfig` — parsed into one typed record that a second program reads as JSON. The
parsers carry the argument, and they carry it in tests: `src/listen.rs` alone has nine, seven of
them over captured-output fixtures, including the two netstat process-field shapes that break
every naive pattern. Awk would have to be right about the same things with no way to say so.

**A separate capability, not a verb on `host-watch`.** `capabilities/host-watch` is a
scheduled job with no port whose README declines both a panel and a server; three interactive
listing verbs on a job nobody invokes is the wrong owner. Its TypeScript runtime is justified
narrowly, by a policy file shape `tools/lib/toml.sh` cannot parse, and that justification does
not transfer (CONTRIBUTING.md#implementation-languages-and-intelligence puts backend logic in Rust).
host-watch is the consumer here, and consumes one verdict.

**Read from the right, never by column index.** macOS documents no column contract for
`netstat -anv`, and the process field both contains spaces and can end in one — `OrbStack
Helper:74138` and `Obsidian Helper :80286` are both live on this machine, six of the LISTEN
rows today. Field counts differ between rows of the same table, because the `(state)` column is
filled for TCP and empty for UDP. What is stable is the eight-field tail after the process
name, so the parser anchors there. `src/listen.rs` states it, and a fixture pins both shapes.

**Every test is a parser test over a captured fixture.** CI runs `cargo test` on Linux, where
there is no `netstat -anv`, no ALF and no tailnet, so a test that shelled out to the real tool
would be a test that never ran. The fixtures under `fixtures/` are synthetic for a second
reason: `tools/check-publication-hygiene.sh` rejects any tracked blob containing a workstation
home path, and a real `--listapps` dump on this machine is full of them.

## Known gaps

- **The Linux branch is unverified.** No Linux host was reachable while this was written, so
  `src/linux.rs` is built from `ss`'s documented output rather than from a capture. It will not
  return an empty list when `ss` is missing.
- **`nft` needs root**, so `firewall` reports the layer as unavailable on Linux rather than
  printing a half-answer.
- **`socketfilterfw --listapps` is believed read-only and not proven.** Repeated calls in one
  session returned a stable count, which is weak evidence. Watching the count over a week is
  the cheap resolution.

## Verification

`cargo test -p sjel-host-net` — 26 parser tests over the fixtures. The one that matters most is
`a_root_owned_listener_survives_the_join`: swap the implementation back to unprivileged `lsof`
and that row disappears, which is the whole reason this capability reads `netstat`.
`tools/sjel-cli/src/host_watch/pure.rs` covers the folding of `check --json` into a single finding.
