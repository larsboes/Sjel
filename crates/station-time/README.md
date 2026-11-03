# station-time

Timezone-correct arithmetic for station-local times. bahn.de serves every stop's
times naive, each in that stop's own local zone — measured live 2026-08-12 on
Köln→London, where wall-clock subtraction ran an hour short. This crate turns
(naive local time, station id) into an unambiguous UTC instant via the station's
UIC country prefix, so durations and transfer buffers stay right when a leg
crosses a zone.

The prefix→zone table covers countries whose rail network sits in one IANA zone;
anything else returns `None` rather than a guess. Source and the live spot-checks
are in the crate doc. Consumers: `transit` (leg `departure_utc`/`arrival_utc`);
the flight adapter (PRD F2) is the intended second consumer, with airport
timezones as its own lookup.
