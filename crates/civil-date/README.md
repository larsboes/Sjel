# civil-date

Howard Hinnant's civil-from-days pair, written once. Public domain, from
[chrono-Compatible Low-Level Date Algorithms](https://howardhinnant.github.io/date_algorithms.html).

Four capabilities had copied it — `places`, `finance`, `calendar` and `trips` —
with a fifth copy inlined inside `trips`' intent parser. Three of the five were
the same twelve lines to the character.

Every one of them explained itself the same way: no date dependency for a
conversion that is a dozen lines of integer arithmetic. So this crate has none
either. A shared home that arrived with chrono and chrono-tz would have undone
the property those five authors were buying, which is also why this is not a
module in `libs/station-time` — that crate carries both.

## Two scales, and the bridge

Hinnant's algorithm counts days from a **proleptic year 0**, not from Unix.
`trips`' window arithmetic calls that a *day number*; everything else counts from
1970-01-01. `UNIX_EPOCH_DAY` is the only bridge between them, and every name here
says which scale it is on:

| Unix scale | year-0 scale |
|---|---|
| `ymd_to_unix_day`, `unix_day_to_ymd` | — |
| `iso_of_unix_day`, `unix_day_of_iso` | `iso_of_day_number`, `day_number_of_iso` |
| `today_unix_day` | — |

`today()` returns `YYYY-MM-DD`, UTC. The scale matters: `trips` recorded that a
raw Unix day count fed to the year-0 inverse returns a well-formed date nearly
two thousand years wrong, so nothing downstream can tell it went wrong.

## How much validation

`unix_day_of_iso` checks *shape*, not the calendar: the month must be 1..=12 and
the day 1..=31, so `2026-13-01` is refused and `2026-02-30` converts to March 2nd.
`finance::clock::iso_day` gets the stricter answer by running its own
`valid_iso_date` first, and `calendar::date::parse_date` by round-tripping through
`unix_day_to_ymd`. Both of those gates stayed where they were.

Consumers: `calendar`, `finance`, `places`, `trips`.
