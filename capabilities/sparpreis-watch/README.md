# sparpreis-watch

The Sparpreis price watch (travel PRD R4). Every 12 hours it runs `tools/sparpreis-watch.ts`. The
script searches the rail fares again for each upcoming trip plan, appends the result to the plan,
and exits.

What it watches, per upcoming plan:

- every train stage that is not booked or completed, searched by the stage's place names on its
  date
- rail `option_set` items, but only while they still match an unbooked train stage

Each watch keeps one `option_set` item, `sparpreis-watch:<key>`, that holds the full price history.
When the cheapest fare today is below every earlier observation, the script writes a `note` item to
the plan. That note is the alert.

The script calls `trips` and `transit` over HTTP and never opens their databases. Both must be
enabled on a machine that schedules the watch, which `requires` enforces. Each run sends real
requests to bahn.de through transit's self-paced client, so the interval stays at 12 hours. The
reasons for that cadence are at the top of `tools/sparpreis-watch.ts`.

Run it by hand:

```
bun run tools/sparpreis-watch.ts
```
