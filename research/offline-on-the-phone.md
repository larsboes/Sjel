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

## Decided 2026-10-21

**One database for now.** Nothing is split. The primary exists (`sqld` on `127.0.0.1:8089`, JWT
auth, daily backup) and holds no data, because no writer may follow it until the file question is
settled. `SJEL_TURSO_URL` stays unset, so `projects` keeps writing `axon.db`.

The split is not refused — it is deferred, and the reason is recorded in `PLAN.md` §6 with these
same numbers. Two things argued against doing it now: the owner is not yet sure how much of their
data belongs on a phone at all, and every split is a one-way move of data plus a permanent extra
place to look.

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

- No measurement of how large a phone-side copy would be for a realistic table set, nor of iPhone
  storage or read latency on it. R1's size is unknown until the table set is chosen.
- No measurement of libSQL replica behaviour across a primary restart, or of what a phone does when
  its primary is unreachable for days (write queueing, conflict behaviour). R2/R3/R4 all depend on
  it and none of it is tested here.
- No security review of putting Sjel's data on a phone: at-rest encryption, what happens when the
  device is lost, and whether a phone replica should hold `comms` at all are all open. The tailnet
  and sqld's JWT check cover the *transport*, which is the smaller half.
