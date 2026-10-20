# tools/storage — what is filling this disk, and what is safe to reclaim

`df` answers "am I full". This answers "what is filling me", which `df` structurally cannot:
the 46 GB that started this tool was 215 individually unremarkable 230 MB cache blocks, and no
per-file view showed it.

The crate is `sjel-storage`, a member of the root Cargo workspace. Run it as
`sjel storage <verb>`; `tools/storage/storage` is the launcher underneath, and
`tools/sysmon storage` delegates to it. Operator machinery lives in `tools/` and its backend
logic is Rust (`Packs/harness/skills/sjel/references/on-placement.md`,
`on-dependencies-and-build.md`).

## What it measures

**The machine.** `du -sx -k` per policy class, `df -k` for the volume. `du` is the kernel's own
accounting, stays on one filesystem, and reports allocated blocks — which is what actually
frees. It also prints a partial total where an in-process walk would fail outright.

**This checkout.** An in-process walk of the Cargo target dir: `target` reports six buckets per
profile, and shelling out per bucket would walk the same tree six times. It uses `du`'s unit,
allocated blocks, and counts each inode once — cargo hard-links the finished binary from
`deps/` up to the profile root, so a naive walk reports it twice. Buckets are walked in a fixed
order, so a hard link is charged to whichever bucket reaches it first, which is what `du` does
with the first path it walks.

**This checkout, again.** `target` reports every directory under the repo root carrying a
`CACHEDIR.TAG` that is not the one cargo resolves to here. They appear because a `cargo` launched
without the shell environment does not see `CARGO_TARGET_DIR` — Xcode's build phase is the case
that produced one. One directory is deliberately never reported, the checkout's own `target/`:
the supervisor runs binaries out of it (`capabilities/calendar/service.toml` names
`target/release/calendar-server`), which is why `tools/cargo-hermetic` refuses to point
`CARGO_TARGET_DIR` inside the checkout. Listing it as reclaimable would invite the mistake that
guard exists to prevent. A second Cargo target dir under the repo root is a *nested* workspace,
which the checkout's own `target/` never accounts for.

`reclaim --repo --target` cleans a secondary directory through its own manifest, with
`--target-dir` naming it. The flag is load-bearing: scrubbing `CARGO_TARGET_DIR` is not enough,
because a machine-wide `[build] target-dir` in a cargo config would then redirect cargo at the
primary directory, clean that instead, and report success.

## Reporting and reclaiming

Two surfaces, and the split is the point: one reads, the other removes, and only the second
needs a guard.

| Verb | Reads | Answers |
|---|---|---|
| `report [--json]` | the overlay policy | free/used/total, every class against its cap, what is over the flag, what is protected, which entries are gone, which paths exist but could not be read, and how much space no entry accounts for |
| `discover [--top N] [--depth D] [--root PATH] [--json]` | the overlay policy | the largest directories under `PATH` (default `$HOME`) that no policy path names — where `unattributed` lives |

| Verb | Acts on | Answers |
|---|---|---|
| `reclaim [--dry-run] [--force] [--scheduled] [--only <class>]` | the policy's classes | each applicable class's reclaim, gated by its cap or its age, refusing anything a live process holds or the policy protects |
| `reclaim --repo [--incremental] [--target] [--node-modules]` | the checkout's build output | the same, for target dirs and `node_modules` |
| `target [--json]` | nothing | the Cargo target dir per profile and bucket, the R6 ratio, and whether the artifacts match the installed rustc |

`report --where [--root PATH]` adds the walk `discover` prints, under the `unattributed` figure
it explains. The walk is the expensive half — 35 s under `$HOME`, 45 s over the whole volume — so
neither surfaces it unasked.

`apply` and `prune` are **spellings of `reclaim`**, not separate commands. Both were reclaim, and
each grew its own answer to "may I delete this": `apply` ran a class's reclaim without asking
whether a process was using the path, and `prune` refused paths the policy owned while offering
to `cargo clean` the very directory the policy called report-only. Keeping the names means a
finding already written that says `--apply` still resolves; keeping them as *aliases* means one
guard, one plan format and one exit code behind both.

`--repo` replaces the class scope rather than adding to it, and is refused alone: a
`reclaim --repo --incremental` that quietly also cleared the app caches is not something to
discover afterwards. `--root` is refused without `--where`, and `--root /` is refused because
`/System/Volumes/Data` is reached through a firmlink rather than a mount point, so `du -x /`
counts the Data volume inside `/System` instead of stopping at the boundary.

### The two gates, and why only one of them is an error

`skipped` is a class held back by its own policy — under its cap, or unbounded on a schedule.
Nothing went wrong, so it does not touch the exit code; it is printed because a scheduled run
that silently did nothing is indistinguishable from one with nothing to do. `refused` is the
guard declining, and that is exit 1.

Exit 0 means nothing is over a threshold. Exit 1 means something is: free space below
`free_critical_gb`, an R6 ratio above 3×, or a reclaim path that had to be refused. Exit 2 means
it could not measure — a usage error, or no overlay policy to read for a scope that needs one.

### The size axis

`cap_gb` gates `reclaim`: a capped class acts only when it is measured above the cap, and a class
with no cap acts whenever it is asked. `older_than_days` removes the entries *inside* each path
older than the age rather than the path itself, and is a reclaim mechanism on its own — a class
using it needs no `reclaim`. The cheap way to hold a directory down without a full rebuild;
the alternative is dropping all of it at the cap, over and over.

Together they are what makes a class safe to run unattended, so `reclaim --scheduled` acts only
on classes the policy itself bounds, by size or by age. An unbounded class is a person's
decision, and that is what a bare `reclaim` is for. `capabilities/cargo-trim` runs
`reclaim --scheduled` daily.

## The report's own blind spot

**`named` is a ceiling, not a measurement.** `unattributed` is `used` minus everything every
class and every protected path accounts for, so it includes the OS, `/Applications` and other
users; and `named` over-counts a class that only partly sits inside a protected path. It is
deliberately not an exit-code trigger: exit 1 is a contract `tools/host-watch` reads, and "this
policy is incomplete" is a different alarm from "this disk is full".

**A vanished path is not a zero-byte one.** A class whose paths are all gone is printed under
`Declared but absent` rather than skipped, because skipping makes a dead entry quieter than a
live one. A protected path that is gone prints `absent` instead of `0 MB`, so the reason beside
it is not mistaken for a current justification. How long one has been gone is a history question,
not a report's — that record belongs to the surface that samples this report, not to this crate.

**A partial reading is printed as partial.** `du` exits non-zero and still prints a total for
what it read when it meets a subdirectory it cannot enter; that total is `measured: false` in
JSON and `[partially measured]` in text, and a class that returned nothing gets its own
`Unmeasured` section instead of being skipped by the size list. `discover` follows the same rule
and prints `PARTIAL` with the first error, because a lower bound presented as a size is the
failure this whole section is about.

**Expected services are checked, not repeated.** Each policy entry is probed by its `kind`
(`process`, `launchagent`, `apple-container`), and the answer is `running`, `ABSENT`, or
`unverifiable` when the probe's own tool is not installed or the `kind` is one this build does
not know. The third state is the point: a policy written for another init system must not read as
a stopped service.

`--json` is additive throughout. `named_bytes`, `unattributed_bytes`, `measured`, `present`,
`cap_bytes`, `over_cap`, `presence` and `where` all arrived after the field names
`tools/host-watch` reads, so a reader that predates one sees what it always saw.

## Where the unattributed space is

`discover` ranks the largest directories under a root that no policy path names, which is the
question `report` can pose and not answer. A directory at the requested depth is reported with
what is left after the named paths inside it, and one that is itself named — or lies inside a
named path — is dropped, because none of it is unnamed. A named path nested inside another is
subtracted once, not twice.

Depth is the partition: `du` walks the whole tree at any depth, so the flag changes the answer's
shape, not its cost. The default root is `$HOME`, where the report's `unattributed` covers the
whole Data volume and this covers one tree; `--root /System/Volumes/Data` is the walk over the
scope that figure does. It exits 0 whatever it finds, because "here is where your space went" is
not the alarm exit 1 carries.

## Growth, which a report cannot show

`report` is a point and "is it growing" is a slope, so this crate measures, prints and exits
without keeping a record. The record is kept by the surface that is always up, which samples this
report into the shared database and serves the trend; see that capability's own README.

## The policy contract

Every path, threshold and reclaim command is a deployment fact and lives in the overlay at
`<overlay>/config/storage-policy.toml`. Nothing in this crate names a path on this machine
(CONTRIBUTING.md#public-core-and-private-overlays). `schemas/storage-policy.toml.example` is the
shape, and a unit test deserialises it so the template cannot rot away from the parser. Each class
carries `name`, `paths`, `reclaim`, `apply`, `regrows`, `note`, and the two size-axis fields.

Three rules keep reclaim safe, all tested:

1. A class is touchable only when the policy both allows it and says how. `apply = true` with
   neither a `reclaim` nor an `older_than_days` is a policy bug, not a licence to guess — and an
   age counts as saying how, because removing entries by age is a mechanism this tool owns rather
   than a command the policy hands over.
2. A policy-supplied string is data, not code. The literal `reclaim = "rm -rf"` deletes the paths
   this tool measured, never a path the policy named. Anything else runs verbatim, because it is
   a named tool's own cleanup verb — `brew cleanup`, `cargo clean` — that only its own CLI can
   express.
3. A class may name a path inside another class, which is how a size gate and an age gate cover
   the same tree. The report subtracts the inner one from `reclaimable` and says so, so the number
   a reader acts on is not an overstatement.

The overlay is resolved through `SJEL_PERSONAL_ROOT`, the same way `capabilities/host-net`
resolves its own policy. The `sjel.local.toml` → `sjel.toml` order stays owned by
`tools/lib/paths.sh` and `libs/overlay/overlay.ts`, and the launcher sources `paths.sh`.

## R6

PRD §9's R6, ratified as Q53: build artifacts are not state, and `target/debug` may not exceed
`target/release` by more than 3×. §9 rejected an absolute cap on its own stated grounds — a GB
figure would be "a guess wearing a gate's clothes", and it would need revising the first time a
bigger disk arrives. A ratio does not. `tools/doctor` reads `sjel storage target --json` rather
than re-deriving the walk.

`target` also reports the compilation-unit count per profile beside the ratio. Q53's argument for
`target/release` being a valid control is "same crates, same machine, same moment", and a checkout
with a full debug build and a partial release build produces a large ratio with nothing wrong; the
unit counts are what show that. They are deliberately not a gate: a threshold on them would be
exactly the guess §9 refuses.

**What a ratio cannot see.** A clean full build after a toolchain roll leaves the previous
generation's output in `deps/` while both profiles carry it equally, so no ratio detects it. What
does is `target/.rustc_info.json`, where cargo caches the `rustc -vV` output it saw: when the
recorded commit hash differs from the rustc this machine runs today, every artifact beside it was
produced by a compiler that is gone. **The detector has a narrow window** — cargo rewrites that
file on its first run after the roll and does not delete the previous generation — so a match
after that means "cargo has run since the roll", not "the tree is clean", and `target` prints it
in those words. It reports the mismatch and the size of `deps` plus `.fingerprint` across both
profiles. Attributing individual files to a toolchain would need a per-file signal this tool does
not have, and inventing one that is merely plausible would be worse than the honest gap.

## What it refuses to do

Every removal goes through one guard, and only some of what it says is negotiable.

**Structural, and `--force` never reaches these.** Each one means the tool does not know what it
is being asked to remove, and a flag is the wrong answer to not knowing:

- A path that resolves outside the repo root and the Cargo target dir. The check runs after
  `canonicalize`, so a `node_modules` that is a symlink to a shared store fails it on the resolved
  path, not the one that was passed in.
- A path tracked by git. The `--node-modules` list is derived from
  `git status --ignored --short -z`, never a hardcoded name, and each candidate is re-checked
  against `git ls-files` before removal.
- The repo root itself, and the Cargo target dir as a recursive delete. The target dir is a
  candidate only through `cargo clean`, which is cargo's own verb for a directory whose layout
  cargo owns.

**Judgement, and `--force` overrides these.** They are calls the policy and the machine have
already made, so the reader is given the reason and a way to disagree:

- A path the policy protects, *or that would take a protected path with it*. The containment half
  is the load-bearing one: `cargo clean` on `~/.cargo-target` deletes `release/`, where `sjel-mcp`
  runs from, and the protection naming `release/` is what refuses it. A candidate *inside* a named
  path is still prunable, which is what keeps a subdirectory sweep possible inside a report-only
  directory.
- A path a policy class names, when something other than that class is asking. A class reclaiming
  its own path is the policy acting; a sweep over it is not.
- A directory a live process has open. The refusal names the pid and what it holds. A probe that
  cannot run refuses too, because "could not ask" must not read as "nothing is using it" — and the
  process asking is subtracted from its own answer, so a run does not refuse itself.

And one it will not do at all: run a reclaim command for a class the policy marked report-only, or
run a bare `rm -rf` when nothing was measured. A policy string is data, not code, and the one case
where it reaches a shell substitutes only the paths this tool measured.

`reclaim --json` is refused rather than answered with a report, because a caller that meant to act
would read a report as success. `report --json` reads the report; `reclaim --dry-run` shows the
plan.

Nothing here writes to the overlay, and nothing here reads a secret.
