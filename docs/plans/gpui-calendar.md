# Calendar migration to Ely

Calendar is a work package under [the GPUI Ely plan](gpui.md#components-from-ely).
Replace the Mac calendar renderers after the fork covers the existing interactions.
The source comparison is complete; fork changes and application migration remain open.

## Comparison boundary

Sjel sources: `apps/gpui/ui/src/calendar/{actions,grid,page,proposals,types}.rs`,
`apps/gpui/core/src/calendar.rs` and `apps/gpui/core/src/calendar/timeline.rs`.
The comparison uses the locked Ely revision `c9f6c1a60373d5c871ac80a398eb15130a19c8c2`.
Fork paths below are relative to
[`src/calendar/`](https://github.com/larsboes/Ely-GPUI-Components/tree/c9f6c1a60373d5c871ac80a398eb15130a19c8c2/src/calendar).
The dependency manifest and lockfile own the current pin.

Ely supplies Monday-first month/week/day views, overlap lanes, all-day spans, overnight
clipping, a current-time line, event activation and a validated event editor. `AgendaView`
can replace the day event list after the selection and detail requirements below are met.
Sjel exposes Day, Week and Month, with a day list beside the grid. Agenda and year are not
separate modes. No new modes are required for this migration.

## Fork gaps

### Move and resize

Sjel's `grid.rs` drops `CalendarDrag` into another day, previews the target and persists
once on drop. Core `timeline::reschedule` preserves duration for moves and snaps to quarters.
Ely's `TimeGrid` and `CalendarDays` expose create, resize and event activation, with no
event-move callback. Ely `grid.rs` invokes `on_resize` during `on_drag_move`, on the final
visible segment of an event. Passing that callback straight to Sjel's writer would send
intermediate writes and replace the single undo boundary.

Add event movement and distinguish preview from commit in the fork. A committed move
reports the key and proposed start/end; a committed resize reports the key and proposed end.
Both support a target on another visible day. Test cross-day moves, duration preservation,
quarter-hour snapping, invalid ends, cancellation without a commit, isolation between grids,
and many pointer moves producing exactly one commit.

### Slot creation

Sjel opens a one-hour draft from a quarter-hour click. A click on a free gap caps the draft
at the gap end. Ely's `TimeGrid::on_create` requires an empty-time drag of at least 15 minutes.
Add slot activation alongside range creation. Test click versus drag, event presses that
create nothing, late-night creation ending on the next day, and short-gap end caps.

### Selection and keyboard control

Ely `month.rs` keeps a private cursor and consumes arrows and Enter. `on_day` reports opening
a day, not every selection; `on_month` reports month changes. `days.rs` has no selected-day
or selected-event input and no day-heading callback. Its seeded first date follows changed
owner input, but does not represent Sjel's selected day within the week.

Expose controlled day/event selection and selection changes across month/day/week, with a
way to use Sjel's actions without competing key handlers. Keep Sjel's selection authoritative
for its detail panel, new-entry date and `open_day` navigation. Test pointer selection,
arrows/h/j/k/l, brackets, t/m/d/w, e, n, Backspace, Enter, Escape and u/Cmd-Z. Include focus in
form inputs, selected-event highlighting, and detail invalidation when the selection changes.

### Free-time overlay

Sjel `timeline::gaps` excludes `possible` and `travel_ok` entries from reservations and
draws clickable free intervals from 08:00 to 20:00. Ely's `AvailabilityPicker` is a separate
duration-based slot picker; it does not replace these intervals inside `TimeGrid`.

Let the grid display supplied free intervals and report interval activation. Keep reservation
rules in core. Test overlaps, short gaps, possible entries, all-day reservations and suppression
while calendar data is loading or failed. Preserve the existing availability toggle.

### Event presentation and editor integration

Ely's `Event` carries key/title/when/hue/place. Its cards and chips lack Sjel's selected
appearance and possible/tentative commitment cues. `EventEditor` supplies title, dates/times,
all-day switching, place, color, recurrence, reminders and notes, but lacks kind/commitment
controls, pending/error inputs and configurable fields. Its timed start edit preserves duration;
Sjel's form currently clamps the end date when the start date passes it.

Add generic event appearance/selection inputs and editor extension points for Sjel-owned
choices, pending state and errors. Allow unsupported recurrence/reminder/color controls to be
hidden. Retain Sjel's kind and commitment values independently of the display hue. Preserve
end-date clamping rather than silently adopting a different start-edit rule. Test choice
preservation, disabled repeat saves, failed-save draft retention, Enter save, Escape cancel,
focus return and all-day switching. Keep existing location/notes when a time-only edit saves.

## Time conversion and DST

Sjel stores local naive timestamp strings and exclusive all-day ends; Ely uses Jiff timestamps
and inclusive all-day last dates. Validate entries before Ely's asserting constructors,
preserve original metadata by key, and convert all-day ends in both directions. Resolve timed
values in an explicit zone and serialize them back to the capability's local-time contract.
Test malformed entries, midnight ends, multi-day entries and round trips without metadata loss.

Ely `hours.rs::minutes` computes grid positions from elapsed minutes since zoned midnight;
`moment` adds elapsed minutes to midnight. On a DST transition day, these differ from the
wall-clock hour shown by the 24-hour grid. This is a source-derived defect, not a reproduced
runtime result. Fix the fork's grid coordinate conversion and test Europe/Berlin's spring gap
and autumn fold, including selection, resize, clipping and round trips. The adapter must handle
ambiguous or nonexistent local times explicitly rather than silently changing an event's time.

## Sjel ownership

Keep these application behaviors outside the component fork:

- Proposal loading for the next 90 days, Add to Calendar, confirmed removal, busy/error states
  and refresh after review (`proposals.rs`). Add promotes the entry to event/planned.
- Capability writes, stale-time checks before rescheduling, single-step undo, loading/error
  states, sync status and source metadata (`page.rs`). Fork callbacks describe user intent.
- Source provenance, importance details, Mail/Travel navigation and quick-shift actions.
  Ely's fixed `EventPopover` offers edit/delete/attendees but no application-detail slot;
  retain Sjel's detail panel beside the Ely views.
- Shared calendar and timeline rules in core, including helpers still used by NSH. The phone
  remains on NSH. Recurrence, reminders, attendees and YearView stay outside this migration.

## Implementation and acceptance

1. Fix time geometry and add the missing fork interactions with focused headless tests.
   Test both supported engines, push the fork, then update Sjel's dependency pin and lockfile.
2. Add the validated conversion boundary. Replace month/day/week grids, event cards/chips
   and the day list; delete each superseded Mac renderer in the same change. Keep one selected
   day/event across the views and the detail panel. Add the availability overlay from core gaps.
3. Replace the form after editor integration supports the existing choices and write states.
   Keep deletion confirmation and undo at the application boundary.
4. Verify production-view input dispatch and capability request bodies, including proposal
   review and stale-time refusal. Keep generic component interaction tests in the fork and
   Sjel-specific conversion, navigation and write tests in the application.
5. Run the app workspace tests, no-default-features build, UI `nsh` build and fork engine checks
   from [the parent plan](gpui.md#components-from-ely). Complete Mac light/dark acceptance and
   phone regression checks before declaring the migration done.

Verification for this audit is source inspection only. Fork tests, native tests, feature builds,
DST execution checks and visual acceptance have not run. Documentation checks are reported in
the change's verification output.
