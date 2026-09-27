# container-refresh

The daily pull of every image this machine declares, and the recreate for the ones whose digest
moved. `Q77` (2026-09-02) turned every `service.toml` `tag` into a rolling channel —
`:stable` for Home Assistant, `:latest` for Pi-hole, `:alpine` for Vaultwarden. A channel moves in
the registry and nothing on the host notices, because `docker run` resolved the digest once, at
creation. This capability is what makes the channel real.

It is the container half of `capabilities/host-patch`: that one moves `brew`, `uv` and `rustup`
every 24 hours, this one moves the images, and both write a receipt `tools/doctor` reads.

## Why this shape

**Build, and it is one script over the runtime already on the machine.** There is a whole class of
tool for this — Watchtower, podman-auto-update, Renovate against a digest — and each one adds a
scheduler, a config format and a daemon that has to be trusted with the docker socket. Axon
already has the three things such a tool would bring: a manifest that declares the image
(`service.toml`), a runner that recreates a container from that declaration
(`tools/service-runner.sh recreate`), and a scheduler that renders launchd and systemd units from
a `schedule` field. What was missing is the loop between them, which is `tools/container-refresh.sh`
and about a hundred lines.

**The digest decides, never the tag.** `docker pull ghcr.io/…:stable` is cheap and answers
"unchanged" most days. The script records the local digest
(`image inspect --format '{{index .RepoDigests 0}}'`) before and after the pull and recreates only
on a difference. That is also the honest reading of what a rolling tag costs: the declared tag no
longer says what is running, so the digest is the only version fact, and `ISA.md` C4 says so for
every reader.

**A `recreate` is a real interruption, so it is the narrow case.** `recreate` stops the container,
removes it and starts a new one from the current declaration. Declared state survives by
construction — every state path is a named volume or a host mount — and undeclared in-container
state does not, which is the rule `CONTRIBUTING.md#state-mounts-record-reality` already sets.

**It will not restart what somebody stopped.** `recreate` clears the maintenance hold on its way
through (`tools/service-runner.sh` `recreate_service`), and `start_service` brings the capability
up. So a held or stopped capability gets its image pulled and is then left alone, named in the
receipt as `<cap>:held` or `<cap>:not-running`. The new image applies at the next
`tools/service-runner.sh recreate <cap>`, run by whoever ended the hold. Doing otherwise would let
a scheduled job overturn an operator's decision at 03:00.

**No scanning here.** `grype` reads every declared image from the registry in
`.github/workflows/security.yml`, which runs whether or not this host is awake. Since Q77 its
findings are report-only, so a red upstream image is visible on every run without holding a merge.
The job itself still goes red for one thing: a discovery loop that found no image to scan.

## What it does on a machine with no containers

Writes a receipt saying `no-container-capabilities` and exits 0. That is the expected result on a
workstation, and it is a receipt rather than silence so `tools/doctor` can tell "nothing to
refresh" apart from "the job stopped firing".

The same applies one level down: a machine whose `container_runtime` binary is not installed
records `<runtime>-not-installed` and exits 0. A `container_runtime` that is neither `docker` nor
`podman` is a different thing — a `machine.toml` defect — and exits 2.

## The receipt

`<overlay>/data/container-refresh/last.json`: the timestamp, which capabilities were recreated,
which were skipped and why, and which failed and at which step. `tools/doctor` reads it and
reports its age, warning past 48 hours. That check is the point of the file: a scheduled job's
real failure is that it quietly stops running, and the launchd unit stays loaded either way.

## Enabling it

```
tools/capability.sh enable container-refresh
tools/service-runner.sh install-persistence container-refresh
```

In that order, for the reason `capabilities/host-patch/README.md` gives: enabling second leaves
`tools/doctor` calling the rendered unit an orphan and advising `remove-persistence`, which is the
wrong advice.

Run it by hand once first. `RunAtLoad` fires the unit immediately on install, and the first run is
the one that has a real pull in it.

## Done looks like

- On a host with no container capability enabled, the run exits 0 and the receipt says
  `no-container-capabilities`.
- With a runtime whose `pull` fails for one image, the images after it are still pulled, and the
  run exits 2 with that capability named in the receipt.
- An unchanged digest recreates nothing.
- A moved digest recreates the running container, and leaves a held one alone.
- `tools/doctor` reports the receipt's age, and warns once it is over 48 hours old.
