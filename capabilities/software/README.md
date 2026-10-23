# software

The apps, leftovers and command-line tools on this Mac: what each holds on disk, when it was last
opened, and what the operator decided about it. Use, remove, replace with something better, or
rebuild the useful part in Sjel.

It began as a storage question on 2026-10-22. Cleaning the disk by hand meant finding each app,
adding up its bundle and its `~/Library` data, asking whether it was still used, and remembering
the answer. A review that has to be redone from memory every time is the thing this keeps.

```
software serve   HTTP API on port 8106; scans on start and every six hours
software scan    scan apps, ~/Library leftovers and package-manager tooling now
software list    what is present: kind, size with data, last used, decision, review
```

The native page is `sjel://software` in the GPUI app, with tabs for apps, leftovers, tools and
the rows due review.

## What a scan reads

Three kinds of row, one table. The key is a bundle id for apps and leftovers and
`<manager>:<name>` for tools (`brew:jq`, `npm:@playwright/cli`).

**Apps.** Every `.app` in `/Applications` and `~/Applications`, and one folder deeper.

| Field | From |
|---|---|
| bundle id, name, version | `Contents/Info.plist` through `plutil -convert json` |
| source | `cask:<token>` when a cask's `INSTALL_RECEIPT.json` lists the bundle, `app-store` when `Contents/_MASReceipt/receipt` exists, else `other` |
| app size | `du -sk` on the bundle |
| data size and paths | the `~/Library` entries keyed by bundle id (`Application Support`, `Containers`, `Caches`, `HTTPStorages`, `WebKit`, `Saved Application State`, `Preferences`, `Group Containers`), plus `Application Support/<name>` and `Logs/<name>` |
| last used | Spotlight's `kMDItemLastUsedDate` through `mdls` |

The last-used date is advice. An app started from a shell never updates it: on the first scan
Xcode had no date at all, on a day when the iOS builds ran its toolchain for hours.

**Leftovers** (`src/leftovers.rs`). A `~/Library` folder named like a bundle id that no installed
app owns, holding at least 1 MB in total. The first scan found 18, among them ChatGPT, Perplexity
and Civilization VI from the uninstalled Steam. It is a candidate list: a cache a tool still uses
(`org.swift.swiftpm`) qualifies too, which is why nothing here acts without a decision.

**Tools** (`src/tooling.rs`). Each package manager's own record: `brew leaves
--installed-on-request`, the Caskroom, `~/.cargo/.crates2.json`, npm's global root and uv's tool
folder. Not `tools/updates --json`: that is a staleness report, and offline it reduces brew and uv
to one summary row each. A cask whose app is gone is still listed; `macspace` was one, and only
`brew uninstall --cask` clears it.

Sizes are allocated blocks, the number `sjel-storage` uses for its policy paths
(`tools/storage/src/measure.rs`, `size_of`), so the two tools agree on what something costs.

## Review

A present row is due another look when (`review_reason` in `src/store.rs`):

| Rule | Why |
|---|---|
| decided `remove`, still present | the uninstall has not happened |
| kept with `use`, not opened in 90 days, and the decision is 90 days old too | a recent `use` was made knowing the date, so it waits until it ages |
| any keep older than 180 days | replacements improve and rebuilds land; a standing decision expires |

A missing last-used date never triggers review: its absence is not evidence.

## Uninstall

Only a row whose current decision is `remove` (`src/uninstall.rs`). An app or leftover moves to
`~/.Trash`, so Finder's Put Back works until the Trash is emptied. A cask goes through
`brew uninstall --cask`, and brew, cargo, npm and uv packages through their own uninstall, so each
manager's record stays true. Refused: a running app, and any path that resolves outside
`/Applications`, `~/Applications` and `~/Library`.

This is its own path rather than `sjel-storage`'s guard. That guard sits in a binary-only crate
and protects `rm -rf` of policy classes; moving one reviewed item to the Trash is a different,
reversible operation.

## Tables

| Table | What |
|---|---|
| `software_apps` | one row per key, as the last scan saw it, with its `kind`. A row that disappears keeps `present = 0`; an app uninstalled with its data left behind returns as a leftover under the same key, history intact |
| `software_decisions` | append-only. The newest row per key is the current decision; older rows say why it changed |

Both live in the shared database through `sjel-turso`, as `operator-profile` does.

## HTTP

| Route | What |
|---|---|
| `GET /api/apps` | every row a scan has seen, largest footprint first, with its decision, note and review reason |
| `POST /api/apps/scan` | scan now; 409 while a scan or an uninstall runs |
| `POST /api/apps/{key}/decision` | `{ decision, note }`, where decision is `use`, `remove`, `replace` or `rebuild` |
| `POST /api/apps/{key}/uninstall` | run the uninstall for a `remove` row, then rescan; 409 for any other decision |

A key with `/` or `@` is percent-encoded in the path (`npm:%40playwright%2Fcli`). `GET /routes` is
the manifest the gate checks against the router.

## Not built

- Review in the doctor or the status page. It shows on the Software page and in `software list`.
- Vendor CLIs (Claude Code, Codex, opencode): they install and uninstall through their own
  installers, so a row here could record a decision but never act on it.
