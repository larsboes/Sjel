# health

The operator's daily health values from Apple Health: sleep, activity, heart and body. One value
per metric per day, in metric units. Data class `c1`: the operator allows cloud models to read it
(task `task-bd7107c0-a1a4-4bbf-8206-19acd4ffc6a6`).

The default build uses the shared database. A `turso-wal` build opens the staged file at
`data/health/health.db` in the overlay. `SJEL_HEALTH_DB_PATH` overrides that location. In normal
operation, CLI imports and reads go through the owner socket, and `health-wal-backup` takes its
backups from an owner snapshot. See the [cutover runbook](../../docs/tasks/task-8f68cb3e-631e-4646-a474-1b8a68ae447c.md).

## Surface

```bash
health import <path>   # the Health app's export .zip, its unpacked folder, or export.xml
health serve           # HTTP API (service.toml)
health metrics         # each stored metric with its coverage, as JSON
health summary [days]  # mean, min and max per metric over the last days (default 30)
health snapshot        # turso-wal builds: ask the running owner for a backup snapshot
```

On the iPhone: Health → profile picture → Export All Health Data. Send the zip to the Mac and run
`health import` on it. The export is complete, so an import replaces, for each metric it carries,
every stored day in its range. A phone upload (`POST /api/days`) replaces only the days it sends. A
zip streams through the system `unzip`. The export is never unpacked to disk or held whole in
memory. A synthetic export of 954,000 records (164 MB) folds in about 3 s with 27 MB peak memory
in a debug build.

| Route | Answers |
|---|---|
| `GET /api/metrics` | Each stored metric: group, unit, number of days, first and last day, newest value |
| `GET /api/days` | Daily values. `?metric=`, `?from=`, `?to=` (default: the last 30 days, at most 3,660) |
| `GET /api/summary` | Mean, min and max per metric over `?days=` (default 30) |
| `GET /api/imports` | The 50 newest import runs |
| `POST /api/days` | Daily values and raw sleep samples from one device (the iOS HealthKit sync). Sleep samples fold into nights by the export's rule. A bad value refuses the whole upload |

Agents read these routes through the agent gate under the owner's policy for `health`.

## Metrics

The catalogue is `METRICS` and `SLEEP` in `src/export.rs`. A record type it does not list is
counted and skipped.

| Group | Keys | Rule per day |
|---|---|---|
| Activity | `steps`, `distance_walking_km`, `flights_climbed`, `active_energy_kcal`, `exercise_min` | Total per source. The source with the largest total wins |
| Heart | `resting_hr`, `walking_hr`, `hrv_sdnn_ms`, `heart_rate_avg`, `heart_rate_min`, `heart_rate_max` | Mean, or min and max, over all samples |
| Body | `body_mass_kg`, `body_fat_pct`, `lean_mass_kg`, `bmi` | The newest sample |
| Sleep | `sleep_asleep_h`, `sleep_in_bed_h`, `sleep_awake_h`, `sleep_core_h`, `sleep_deep_h`, `sleep_rem_h`, `sleep_onset_h`, `sleep_wake_h` | Hours, for the night whose midpoint falls between 20:00 and 12:00, from the one source with the most sleep. Naps are left out |

## Why this shape

1. **The largest source wins for totals.** A phone and a watch both count the same walk. Adding
   them doubles it, and the Health app's own source priority is not in the export. The largest
   total per day is the closest rule the export allows.
2. **Days are local.** A timestamp in the export carries its offset, so each sample lands on the
   date the operator lived it, wherever that was.
3. **Daily values, not samples.** The questions this answers are about days and trends. Raw
   samples would grow the shared database by millions of rows with no reader.
4. **The phone sends samples, the Mac folds them.** A Mac has no HealthKit store, so live data
   comes from the Sjel iOS app (`apps/gpui/ios/host/Health.swift`). It sends HealthKit's daily
   statistics, which merge sources by the user's own order, and raw sleep samples. The night
   rule stays here, so a synced night and an exported night are the same night.
