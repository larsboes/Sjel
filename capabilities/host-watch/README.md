# host-watch

The machine's own vital signs, checked hourly, reported only when something crosses a
line. Most runs print one line and write nothing, and that silence is the product.

## Why it exists

Two host conditions were invisible until they hurt, and both did on 2026-08-15 (Axon#177).

A System Settings Storage pane got stuck at 08:35 and ran until it was killed at 17:52.
`ApplicationsStorageExtension` burned **3h29m of CPU** in that window; the only symptom
anyone experienced was "the Mac feels slow", nine hours in. Nothing here watched
cumulative CPU time, so a process quietly holding a third of a core forever looked
exactly like an idle one.

The disk half is the more instructive failure, because it was **already solved**.
`tools/storage` and `tools/sysmon report` (#135/#136) answered the question in seconds
when someone finally ran them. But both were on-demand only — no schedule, no threshold,
no surface. A tool nobody runs and a tool nobody built are the same tool. That is the
gap this closes, and it is a scheduling gap, not a missing feature.

## What it checks

**Runaway processes.** Cumulative CPU time divided by seconds alive — how much of a core
a process has held for its entire life. Two conditions must both hold: a floor of
accumulated CPU (`min_cpu_seconds`, so a five-minute compile is not a finding) and a
sustained-core ratio (`min_cpu_ratio`, so merely being old is not a finding either).

Ranking by raw CPU time is the obvious rule and it is wrong. On the day this was written
WindowServer had **more** cumulative CPU than the stuck extension — 168 minutes against
148 — and was perfectly healthy. It had simply been alive four times longer. That pair is
frozen as a fixture in `tools/sjel-cli/src/host_watch/pure.rs`, because it is the one comparison that
tells a working implementation from a plausible one.

**Free space.** Delegated whole to `sjel storage report --json`; this reads its verdict and
adds no thresholds of its own. Deliberately fires on the volume state alone and **not**
on a class being large: `class_flag_gb` legitimately fires today, on a 28 GB cargo target
dir, on a machine with 130 GB free and nothing wrong with it. Alerting on that would mean
the watcher's first ever run produced a finding nobody needed, which is how a watcher gets
muted. The class breakdown stays what it already was — what `sysmon storage` tells you
once you are already looking.

**Unexpected network exposure.** Delegated whole to `host-net check --json`, the same shape
the disk half already has: `capabilities/host-net` owns the scope rule, the netstat parsing and
the overlay policy of expected wildcard listeners, and this reads its verdict and adds nothing.
A wildcard bind is reachable from loopback, the local network, the tailnet and any container
bridge the host grows later, so it is the one scope worth an interrupt; a loopback listener is
not a finding and never becomes one.

One finding for the whole condition, keyed `net:unexpected-exposure`, listing every listener in
its note. Not one per port: a mesh VPN is assigned fresh wildcard ports on every start, so a
per-port key would mint a new generation every hour — the same collapse `cpu:<comm>` makes,
one step further.

A non-zero exit from `host-net` is data, not a failure: exit 1 is how it reports an unexpected
listener and exit 2 is how it reports that it could not check. If the binary has never been
built, this prints one stderr line and files nothing rather than reporting a clean host. It
calls `target/release/host-net-cli` directly and never the launcher: this tool builds nobody
else's crate, and the one build an hourly run can start is its own, which `service.toml`
declares in the open.

Memory and thermals are not checked. Both measured healthy at the incident (pressure
normal, zero swap, no thermal warning recorded in 26h) and adding a check for a condition
that has never fired manufactures alerts rather than information.

## How it reports

It writes a row into `host_watch_findings` — its own table in the shared store
(`capabilities/store`) — and nothing else. No new notification machinery: core Sjel has
never had a notifier and does not grow one here; the precedent is stated in
`tools/sjel-cli/src/sparpreis_watch/mod.rs`'s own header.

It filed a `tasks` record until PRD **Q48** (2026-08-27). That capability retired and the
Action kind went back to the vault, which is the right ruling and the wrong home for
this: a runaway process is machine state, not an action a human wrote. So the findings
stayed machine data and moved into a table this capability owns.

**Who serves them.** `sjel-status`, at `GET /api/sjel-status/host-watch`, and the
dashboard's decision ladder ranks them at band 900. This capability is a scheduled job —
it runs and exits, and the manifest schema refuses a port on a job because nothing would
be listening on it — so something always-on has to publish the rows. `sjel-status` is
that process and already answers "what is wrong with this machine"; the shape is the one
`/backups` already has, publishing receipts written by `tools/backup.sh`. Ownership does
not move with the surface: the content, the lifecycle and the table are this
capability's, and sjel-status only reads.

**One row per run of a condition.** A partial unique index keeps at most one *open*
finding per condition, so a breach that persists for a week is one row whose note is
refreshed, not seven rows. The row also carries a generation, and a new one is minted
only once every prior row for that condition is closed — otherwise a condition that
cleared in June and returned in August would upsert onto the closed row and say nothing.

**A run closes what it no longer sees.** The half that could not exist before. Under
`tasks` a finding closed only when the operator pressed Done; that button went with the
capability, so without this a row would stay open forever and the ladder would keep
ranking a process that exited months ago. The watcher owns the whole lifecycle now, which
is also the more honest one — a condition is over when it is over, not when somebody
gets around to saying so.

**One finding per command, the worst instance.** A browser runs several helper processes
under one command name. The key names the command, so those are one condition; the
highest sustained ratio is the row, and its note says how many others crossed the line.
Found by the first end-to-end run against the new table, which failed on the unique
index — `tasks` had been swallowing the duplicates silently and this tool counted a task
it never wrote.

## Commands

```
tools/host-watch              check, record findings
tools/host-watch --dry-run    check and print; write nothing
tools/host-watch --json       machine-readable findings
```

Rust since 2026-10-04, in `tools/sjel-cli/src/host_watch/`: the policy read, the three probes and
the store in `mod.rs`, the rules and their fixtures in `pure.rs`. The scheduled job runs the
built binary — `command = ["target/release/sjel-cli", "host-watch"]` — rather than a launcher, so
an hourly run starts no interpreter; `tools/host-watch` is the path a person and the tests use.

## Why it is a manifest and not a LaunchAgent

Same reason as macmon, learned the same way (Axon#65): a hand-written unit versions its
interval nowhere and needs a doctor exemption that removes the only check that would
notice it is gone. `schedule = "1h"` renders launchd or systemd through
`tools/service-runner.sh install-persistence` like everything else.

Hourly is measured, not assumed. The expensive half is the storage scan it shells out to
— 6.6s wall, almost entirely I/O wait, 0.2% of an hour. A watcher that is itself a
resource hog would be a poor joke given what it watches for.

## Machine facts live in the overlay

Every threshold and allowed process name is in
`<overlay>/config/host-watch-policy.toml`; this capability's code contains no process
name and no number (CONTRIBUTING.md#public-core-and-private-overlays). Shape:
`schemas/host-watch-policy.toml.example`.

The network half reads a second file, `<overlay>/config/host-net-policy.toml`, which belongs
to `capabilities/host-net` and is documented there. Nothing in this capability parses it.

The allowlist is meant to stay short. Every name on it is a process this watcher can
never warn about again, so an allowlist grown to silence noise is how the check quietly
stops working. Raising `min_cpu_ratio` is nearly always the better answer.

## Considered and declined

- **Killing the runaway.** Detection only. A scheduled job with kill authority over
  arbitrary host processes is a blast radius nobody asked for, and the diagnosis of
  "stuck" versus "working hard" is exactly the judgement a threshold cannot make.
- **Running `storage --apply` on a schedule.** Reclaiming stays a deliberate human
  command. Deleting a build cache mid-compile is a cure worse than a full disk.
- **A dashboard panel.** The decision ladder on home already ranks these findings, beside
  everything else waiting on a call. A second place to look is the failure this exists to
  prevent.
- **A server of its own.** It would need `autostart`, which the manifest schema refuses
  alongside `schedule`, and rightly: a watcher that holds a process up all hour to answer
  a question once is a worse joke than the resource hog it watches for.
