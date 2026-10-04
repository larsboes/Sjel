# host-patch

The daily upgrade of this machine's tools, and the audit that follows it — over this repository,
the private overlay, and the software those tools installed outside the checkout. Axon held a new
release for seven to fourteen days before adopting it; Q74 removed
that hold on 2026-09-02 (CONTRIBUTING.md#patch-first), and this is what stands in its place. Nothing
waits any more, so something has to move: `brew`, `uv` and `rustup` run every 24 hours, then
`tools/audit`, then a receipt that says what happened.

## Why this shape: build, and no second scheduler

**Build**, and it is ninety lines of shell over three package managers. It adopts nothing.
`brew`, `uv` and `rustup` are already `toolchain.toml` entries, and no scheduler is added
either — `tools/service-runner.sh` renders launchd units from a manifest already, which is why
this capability is a manifest and not a plist. `tools/host-patch.sh` is the whole implementation,
and what it holds is the order of the steps, the guard in front of each one, and the receipt.

## What it does not upgrade, and why

Casks that ship their own updater ARE upgraded, since 2026-09-02: `brew upgrade --cask
--greedy` (Q77). Without `--greedy` brew skips every cask marked `auto_updates` or
`version :latest` and reports success over them, which is a patch run that patches nothing and
says it did. A cask that must stay put is a `brew pin` — one owner, one mechanism, the same
rule the paragraph below states.

`bun` and `uv` both have self-update verbs, and neither is called. Both are Homebrew formulae on
this host (`toolchain.toml` `[bun]`, `[uv]`), so `brew upgrade --formula` already moves them, and
two owners of one binary is a failure this deployment has already paid for: a `~/.local/bin`
`yt-dlp` shadowed brew's copy on `PATH` and returned HTTP 403 on every media URL while
`--dump-json` kept working (PRD §13). If `bun` should stay put on this machine, that is a
`brew pin`, not a special case in the script — one owner, one mechanism.

Container images are neither pulled nor scanned here. `capabilities/container-refresh` is the
pull, on the hosts that run containers; `grype` is the scan, in
`.github/workflows/security.yml`, which installs it itself and runs whether or not this Mac is
awake.

## The half this job does not cover

Everything above is a class this job or `container-refresh` already owns. The classes nothing
owned were invisible until 2026-10-01, when four `cargo install`ed crates and eleven global npm
packages were found behind and the two upstream harness integrations had gone three weeks since
upstream's installer last wrote them. `sjel update` is that surface: it reports every class with
its owner named, and `apply` moves only what nothing else moves — `cargo install`ed crates and
`npm -g` packages — while sending brew, uv and rustup straight back through this script. It
deliberately does not reimplement a step below, for the one-owner reason this file states twice
already. See `tools/updates.ts` and ISA.md F11.

## The cadence, honestly

A launchd `StartInterval` job does not fire while the Mac sleeps; it runs once on wake. So "every
24 hours" means at most one run per waking day, and a machine closed for a long weekend patches
on Monday. `RunAtLoad` in `tools/templates/launchd-schedule.plist.tmpl` means installing the unit
fires a run immediately — which is why the first run is worth doing by hand, where its output can
be read.

## The second job

It runs `tools/audit` last, over a machine that was just patched, so the verdict describes today
rather than yesterday. That also means the private overlay's secret scan — the one surface no CI
can reach — happens daily instead of when somebody thinks of it.

## The receipt

`<overlay>/data/host-patch/last.json`: the timestamp, which steps ran, which were skipped because
the binary is not installed, which failed, and the audit's verdict. `tools/doctor` reads it and
reports how old it is. That check is the point of the file. A scheduled job's real failure mode is
that it quietly stops running, and without a receipt nothing at all would notice — the launchd
unit would still be loaded, and `doctor` would still be green.

## Enabling it

```
tools/capability.sh enable host-patch
tools/service-runner.sh install-persistence host-patch
```

In that order. Enabling first is what makes the rendered unit owned: skip it and `tools/doctor`
reports the unit as an orphan and advises `remove-persistence`, which is the exact wrong advice
recorded in `capabilities/backup/service.toml`.

## Done looks like

- `tools/host-patch.sh` on a machine with no `brew`, `uv` or `rustup` exits 0 and reports each
  step as skipped, never as failed.
- A failing upgrade step does not stop the steps after it, and the run exits 2 with the step
  named in the receipt.
- `tools/doctor` reports the receipt's age, and warns once it is over 48 hours old.
- `sjel update` reports this job's receipt as one row among the classes, and never as a claim
  that the audit was clean.
