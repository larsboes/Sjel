# tools/storage — what is filling this disk, and what is safe to reclaim

`sysmon report` answers "am I full" with one `df` line. This answers "what is filling me",
which `df` structurally cannot. The 46 GB that started this tool was 215 individually
unremarkable 230 MB cache blocks. No per-file view showed it.

The crate is `sjel-storage`, a member of the root Cargo workspace. Operator machinery lives
in `tools/` and its backend logic is Rust, so `tools/` holds a Cargo member
(`Packs/harness/skills/sjel/references/on-placement.md`,
`Packs/harness/skills/sjel/references/on-dependencies-and-build.md`). Run it as
`sjel storage <verb>`; `tools/storage/storage` is the launcher underneath, and
`tools/sysmon storage` still delegates to it.

## What it measures

Two questions, and they use different instruments.

**The machine.** `du -sx -k` per policy class, `df -k` for the volume. `du` is the kernel's
own accounting, it stays on one filesystem, and it reports allocated blocks — which is what
actually frees. A class may point at a directory this process cannot read, and `du` reports
a partial number where an in-process walk would fail outright.

**This checkout.** An in-process walk of the Cargo target dir, because `target` reports six
buckets per profile and shelling out per bucket would walk the same tree six times. It uses
the same unit `du` does, allocated blocks, and it counts each inode once — cargo hard-links
the finished binary from `deps/` up to the profile root, so a naive walk reports it twice.
Buckets are walked in a fixed order, so a hard link is charged to whichever bucket reaches
it first. That is what `du` does with the first path it walks.

**This checkout, again.** Cargo stamps its target dir with a `CACHEDIR.TAG`, and `target`
reports every such directory under the repo root that is not the one cargo resolves to here.
They appear because a `cargo` launched without the shell environment does not see
`CARGO_TARGET_DIR` — Xcode's build phase is the case that produced one, and
`dashboard/src-tauri/target` had reached 15 GB before anyone noticed (measured 2026-09-30).

One directory is deliberately never reported: the checkout's own `target/`. That is not an
escapee, it is where a workspace build puts the binaries the supervisor runs
(`capabilities/calendar/service.toml` names `target/release/calendar-server`), which is why
`tools/cargo-hermetic` refuses to point `CARGO_TARGET_DIR` at anything inside the checkout.
Listing it as reclaimable would invite the mistake that guard exists to prevent. A second
Cargo target dir under the repo root is a *nested* workspace, which the checkout's own
`target/` never accounts for.

`prune --target` cleans each secondary directory through its own manifest, with `--target-dir`
naming it. The flag is load-bearing: scrubbing `CARGO_TARGET_DIR` is not enough, because a
machine-wide `[build] target-dir` in a cargo config would then redirect cargo at the primary
directory, clean that instead, and report success.

Before anything is removed, every candidate is asked of the machine: `lsof +D` names the
processes with a file open under it, and one that is held is refused rather than cleaned
(`inuse.rs`). The total printed below the lines counts only what survived, so a refusal
cannot leave the one number a reader acts on overstated by exactly the amount refused.

Then it is asked of the policy, which is the end of the two-answers problem. `prune` translates
no policy itself — the caller resolves the overlay into an `Authority` (every protected path and
every class path, with the sentence explaining it) and hands it over, so a clone with no overlay
passes an empty one and the verb behaves as it always did. A candidate the policy names, *or that
would delete a path the policy names*, is refused with the policy's own reason. The containment
half is the load-bearing one: `cargo clean` on `~/.cargo-target` deletes `release/`, where
`sjel-mcp` runs from, and the policy class that says so is the reason the sweep was the wrong
answer to that question. A candidate *inside* a named path is still prunable, which is what keeps
`sjel storage prune --incremental` usable inside a report-only target directory. `--force` is the
only way past it.

## Reporting and reclaiming

Two surfaces, and the split is the point: one reads, the other removes, and only the second
needs a guard.

| Verb | Reads | Answers |
|---|---|---|
| `report [--json]` | the overlay policy | free/used/total, every class with its size against its cap, what is over the flag, what is protected, which entries name something that is gone, which paths exist but could not be read, and how much space no entry accounts for |
| `discover [--top N] [--depth D] [--json]` | the overlay policy | the largest directories under `$HOME` that no policy path names — where the `unattributed` figure lives |
| `target [--json]` | nothing | the Cargo target dir per profile and per bucket, the R6 ratio, and whether the artifacts match the installed rustc |

| Verb | Acts on | Answers |
|---|---|---|
| `reclaim [--dry-run] [--force] [--scheduled] [--only <class>]` | the policy's classes | runs each applicable class's reclaim — gated by its cap or its age, refusing anything a live process holds or the policy protects |
| `reclaim --repo [--incremental] [--target] [--node-modules]` | the checkout's build output | the same, for target dirs and `node_modules` |

`apply` and `prune` are **spellings of `reclaim`**, not separate commands. They were both
reclaim, differing only in which candidates they looked at, and each grew its own answer to
"may I delete this" — `apply` ran a class's reclaim verb without asking whether a process was
using the path, and `prune` refused paths the policy owned while offering to `cargo clean` the
very directory the policy called report-only. Keeping the names means a finding already written
that says `--apply` still resolves; keeping them as *aliases* means there is one guard, one plan
format and one exit code behind both.

`--repo` replaces the class scope rather than adding to it. A `reclaim --repo --incremental`
that quietly also cleared the app caches is not something to discover afterwards. `--repo` alone
is refused, and still requires naming which part of the checkout to act on.

### The two gates, and why only one of them is an error

`skipped` is a class held back by its own policy — under its cap, or unbounded on a schedule.
Nothing went wrong, so it does not touch the exit code; it is printed because a scheduled run
that silently did nothing is indistinguishable from one with nothing to do. `refused` is the
guard declining, and that is exit 1.

Exit 0 means nothing is over a threshold. Exit 1 means something is: free space below
`free_critical_gb`, an R6 ratio above 3×, or a reclaim path that had to be refused. Exit 2
means it could not measure — a usage error, or no overlay policy to read for a scope that needs
one.

### The size axis

`apply` answered "may I" and never "is it too big", and the policy had no way to say the
second. Every incident that built this tool was the second question — 46 GB of KV cache, 149 GB
of target, 130 GB of debug — and the daily trim of `~/.cargo-target/debug` therefore lived in a
40-line shell script outside this crate, carrying its own `du`, its own `pgrep` guard and its
own 50 GB cap. `cap_gb` and `older_than_days` are that axis:

* **`cap_gb`** gates `apply`. A capped class acts only when it is measured above the cap; a class
  with no cap acts whenever it is asked, which is what every class did before this existed.
* **`older_than_days`** removes the entries *inside* each path older than the age, rather than
  the path itself, and is a reclaim mechanism on its own — a class using it needs no `reclaim`.
  The cheap way to hold a directory down without a full rebuild: the alternative is dropping all
  of it at the cap, over and over.

Together they are what makes a class safe to run unattended, so **`reclaim --scheduled` acts
only on classes the policy itself bounds**, by size or by age. An unbounded class is a person's
decision, and that is what a bare `reclaim` is for. `capabilities/cargo-trim` runs
`reclaim --scheduled` daily, which is how the shell script's three guards became this crate's
one guard and its cap became a line in the policy beside the others.

## What the report says about its own blind spot

Every figure above is a thing the policy named, and for a long time that was the whole report.
It could not say what it was missing, which is the worse failure: on 2026-10-11 the disk was
96% full, `storage report` answered "1.3 GB reclaimable", and `~/.cargo-target` was holding
**149 GB** that no class named. Nothing was wrong with the arithmetic. The policy was simply
incomplete, and a tool that only totals what it was told about has no way to say so.

Two lines fix that, and both are the reason to prefer `report` over `df`:

* **`N GB named by policy · M GB unattributed`** — `used` minus everything every class and
every protected path accounts for. That figure was ~375 GB on the day above. It is a ceiling
on the gap rather than a measurement of one thing (`used` covers the OS, `/Applications` and
other users too), and `named` can over-count a path that is both a class and a protection. A
class that sits entirely inside a protected path is counted once now; one only partly inside
one still over-counts, which is why `named` stays a ceiling rather than a measurement. It is
deliberately not an exit-code trigger: exit 1 is a contract `tools/host-watch` reads, and
"this policy is incomplete" is a different alarm from "this disk is full".
* **`Declared but absent`** — a class whose paths have all vanished used to be skipped, which
is right for an empty directory and wrong for a path that is gone, because it made a dead
entry quieter than a live one. A protected path that is gone now prints `absent` instead of
`0 MB`, so the reason beside it is not mistaken for a current justification. Four of six
protected entries were stale when this was added. How long one has been gone is not a
report's to answer — a report is a point — so the hourly record is kept by
`capabilities/sjel-status` (`storage_absent`, one row per gone name per hour) and read back as
"absent since"; see that README.

`--json` gained `named_bytes` and `unattributed_bytes`, and each protected entry gained
`present`. Additive, like `expected_service` before them. Two more arrived on the same rule,
and both close a section that could print a number or a claim meaning nothing:

* **`Expected running` is checked, not repeated.** The section used to print the policy's
declaration under "(not findings)", and on 2026-10-11 two of its three entries carried a note
that contradicted the line above it — "nothing named postgres is running", "not running…
Retired" — while the report printed both. Each entry is now probed by its `kind` (`process`,
`launchagent`, `apple-container`), and the answer is `running`, `ABSENT`, or `unverifiable`
when the probe's own tool is not installed or the `kind` is one this build does not know. The
third state is the point: a policy written for another init system must not read as a stopped
service, which is the same rule as "absent thresholds never manufacture an alarm". A
launchagent declared where a plain process runs is a policy bug the check exposes rather than
hides — `macmon` was one, and its entry left the policy the day this landed.
* **A class that exists but cannot be read no longer disappears.** `du` exits non-zero and
still prints a total for what it read when it meets a subdirectory it cannot enter, so that
total was a lower bound presented as a size. It is now `measured: false` in JSON and
`[partially measured]` in text when some bytes came back, and a class that returned nothing at
all gets its own `Unmeasured` section instead of being skipped by the size list (which is
right for an empty directory and was wrong here, exactly as it was for a vanished path).

`--json` gained `measured` on each class, and `presence` · `running` · `reason` on each
expected service. `present` arrived on classes with them, so "the path is gone" stopped
being the same number as "it is here and measures 0" — the distinction `protected` gained a
day earlier. Additive, and a reader that predates them sees what it always saw.

`apply` gained `--dry-run`, which prints the argv each applicable class would run and the
total it would free, and executes nothing. `apply --json` is now refused rather than
silently answered: `--json` returned the report before the apply step, so it printed a
report and deleted nothing, which a caller that meant to apply would have read as success.
`report --json` reads the report; `apply --dry-run` shows the plan.

`report` and `apply` reproduce the contract `tools/storage.ts` had, including the `--json`
field names, so `tools/host-watch` and any overlay hook read the same shape they always
did. `--json` has gained keys since — `expected_service` was the first, when the text
report printed it and a JSON reader had no way to see it — and every one is additive, so a
reader that predates it sees what it always saw.

## Where the unattributed space is

`report` says how much of the disk no entry accounts for; it cannot say which directories.
`discover` answers that one level down, and it is the verb the 2026-10-11 incident asked
for: `~/.cargo-target` held 149 GB that no class named, and the tool written to find it
reported "1.3 GB reclaimable".

One `du -x -k -d <depth>` walk of `$HOME` (default depth 2, top 20), then every policy path
measured on its own so its bytes can be subtracted from whichever directory contains it. A
directory at the requested depth is reported with what is left after the named paths inside
it, and one that is itself named — or lies inside a named path — is dropped, because none of
it is unnamed. Depth is the partition: `du` walks the whole tree at any depth, so the flag
changes the answer's shape, not its cost. A named path nested inside another is subtracted
once, not twice.

Two things it says out loud rather than smoothing over:

* **It is scoped to `$HOME`.** The report's `unattributed` is the whole Data volume, so it
also covers `/Applications`, `/Library` and other users. A run on 2026-10-12 read 104 GB
unnamed under `$HOME` against the report's 190 GB, and the difference is exactly that
space — not a disagreement between the two figures.
* **A partial walk is reported as partial.** `du` exits non-zero and prints a lower bound
when it meets a directory it cannot read; on this machine those are the cloud-backed ones
(`~/Library/CloudStorage`, iCloud Drive), where it writes `Resource deadlock avoided`. The
scan prints `PARTIAL` and the first error rather than presenting the total as a size — the
same rule a class measurement follows with `measured`.

`discover` exits 0 whatever it finds: it is a report, not a threshold. Exit 1 is the
contract `tools/host-watch` reads, and "here is where your space went" is not that alarm.

## Growth, which a report cannot show

`report` is a point; "is it growing" is a slope, and this tool keeps no record — it
measures, prints and exits. `sjel-status` samples this report once an hour into
`storage_samples` in the shared database and serves it at
`/api/sjel-status/storage-history`; the Systems page draws the trend. 90 days are kept,
and the hour is the primary key so a restart cannot add a second row for the same hour.
The sampler is the only writer, and the measurement it stores is the one this file
documents — including `named_bytes` and `unattributed_bytes`, which is why a report from
a build that predates them is refused rather than recorded as zero.

## The policy contract

Every path, threshold and reclaim command is a deployment fact and lives in the overlay at
`<overlay>/config/storage-policy.toml`. Nothing in this crate names a path on this machine
(CONTRIBUTING.md#public-core-and-private-overlays). `schemas/storage-policy.toml.example` is the
shape, and a unit test deserialises it so the template cannot rot away from the parser. Each
class carries `name`, `paths`, `reclaim`, `apply`, `regrows`, `note`, and the two size-axis
fields above; `[[protected]]` and `[[expected_service]]` are unchanged.

Three rules keep reclaim safe, all tested:

1. A class is touchable only when the policy both allows it and says how. `apply = true` with
   neither a `reclaim` nor an `older_than_days` is a policy bug, not a licence to guess — and an
   age counts as saying how, because removing entries by age is a mechanism this tool owns rather
   than a command the policy hands over.
2. A policy-supplied string is data, not code. The literal `reclaim = "rm -rf"` deletes the paths
   this tool measured, never a path the policy named. Anything else runs verbatim, because it is a
   named tool's own cleanup verb — `brew cleanup`, `cargo clean` — that only its own CLI can
   express.
3. A class may name a path inside another class, which is how a size gate and an age gate cover
   the same tree. The report subtracts the inner one from `reclaimable` and says so, so the
   number a reader acts on is not the 210.9-GB-for-150-GB mistake one level up.

The overlay is resolved through `SJEL_PERSONAL_ROOT`, the same way
`capabilities/host-net` resolves its own policy. The `axon.local.toml` → `axon.toml` order
stays owned by `tools/lib/paths.sh` and `libs/overlay/overlay.ts`; the launcher sources
`paths.sh`, so this crate holds no third copy of it.

## R6

PRD §9's R6, ratified as Q53 on 2026-08-28: build artifacts are not state, and
`target/debug` may not exceed `target/release` by more than 3×. §9 rejected an absolute cap
on its own stated grounds — a GB figure would be "a guess wearing a gate's clothes", and it
would need revising the first time a bigger disk arrives. A ratio does not.

Q53 named `tools/doctor` as the checker. Doctor had no such section until 2026-09-03; it
has one now, and it reads `sjel-storage target --json` rather than re-deriving the walk.

`target` reports the compilation-unit count per profile beside the ratio. Q53's argument for
`target/release` being a valid control is "same crates, same machine, same moment", and a
checkout with a full debug build and a partial release build produces a large ratio with
nothing wrong. The unit counts are what show that. They are deliberately not a gate: a
threshold on them would be exactly the guess §9 refuses.

## What a ratio cannot see

Measured 2026-09-03 on this workspace, before a `cargo clean`: `target/` was **21 GB**, of
which `debug/deps` was 13 GB and `debug/incremental` 7 GB, accumulated across toolchain
rolls. A clean full `cargo build --workspace` rebuilt the same tree as **4.7 GB in 49 s**.
The bulk was output from rustc versions no longer installed.

No ratio detects that, because both profiles carry it equally. What detects it is
`target/.rustc_info.json`, where cargo caches the `rustc -vV` output it saw. When the
recorded commit hash differs from the `rustc -vV` this machine runs today, every artifact
beside it was produced by a compiler that is gone, and a clean is warranted.

**The detector has a narrow window, and saying so is the point.** On 2026-09-05 rustup
rolled stable from 1.98.0 (88d9e12ae) to 1.98.1 (48a229cea) while this crate was being
written. Cargo rewrote `.rustc_info.json` to the new hash on its first run afterwards, and
it did not delete the previous generation's output from `deps/`. So the mismatch is visible
only between the roll and the next `cargo` invocation. A match after that means "cargo has
run since the roll", not "the tree is clean", and `target` prints it in those words.

This reports the mismatch and the size of `deps` plus `.fingerprint` across both profiles.
It does not attribute individual files to a toolchain. Detecting accumulation after the
window has closed would need a per-file signal this tool does not have, and inventing one
that is merely plausible would be worse than the honest gap.

## What it refuses to do

Every removal goes through one guard, and only some of what it says is negotiable.

**Structural, and `--force` never reaches these.** Each one means the tool does not know what it
is being asked to remove, and a flag is the wrong answer to not knowing:

- A path that resolves outside the repo root and the Cargo target dir. The check runs after
  `canonicalize`, so a `node_modules` that is a symlink to a shared store fails it — on the
  resolved path, not the one that was passed in.
- A path tracked by git. The `--node-modules` list is derived from
  `git status --ignored --short -z`, never a hardcoded name, and each candidate is re-checked
  against `git ls-files` before removal.
- The repo root itself, and the Cargo target dir as a recursive delete. The target dir is a
  candidate only through `cargo clean`, which is cargo's own verb for a directory whose layout
  cargo owns.

**Judgement, and `--force` overrides these.** They are calls the policy and the machine have
already made, so the reader is given the reason and a way to disagree:

- A path the policy protects, *or that would take a protected path with it*. The second half is
  the load-bearing one: `cargo clean` on `~/.cargo-target` deletes `release/`, where `sjel-mcp`
  runs from, and the protection naming `release/` is what refuses it. On 2026-10-13 the policy
  called that directory report-only while `reclaim --repo --target` offered to clean 18.1 GB of
  it — two answers to one question, and the sweep was the wrong one.
- A path a policy class names, when something other than that class is asking. A class reclaiming
  its own path is the policy acting; a sweep over it is not. A candidate *inside* a named path is
  still allowed, which is what keeps a subdirectory sweep possible inside a report-only
  directory.
- A directory a live process has open. The refusal names the pid and what it holds. On 2026-10-12
  `~/.cargo-target` held 17.5 GB with `release/sjel-mcp` — that session's own MCP server — inside
  it, while `tools/doctor` was naming `sjel storage prune --target` as the fix for a stale
  toolchain. A probe that cannot run refuses too, because "could not ask" must not read as
  "nothing is using it". The process asking is subtracted from its own answer:
  `CARGO_TARGET_DIR` here is the shared target dir, so otherwise every run would refuse itself.

And one it will not do at all: run a reclaim command for a class the policy marked report-only,
or run a bare `rm -rf` when nothing was measured. A policy string is data, not code, and the one
case where it reaches a shell substitutes only the paths this tool measured.

`reclaim --json` is refused rather than answered with a report, because a caller that meant to
act would read a report as success. `report --json` reads the report; `reclaim --dry-run` shows
the plan.

- Nothing here writes to the overlay, and nothing here reads a secret.
