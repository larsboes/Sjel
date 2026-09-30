# knowledge-base

The vault, as a thing that gets backed up.

`kind = "data"` — no image, no command, nothing to start. Obsidian owns this directory and Sjel
writes to it only inside declared projection regions (`libs/markdown-root/src/projection.rs`).
The manifest exists for the one thing a file tree needs an owner for, which is backup.

## Why it exists

Because the vault had three declared backups and none of them worked.

Measured 2026-08-29, before this capability existed:

| Declared where | What it claimed | What was true |
|---|---|---|
| overlay `machine.toml`, `[[state_mount]]` | `sync = "git"` — *"git is history + backup (bare repo on homepi)"*, `direction = "capture"` | The vault checkout had **no remote and never had one** |
| overlay `systems.local.toml`, `[knowledge-base]` | `url = "https://github.com/larsboes/Knowledge-Base"` | The repository **does not exist** |
| the backup target host | a restic repo, 8.9 GB | **Two snapshots, both 2026-08-02**, and `restic` is not installed on this Mac |

Time Machine had no destination configured either, and no LaunchAgent referenced the vault path.
So the only off-machine copy was iCloud Drive live sync, which propagates a deletion rather than
surviving one.

**A `[[state_mount]]` says where data lives. Only a manifest puts it in the registry that reports
when a backup goes stale** — and that difference is the entire reason for this directory. Every
one of the three failures above was silent, and each would have stayed silent indefinitely,
because nothing was watching a path that no manifest owned.

## What it backs up

The whole vault directory including `.git`, so one archive carries current state *and* complete
history. That is also why `backup_retain` is 7 rather than store's 14: how far back you can go is
not answered by how many tarballs survive.

`.obsidian/plugin-backups/` is excluded. It held 19 symlinks pointing out of the vault at a
since-retired plugin monorepo, so every one dangled — and 4 KB of dead links made the first 704 MB
archive **unrestorable**, because `tools/restore.sh` refuses any archive carrying a link. That
refusal is correct: a symlink inside an archive is how extraction writes outside the directory the
operator chose. `tools/backup.sh` now refuses to *produce* one, so the two tools hold the same
contract instead of disagreeing at restore time.

## Where the path comes from

The member name is tracked here; the root it hangs from is machine-local, in the overlay:

```toml
[capability.knowledge-base]
backup_source_root = "~/Library/.../Documents"
```

An iCloud container path is a fact about one machine, and `backup_paths` must stay relative
because those strings are tar member names as well as sources.

## Rehearsed

**2026-09-23, and it closed D10.** The archive is 4,843,575,394 bytes, `blocks=9460112` (on this
machine, not an iCloud stub), `flags=-`, and its sha256 matches its receipt byte for byte. It was
restored into an isolated destination and compared against the live vault file by file:
**13,436 files and 2,848 markdown notes, zero content differences, zero files on one side only.**

Getting there required fixing two defects, and both are worth knowing because each made a
previous success claim false:

1. **The producer's link guard never worked.** `verify_archive` in `tools/backup.sh` read
   `if printf … | grep -q '^[lhbcps]'`. `-q` exits at the first match, which closes the pipe while
   the writer is still going, the writer dies on SIGPIPE, and `set -o pipefail` makes the pipeline
   report **141 — so a match evaluated as false**. Measured on this archive: exit 141 with
   pipefail, 0 without. The guard shipped the archive it exists to refuse, and its only trace was
   a `printf: write error: Broken pipe` that reads like noise. Every archive since 2026-08-29 was
   unchecked. It now captures the offending members first, without `-q` and without `head`, and
   `tools/backup-archive-guard.test.sh` drives the real extracted function under pipefail.
2. **The archive did contain a link.** `Projects/Bachelor-Thesis/Kolloquium/Assets/revision-build/node_modules`
   points into `~/.cache/codex-runtimes/` — build residue from a runtime that built inside the
   vault, and the only symlink in the vault outside `.obsidian/plugin-backups/`. One link is enough
   to make the whole archive unrestorable, so it is excluded by exact path. The exclusion is
   deliberately not a blanket `node_modules` pattern: a backup that silently drops content is worse
   than one that refuses.

A third, smaller defect surfaced while fixing the second, and it is the same class: `backup_exclude`
written as a **multi-line** array parses as *empty*, because `tools/lib/toml.sh`'s `toml_array`
greps a single line. The exclusion vanished, the archive carried all 20 symlinks, and the fixed
guard refused the run instead of shipping it — which is how it was found. The array is one line,
with the reason beside it.

**2026-08-29**, and it is the only reason any of the above was a claim rather than a hope:
704,596,192 bytes shipped, fetched back, restored into an isolated destination, and compared
against the live vault — **2,168 notes, 14,878,214 bytes, zero content differences**.

Eleven filenames differ in Unicode normalisation only (NFD on APFS, NFC through tar). Harmless on
macOS, which is normalisation-insensitive; worth knowing before restoring onto Linux, where those
eleven names would differ byte-wise and Obsidian's wikilinks to them would not resolve. The
2026-09-23 comparison normalises both sides to NFC and found no such difference, which is the
confirmation that the earlier eleven were cosmetic.

## `plugin/` — the Obsidian plugin for the directory this capability owns

`plugin/sjel-lens/` is an Obsidian plugin: a right-sidebar pane showing what Sjel knows about the
open note, read-only, with its own README. It is not built and not installed by anything here —
the three files are what Obsidian wants, and the operator copies them into
`<vault>/.obsidian/plugins/sjel-lens/` (`plugin/sjel-lens/README.md`, "Install it").

**Why here and not under `vault/`.** The placement question is which capability's removal makes
the plugin meaningless. Remove the `vault` capability and the People pane degrades to one line
saying so — which is a first-class behaviour, not a failure — while the trip badge still works.
Remove *this* capability and there is no vault, no Obsidian, and nowhere for a plugin to be
installed. The plugin's lifecycle is bound to the directory, so it belongs to the capability that
owns the directory. `capabilities/soundscape/ui/` is the other shape and a different one: that is
a bundle its own server serves, and this is an artifact copied into a file tree.

It reads three capabilities over HTTP — `sjel-status` for discovery, `vault` and `trips` for the
two adapters — which is the ordinary cross-capability contract (CONTRIBUTING.md#schemas-and-dependency-direction),
not a claim on any of them.

`tools/check-obsidian-plugins.sh` gates it, and gates any other Obsidian plugin added anywhere in
the repository: no browser-context HTTP (sjel-status sends no CORS header, so the renderer
withholds its reply; `requestUrl` sends no `Origin` and needs none), no writes, no node require
under `isDesktopOnly: false`, and no address that is not discovered.
