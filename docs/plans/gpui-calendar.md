# Calendar migration to Ely

Calendar belongs to [the GPUI Ely plan](gpui.md#components-from-ely).

Implemented: the Mac calendar composes Ely's month/day/week views, agenda, event cards and
editor. The fork supplies committed moves and resizes, slot activation, free intervals,
controlled selection, commitment appearance, configurable editor fields and pending/error
states. Wall-clock geometry handles DST; ambiguous or nonexistent local targets are rejected.
The dependency manifest and lockfile own the published fork revision.

Sjel retains capability writes, stale-time refusal, undo, proposal review, sync status,
source details and Mail/Travel navigation. NSH retains the shared core timeline rules.
The conversion boundary preserves exclusive all-day ends and source metadata.

Verification: the fork passes 1,748 tests on GPUI Fast, 1,749 on GPUI Pre and library Clippy.
Application tests, feature checks and rendered light/dark acceptance remain pending while
verification moves to a clean snapshot of the calendar change. Concurrent finance/chart
edits prevent a reliable whole-checkout completion claim. No physical-phone check has run.
