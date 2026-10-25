# Can the phone read Sjel's data without this Mac?

First written 2026-10-21, after the primary was built and the split question was opened.

The owner's goal, in their words: *"I want all offline available or as much as possible, but not
entirely sure yet."* This entry exists so that "as much as possible" is a measured choice rather
than an ambition, and so nobody re-derives the numbers a third time.

## What offline actually requires

A device that reads Sjel's data with this Mac asleep needs a **local copy on the device**. There are
exactly two ways to get one:

1. **A file copied across** — the phone holds a database snapshot and reads it. Simple, one-way, and
   stale by however long since the last copy.
2. **A replication protocol** — libSQL's embedded replica: reads local, writes forwarded to a
   primary, a background pull brings changes back. Bidirectional, near-live, and it requires the
   primary to own its database file exclusively.

Everything below is a consequence of that second constraint, because Sjel's writer today is *not* a
server: 20 capability processes open one SQLite file directly.

## What was measured on 2026-10-21

| fact | measurement |
| --- | --- |
| the shared database | `axon.db`, 275 MB, 108 tables, opened by 20 capability modules |
| engines in play | 18 capabilities on `sjel-store` (`rusqlite`), 2 on `sjel-turso` (`libsql`) |
| cross-prefix SQL in the whole codebase | **1** statement: `INSERT INTO inventory_item SELECT … FROM interior_item` |
| cross-prefix foreign keys, triggers, views | **0**, **0**, **0** |
| `projects` (the phone's first need) | **1** table (`projects_records`), 1,221 rows, **0** joins, already on libsql |
| can `sqld` serve the existing file? | **yes**: a `sqlite3 .backup` copy of the live `axon.db` at `<db-path>/dbs/default/data` answered `sqlite_master` and capability queries over Hrana, no conversion, no ETL |
| readers other than the owning capability | `sjel-status` (per-domain statements, no joins to rewrite) and `apps/gpui`, which resolves the path itself at `apps/gpui/src/data/mod.rs:90` — a second copy of the rule that exists today |
| write contention | no `database is locked` evidence found in the logs reachable on this machine, so contention is an architectural expectation here, not a measured problem |

## The routes, and what each costs

| | what the phone holds | freshness | phone writes | files on the Mac | cost |
| --- | --- | --- | --- | --- | --- |
| **R1 snapshot mirror** | a copy of a chosen table set, refreshed on a schedule or on demand | stale by the interval | no | 1 (unchanged) | an export job + a fetch + storage in the app; no replication protocol, no split, no engine migration |
| **R2 split one capability** | an embedded replica of `projects` | near-live (pull interval) | yes, for that domain | 2 | one table moves; a resolver for "which file", one more backup contract |
| **R3 split into domain groups** | replicas of the domains it needs | near-live | yes, per domain | ~5 | the R2 cost five times, plus keeping real joins inside a group (`calendar × scouting × places × transit × trips` is the only group that joins) |
| **R4 whole file becomes the primary** | an embedded replica of everything | near-live | yes, anywhere | 1 | migrate **18 capabilities** off `rusqlite` to `libsql` remote or replica; reads become loopback round trips. The data move is a copy, so the cost is the engine migration |
| **R5 today's phone** | nothing; drives this Mac over the tailnet | live, while the Mac is awake | yes | 1 | nothing — and it fails exactly when the Mac is asleep |

R4 is the only route that gives everything, and it is the largest item in the plan. R1 is the only
route that gives anything today without touching a single capability.

## R1 built, 2026-10-21: `sjel mirror`

The snapshot mirror exists on the Mac side: `sjel mirror build` writes a consistent, digest-verified
copy of a chosen table set to `<overlay>/data/mirror/mirror.db`, and `sjel-status` serves it at
`GET /api/sjel-status/mirror` (with range support) beside `GET /api/sjel-status/mirror/manifest`.

Measured on this machine the same day:

| | |
| --- | --- |
| default phone set | 7 tables: `projects_records` (1,221), `calendar_entries` (51), `calendar_rhythms` (0), `entities_entities` (294), `entities_facts` (34), `places_places` (252), `places_person_places` (19) |
| size | **11.1 MB**, 1,871 rows |
| build time | under a second against a 275 MB source |
| consistency | every table copied inside one read transaction; `PRAGMA integrity_check` → `ok` |
| fetch | 11,141,120 bytes over the authenticated route, sha256 identical to the manifest's, `integrity_check` `ok` on the fetched file, 1,221 projects readable |

The safety properties are the ones that mattered: the copy is built beside its destination and
renamed into place, so a fetch during a rebuild gets the previous copy whole or the new one whole;
a named table that does not exist is an error rather than a silent omission; and a failed copy is
removed rather than left half-written.

Two facts the build taught that the plan did not anticipate:

- **A subset breaks foreign keys, and that is not a defect.** `calendar_entries` references
  `calendar_rhythms`; taking one without the other is what a phone set *is*. The copy therefore runs
  with constraint enforcement off and reports what it broke: the manifest carries `integrity` and
  `foreign_key_violations`, and the phone set currently reports **0**, because it takes both.
- **The size is one column, not one table of blobs.** Measured on the copy the same day:
  `projects_records.content` accounts for **9,738,041 of the 11,141,120 bytes** (87%), because the
  phone set carries 1,119 notes, 59 tasks and 43 projects in one table. `places_places` has no blob
  column at all (11 scalars: id, name, kind, address, city, country, latitude, longitude, source,
  external_ref, created_at). So the budget lever is note bodies, not places, and the number to watch
  is the vault's growth rather than any single media column. (The first version of this entry said
  the opposite; it was inferred from rows-to-bytes and was wrong.)

**The set grew once, 2026-10-23.** The phone's People tab needed two tables the set did not carry:
`entities_values` (612 rows — phones, emails, birthday, last_contact, company, relation) and
`places_people` (3 rows — the operator's decision for a person's name, which is what
`places_person_places` joins on). Without them the copy held 294 names and no contact card. Measured
after the change: **9 tables, 2,493 rows, 11,276,288 bytes** — the contact values cost 135 KB
against a 375 MB source, so the phone set's size is still note bodies, not people.

## R1 on the device, 2026-10-21: `src-tauri/src/mirror.rs`

The phone side is built, without UI. `mirror_sync` fetches the manifest, compares it with what the
device holds, fetches the copy if it differs, and `mirror_status` reports when it was built, what it
holds and whether the file still matches its digest. The fetch, the checks and the reader are one
module, tested against a fake transport and a real SQLite file; `dashboard/src/lib/mirror.ts` is the
typed seam a page will use. One sync also runs shortly after app start (`spawn_mirror_sync` in
`src-tauri/src/lib.rs`), so the copy exists on a device even before a page asks for it.

**The projects page reads it (2026-10-21).** `/projects` falls back to the copy when the Mac does not
answer (`lib/projects/offline.ts`, and the rules it keeps are tested in
`vite/offline-copy.test.ts`): the sidebar lists the projects the copy holds, the index lists every
note, and a selected project renders its records — title, task state, and the markdown body — from
the device. The page says so and says how old the copy is, and every edit is refused while it is
reading the copy: the copy is opened read-only, and a write made here could not reach the Mac
anyway. The rich tables (`DatabaseTable`, which needs the vault's frontmatter map) stay Mac-only;
the copy has no `fields`, `data_class` or `uri`, and the page shows what it has rather than filling
those in.

What the device refuses, and why each refusal is the point:

- **a copy whose bytes or sha256 disagree with the manifest**, before anything is written;
- **a copy that fails `PRAGMA integrity_check`** once written to its final directory;
- **a copy whose per-table row counts are not what the manifest claims** — a partial database that
  opens is otherwise indistinguishable from a complete one;
- **a stored copy that no longer matches its own manifest** on any later read, so a corrupted file is
  never served as data.

The write order is the second half of it: `mirror.db.part`, checked in place, renamed over
`mirror.db`, and the manifest written only after the file it describes. A reader gets the previous
copy whole or the new one whole.

**Two decisions taken with the owner on 2026-10-21, and what each costs:**

1. *The mirror becomes the offline store for project-shaped data.* The phone already keeps a
   URL-keyed snapshot of what it has fetched (`src-tauri/src/sync.rs`): interior, inventory trips and
   media answers, `/projects/api/tasks`, `/vault/api/people`, `/calendar/api/entries`. That reopens a
   visited page and is empty for a page or filter never opened. The copy answers both. The bounded
   version chosen is that notes and project detail come from the copy, the snapshot cache keeps
   interior/trips/media, and the task list can migrate later. The cost is real and recorded rather
   than removed: for as long as both exist, one page may read the same rows from two stores with
   different fetch times, and the copy's own freshness is what says which is older.
2. *The app asks the Mac to rebuild.* Nothing refreshed the copy after `sjel mirror build` ran by
   hand, so a device could hold a two-week-old copy that looked current.
   `POST /api/sjel-status/mirror/rebuild` builds on demand and answers the fresh manifest, and the
   app asks for it when its copy is older than twelve hours. No timer to supervise, and nothing
   happens when the app is never opened. The cost: a copy that nobody asks for stays stale, so the
   status line has to show the age rather than leaving it implied.

## R1 on the native phone, built 2026-10-23

Route R1 reached the native GPUI client, and it is the first page on the phone that reads data the
replica does not hold. `apps/gpui/core/src/mirror.rs` is the fetch: read the manifest, compare its
digest with the copy already on disk, take the copy only when they differ, check the bytes against
the manifest's `sha256` and `bytes` before writing, stage as `mirror.db.part`, rename into place,
then write the manifest. `apps/gpui/ui/src/calendar.rs` draws a day from it at phone density, and
`apps/gpui/ios/src/lib.rs` opens the copy on disk at launch and fetches a newer one when the pairing
answers or **Refresh** is tapped.

What was measured on this machine the same day:

| | |
| --- | --- |
| libsql reading a copy rusqlite wrote | works: 51 `calendar_entries` rows read from the 11.1 MB copy, and one entry returned for 2026-10-21 |
| the copy's age at the time of the run | built `2026-10-21T21:21:30Z`, two days old, which the tab prints rather than hides |
| the fetch | manifest first, so a current copy costs one small answer; a differing one costs 11.1 MB in a single signed request |
| the page on the simulator | a 6-entry trip day (5 to 11 October, all dotted), a single all-day entry, and the no-copy state, each screenshotted |
| tests | 8 in `mirror.rs` against a fake Mac and a real SQLite file: current copy not refetched, digest mismatch refused, byte mismatch refused, unbuilt Mac named, table set without the calendar answers empty, unmatched manifest left no partial file |

What the native reader does **not** do, unlike the Tauri device module: it does not resume a
partial transfer (one request, no `Range`), does not re-run `PRAGMA integrity_check` on the written
file, and does not compare per-table row counts. The manifest digest and byte count cover
transport corruption, which is the failure that was actually seen; a copy that will not open says so
on the page instead of drawing nothing. If a resumed fetch ever matters over a tailnet slower than
the LAN, `DeviceClient::signed` needs a header-carrying variant first.

Asking for a rebuild is tolerated rather than required: the route runs `sjel mirror build` on the
operator surface, so a paired device that is refused it still receives the copy the Mac holds, and
the tab's own age line is what tells the person how old that is.

## Decided 2026-10-21

**One database for now.** Nothing is split. The primary exists (`sqld` on `127.0.0.1:8089`, JWT
auth, daily backup) and holds no data, because no writer may follow it until the file question is
settled. `SJEL_TURSO_URL` stays unset, so `projects` keeps writing `axon.db`.

The split is not refused — it is deferred, and the reason is recorded in the storage plan history (`docs/plans/storage.md`) with these
same numbers. Two things argued against doing it now: the owner is not yet sure how much of their
data belongs on a phone at all, and every split is a one-way move of data plus a permanent extra
place to look.

## Replica removed, 2026-10-22

`libs/sjel-turso` no longer has the `replica` feature. Nothing used it: `SJEL_TURSO_URL` was unset
and the primary held no data. The phone was never a replica either. It links libsql `core` only,
reads the R1 copy, and replays its own write queue (`apps/gpui/core/src/sync.rs`). The feature cost
10 known advisories in `Cargo.lock` with no fixed libsql release (`libs/sjel-turso/README.md`,
"Embedded replicas: removed 2026-10-22"). R2 now has one more precondition: libsql on rustls 0.23,
or a fork.

## Decided 2026-10-22: the phone stands on its own

Taken with the owner on 2026-10-22. Each line is a rule a build can be tested against.

1. **The phone's data is the real record, not a cache of the Mac's.** What is typed on the phone
   lives in its own store first. A Mac, when there is one, receives it through the write queue
   (`apps/gpui/core/src/sync.rs`); it is never required for the data to exist.
2. **The app works without ever being paired.** A first launch with no Mac on the network opens
   to an empty, usable local store, not to an error or a join screen that blocks.
3. **Pairing is offered, never forced.** Setup may ask once. A settings page is always reachable
   and pairs in a few taps, after which the setup that follows runs on its own: discovery,
   allow in the dashboard, first sync.
4. **The scope without a Mac is local data only:** notes, tasks, projects and capture. Anything
   that needs a Mac service (mail, finance, the knowledge vault, local inference) shows a plain
   "needs your Mac" state rather than an empty or broken page.

Falsifier for 1 and 2: a simulator that has never seen a Mac, with the app freshly installed,
cannot create a note and find it again after a relaunch.

## What would settle each step

Not a roadmap — the trigger that makes the next route the obvious one.

- **R1 becomes worth building** when the phone must show *anything* while the Mac is unreachable.
  Its scope question is the real one: which tables does the phone need while out — projects, tasks,
  notes, calendar? Everything else can wait for the Mac.
- **R2 becomes worth building** when the phone must *write* offline (a task ticked on a train) rather
  than only read. `projects` is the cheapest possible subject: one table, no joins.
- **R3 becomes worth building** when R2 has run long enough to show that one replicated file is not
  a maintenance burden — i.e. the resolver and the extra backup contract are proven boring.
- **R4 becomes worth building** only if the answer to "how much belongs on the phone" turns out to
  be "nearly all of it", because it is the same work for all 18 capabilities at once. If that day
  comes, do not split first: R4 leaves one database, which is the shape that avoids the whole
  "which file" problem.

## What this entry does not show

- No measurement of iPhone storage or read latency on the copy. Its size on the Mac is measured
  (11.1 MB, 87% note bodies); what the same file costs once SQLite opens it inside the app, and how
  long a query takes on the phone, is not.
- No measurement of libSQL replica behaviour across a primary restart, or of what a phone does when
  its primary is unreachable for days (write queueing, conflict behaviour). R2/R3/R4 all depend on
  it and none of it is tested here.
- No security review of putting Sjel's data on a phone: at-rest encryption, what happens when the
  device is lost, and whether a phone replica should hold `comms` at all are all open. The tailnet
  and sqld's JWT check cover the *transport*, which is the smaller half.
