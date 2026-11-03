# traveler

Personal travel profile containing hard constraints, destination and journey ranking weights, interests, pace, anchors and provenance for each field.

The technical manual for Travel capabilities is [`website/content/capabilities/travel.md`](../../website/content/capabilities/travel.md).

## Why this shape: preferences remain explicit

Hard constraints are kept separate from ranking weights: a limit must not become a trade-off. Destination weights and journey weights are separate because they rank different decisions. Every field records whether it is a built-in default, stated, derived or proposed, so an unreviewed value is distinguishable from a personal choice.

The capability is independent of Trips. It derives available history when possible, but missing or unavailable trip data does not prevent a search from using its own defaults.

## Run

The service is managed through `tools/service-runner.sh` and its `service.toml` manifest. The live HTTP contract is available at `GET /routes`.

## Considered and declined

- **No profile table inside Trips.** Other capabilities also consume traveller preferences; an independent contract avoids making them depend on Trips.
- **No implicit default provenance.** Every profile field has an explicit basis, so an omitted or misspelled basis cannot be mistaken for a default.
