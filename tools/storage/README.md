# tools/storage — what is filling this disk, and what is safe to reclaim

`sysmon report` answers "am I full" with one `df` line. This answers "what is filling me",
which `df` structurally cannot. The 46 GB that started this tool was 215 individually
unremarkable 230 MB cache blocks. No per-file view showed it.

The crate is `sjel-storage`, a member of the root Cargo workspace. Operator machinery lives
in `tools/` and its backend logic is Rust, so `tools/` holds a Cargo member
(`Packs/harness/skills/sjel/references/on-placement.md`,
`Packs/harness/skills/sjel/references/on-dependencies-and-build.md`). Run it as
`axon storage <verb>`; `tools/storage/storage` is the launcher underneath, and
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

## The four verbs

| Verb | Reads | Answers |
|---|---|---|
| `report [--json]` | the overlay policy | free/used/total, every class with its size, what is over the flag, what is protected, what is expected to be running |
| `apply [--json]` | the overlay policy | runs each applicable class's reclaim command |
| `target [--json]` | nothing | the Cargo target dir per profile and per bucket, the R6 ratio, and whether the artifacts match the installed rustc |
| `prune [--incremental] [--target] [--node-modules] [--dry-run]` | nothing | repo-scoped reclaim with no policy at all |

Exit 0 means nothing is over a threshold. Exit 1 means something is: free space below
`free_critical_gb`, an R6 ratio above 3×, or a `prune` path that had to be refused. Exit 2
means it could not measure — a usage error, or no overlay policy to read.

`report` and `apply` reproduce the contract `tools/storage.ts` had, including the `--json`
field names, so `tools/host-watch` and any overlay hook read the same shape they always
did. `--json` gained one key, `expected_service`: the text report always printed it and a
JSON reader had no way to see it.

## The policy contract it inherits

Every path, threshold and reclaim command is a deployment fact and lives in the overlay at
`<overlay>/config/storage-policy.toml`. Nothing in this crate names a path on this machine
(CONTRIBUTING.md#public-core-and-private-overlays). `schemas/storage-policy.toml.example` is the
shape, and a unit test deserialises it so the template cannot rot away from the parser.

Two rules keep `apply` safe, both carried over unchanged and both tested:

1. A class is touchable only when the policy both allows it and says how. `apply = true`
   with no `reclaim` is a policy bug, not a licence to guess.
2. A policy-supplied string is data, not code. The literal `reclaim = "rm -rf"` deletes the
   paths this tool measured, never a path the policy named. Anything else runs verbatim,
   because it is a named tool's own cleanup verb — `brew cleanup`, `cargo clean` — that
   only its own CLI can express.

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

- `prune` will not touch anything that resolves outside the repo root or the Cargo target
  dir. The check is made after `canonicalize`, so a symlink out of the tree fails it.
- `prune` will not touch anything tracked by git. The `--node-modules` list is derived from
  `git status --ignored --short -z`, never from a hardcoded path, and each candidate is
  re-checked against `git ls-files` before removal.
- `prune` will not remove the repo root or the target dir themselves. `--target` runs
  `cargo clean`, which is cargo's own verb for a directory whose layout cargo owns.
- `apply` will not run a reclaim command for a class the policy marked report-only, and
  will not run a bare `rm -rf` when nothing was measured.
- Nothing here writes to the overlay, and nothing here reads a secret.
