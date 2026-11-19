# Transit reliability

Proposed model. The [Transit tasks](../plans/transit.md) own implementation and acceptance.

## Inputs and storage

A private watch set selects stops, lines and time windows. The initial corridor is declared in
the overlay. Anticipated presence may later join Calendar, Trips, Places and routine through their
existing contracts. Timetables, positions, traffic and map data remain provider-owned with bounded
caches; persistent evidence consists of aggregates and the user's realised rides.

An observation carries stop, route, trip, mode, scheduled/observed times, realtime availability,
cancellation and source. Prefer GTFS identifiers. A scheduled zero-delay value without realtime
is not punctuality evidence. Exclude cancellations from delay histograms.

Histogram keys include line, stop, hour, day type and regime. Exchange batches carry idempotency
keys so an intermittently connected Mac can merge them exactly once. Poll logic stays independent
of filesystem and database access; Punctuality owns measured reliability.

## Evaluation

Fallback from line-at-stop to line, mode-at-region and a mode/right-of-way prior. Recency weighting
and explicit regime changes avoid pooling incompatible conditions. Fresh source remarks,
upstream vehicle delay and road-segment observations override history while valid.

Transit evaluates arrival before a calendar deadline, including transfer headways and correlated
shared shocks. The traveller profile supplies risk tolerance. Cheap fallback routes matter as
well as the arrival quantile. This is a proposed model, not evidence that its forecast is calibrated.

## Acceptance boundary

Record realised rides and compare calibration and Brier score against the provider's prognosis.
Keep personal route preferences separate from measured reliability. Collection and routine stay
on owner-controlled machines; a closed Mac must not imply unobserved data was collected.
