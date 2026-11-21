# Native UI/UX Overhaul and Ely Component Consolidation

This plan defines the four-track UI/UX overhaul and component consolidation across Sjel's native desktop and mobile surfaces (`apps/gpui/ui/`) and Sjel's component library fork (`ely-gpui-component` in `apps/gpui/Cargo.toml`). It builds on the `ui-craftsmanship` rules, the [native UI contract](../architecture/native-ui-boundaries.md), and the [Projects, Tasks, and Notes UI/UX overhaul](gpui-projects-ux.md).

## Track 1: Shared Primitives & Chrome Extraction into Ely

Consolidate [`apps/gpui/ui/src/primitives/`](../../apps/gpui/ui/src/primitives/mod.rs) and [`apps/gpui/ui/src/nav.rs`](../../apps/gpui/ui/src/nav.rs) onto `ely-gpui-component` so Sjel pages share one themed component vocabulary:

1. **Keyboard shortcut hints and footer bars (`ely::typography` / `ely::shell`):** Extract `ShortcutHint` and `KeyBar` into Ely and route [`shortcut_hint`](../../apps/gpui/ui/src/primitives/chrome.rs) and [`key_bar`](../../apps/gpui/ui/src/primitives/chrome.rs) through them.
2. **Tabular figures and progress meters (`ely::typography` / `ely::data_display`):** Route [`mono_num`](../../apps/gpui/ui/src/primitives/badges.rs) and [`tabular_cell`](../../apps/gpui/ui/src/primitives/chrome.rs) through Ely's `tabular` typography helper, and [`progress_bar`](../../apps/gpui/ui/src/primitives/cards.rs) through Ely's `data_display` measures.
3. **Interactive metric tiles and quick actions (`ely::dashboard` / `ely::data_display`):** Extract `MetricTile` and `QuickActionTile` into `ely::dashboard`, routing [`stat_card`](../../apps/gpui/ui/src/primitives/cards.rs), [`dashboard_metric_tile`](../../apps/gpui/ui/src/primitives/cards.rs), and [`quick_action_button`](../../apps/gpui/ui/src/primitives/cards.rs) through Ely.
4. **Mobile bottom tab bar and navigation header (`ely::navigation`):** Extract `BottomTabBar` (`MobileTab`) and `NavHeader` into `ely::navigation` and route [`mobile_tab_bar`](../../apps/gpui/ui/src/nav.rs) and [`nav_header`](../../apps/gpui/ui/src/nav.rs) through them.

## Track 2: Mail, Feed & Inbox Triage Overhaul + Ely Mail Adapter

Unify the three high-frequency triage surfaces ([`mail/`](../../apps/gpui/ui/src/mail/mod.rs), [`feed/`](../../apps/gpui/ui/src/feed/reading.rs), and [`inbox/`](../../apps/gpui/ui/src/inbox/triage.rs)) onto Ely primitives and prove read-only Ely Mail parity ([`gpui-mail-adapter`](../tasks/gpui-mail-adapter.md)):

1. **Read-only Ely Mail adapter and triage row (`ely::mail` + [`mail/`](../../apps/gpui/ui/src/mail/mod.rs)):** Extend `ely::mail::Mail` / `MailItem` / `MailList` with proposal metadata badges (stream, authenticity, waiting, Gmail sync state, model verdict) and a Sjel adapter module that preserves proposal IDs, selection, opened thread state, held writes, sync failure notices, and caller message order.
2. **Mail filter and bulk-action chrome:** Replace manual status/category button loops in [`mail/mod.rs`](../../apps/gpui/ui/src/mail/mod.rs) with `sub_tabs` / `filter_pill`, use `key_bar` for the shortcut strip, and use Ely `Badge` / `InlineMessage` for notices and row metadata.
3. **Feed and Inbox triage row consolidation ([`feed/reading.rs`](../../apps/gpui/ui/src/feed/reading.rs), [`inbox/triage.rs`](../../apps/gpui/ui/src/inbox/triage.rs)):** Replace ad-hoc score pills, Keep/Dismiss div buttons, and keyboard hint strings with Ely `Badge`, `Button`, `InlineMessage`, and `key_bar`.

## Track 3: Block Editor & Projects Wave 2

Complete the remaining items in [`gpui-projects-ux.md`](gpui-projects-ux.md) and [`gpui-editor-adapter`](../tasks/gpui-editor-adapter.md):

1. **Block editor round-trip parity & Ely document chrome ([`editor/`](../../apps/gpui/ui/src/editor/mod.rs), [`gpui-editor-adapter`](../tasks/gpui-editor-adapter.md)):** Adopt Ely's `WordCount`, `ReadingTime`, `Badge`, and `TableOfContents` / outline pills in `NoteEditor`, and add the parity fixture suite comparing `ely::documents::BlockEditor` against `sjel_gpui_core::blocks` (frontmatter, nested lists, tables, unsupported syntax, byte-lossless refusal, split/merge, slash menu).
2. **Task detail panel & project readiness ([`projects/task_panel.rs`](../../apps/gpui/ui/src/projects/task_panel.rs), [`projects/render.rs`](../../apps/gpui/ui/src/projects/render.rs)):** Replace remaining `gpui_kit::component::button::Button` usages in `task_panel.rs` with `ely::buttons::Button`, `ely::data_display::DescriptionList`, and `ely::layout::Card` / `Section`, and surface inline Inventory link readiness in the Project Hub summary header.

## Track 4: Travel Overhaul & Mobile-Width Polish

Bring Travel ([`travel/overview.rs`](../../apps/gpui/ui/src/travel/overview.rs), [`travel/itinerary.rs`](../../apps/gpui/ui/src/travel/itinerary.rs)) onto Ely components and close the mobile density defects in [`gpui-phone-polish`](../tasks/gpui-phone-polish.md):

1. **Travel overview & itinerary component migration:** Replace raw status/readiness/retrospective pills, companion cards, and day-filter strips with Ely `Badge`, `Card`, `CardHeader`, `DescriptionList`, and `filter_pill` / `ToggleButton`.
2. **Mobile-width fit ([`gpui-phone-polish`](../tasks/gpui-phone-polish.md)):**
   - Ensure Calendar's week strip renders compact day labels without truncating to `…` at mobile width.
   - Wrap or compact the Projects hub toolbar at `Density::Mobile` so the search field keeps a usable width and `+ New Project` stays inside the viewport.
   - Hide desktop keyboard hints (`Esc to close · / for blocks`, `⌘S`, `⌘[`) in `NoteEditor` when `Density::Mobile` or on iOS.

## Track 5: Desktop Shell, Home/Today & Settings Consolidation

Consolidate the macOS shell (`apps/gpui/src/`) onto `ely-gpui-component` and extract a reusable settings row (`SettingRow`) into `ely::forms`:

1. **Shell header, sidebar, and Home/Today controls:** Replace `gpui_kit::component::{Button, Checkbox, Tag, Kbd}` in [`header.rs`](../../apps/gpui/src/header.rs), [`sidebar/nav.rs`](../../apps/gpui/src/sidebar/nav.rs), [`home/mod.rs`](../../apps/gpui/src/home/mod.rs), [`home/focus_tasks.rs`](../../apps/gpui/src/home/focus_tasks.rs), and [`home/needs_you.rs`](../../apps/gpui/src/home/needs_you.rs) with `ely::buttons::Button`, `ely::forms::Checkbox`, `ely::data_display::Badge`, and `ely::typography::Kbd`.
2. **Settings rail, switches, shortcuts, and `SettingRow` (`ely::forms`):** Extract `SettingRow` (title and optional description on the left, control on the right) into `ely::forms`, and migrate [`settings/mod.rs`](../../apps/gpui/src/settings/mod.rs), [`settings/appearance.rs`](../../apps/gpui/src/settings/appearance.rs), and [`settings/shortcuts.rs`](../../apps/gpui/src/settings/shortcuts.rs) onto `ely::navigation::{NavGroup, NavItem}`, `ely::forms::{SettingRow, Switch}`, and `ely::typography::Kbd`.

## Track 6: Remaining `gpui_kit` Form & Control Elimination in `sjel-gpui-ui`

Replace remaining legacy `gpui_kit::component` form and button widgets in `apps/gpui/ui/src/` with `ely-gpui-component` equivalents:

1. **People forms (`ely::forms::Select`):** Migrate [`people/forms.rs`](../../apps/gpui/ui/src/people/forms.rs) from `gpui_kit::component::select::Select` / `SearchableVec` to `ely::forms::{Choice, Select}`.
2. **Soundscape sliders (`ely::forms::Slider`):** Migrate [`soundscape/mod.rs`](../../apps/gpui/ui/src/soundscape/mod.rs) and [`soundscape/controls.rs`](../../apps/gpui/ui/src/soundscape/controls.rs) from `gpui_kit::component::slider::Slider` to `ely::forms::Slider`.
3. **Project mail and inventory link buttons (`ely::buttons::Button`):** Migrate [`projects/links.rs`](../../apps/gpui/ui/src/projects/links.rs) and [`projects/mail.rs`](../../apps/gpui/ui/src/projects/mail.rs) from `gpui_kit::component::button::Button` to `ely::buttons::{Button, ButtonVariant}`.

## Track 7: Agent Panel Live Views & Saved Views (P1 Tasks)

Complete the P1 agent view and saved view tasks ([`gpui-live-views`](../tasks/gpui-live-views.md), [`task-336bf8f4-9597-436e-84bb-ef6ff052ce59`](../tasks/task-336bf8f4-9597-436e-84bb-ef6ff052ce59.md), [`gpui-saved-views`](../tasks/gpui-saved-views.md)):

1. **A2UI view document validation in Rust ([`task-336bf8f4-9597-436e-84bb-ef6ff052ce59`](../tasks/task-336bf8f4-9597-436e-84bb-ef6ff052ce59.md)):** Validate A2UI view documents against the Rust component registry (catalog ID, root existence, acyclic parent-child graph, unknown component types, required props, action bindings, and data-class boundaries) with structured errors.
2. **Live views in the agent panel ([`gpui-live-views`](../tasks/gpui-live-views.md)):** Render validated live A2UI / structured view blocks inside [`agent_seat.rs`](../../apps/gpui/ui/src/agent_seat.rs) and [`ask/`](../../apps/gpui/ui/src/ask/mod.rs) using Ely components (`Card`, `DataTable`, `MetricTile`, `DescriptionList`, `Badge`, `Button`).
3. **Saved views reload without a model ([`gpui-saved-views`](../tasks/gpui-saved-views.md)):** Persist and reload saved views directly from deterministic query recipes so refreshing a saved view updates data without invoking a model.

## Track 8: Phone Surface Consolidation (`phone_*` Views onto Ely)

Consolidate the iPhone surfaces onto the shared Ely component vocabulary:

1. **Phone Today, Mail, People, Finance, Inventory, and Settings:** Upgrade [`phone_today.rs`](../../apps/gpui/ui/src/phone_today.rs), [`phone_mail/view.rs`](../../apps/gpui/ui/src/phone_mail/view.rs), [`phone_people.rs`](../../apps/gpui/ui/src/phone_people.rs), [`phone_finance.rs`](../../apps/gpui/ui/src/phone_finance.rs), [`phone_inventory.rs`](../../apps/gpui/ui/src/phone_inventory.rs), and [`phone_settings.rs`](../../apps/gpui/ui/src/phone_settings.rs) to use Ely `MetricTile`, `Badge`, `DescriptionList`, `InlineMessage`, `SettingRow`, and `AdaptedMailProposal` / `AdaptedMailReader`.

## Track 9: Final `gpui_kit::component::input` & `Checkbox` Elimination (`ely::forms`)

Eliminate legacy single-line `gpui_kit::component::input::{Input, InputState}`, multi-line `Textarea`, and `Checkbox` usages across desktop and shared views:

1. **Projects, Home cards, and Block Editor checkbox:** Migrate [`projects/mod.rs`](../../apps/gpui/ui/src/projects/mod.rs), [`projects/tables.rs`](../../apps/gpui/ui/src/projects/tables.rs), [`projects/task_panel.rs`](../../apps/gpui/ui/src/projects/task_panel.rs), [`projects/board.rs`](../../apps/gpui/ui/src/projects/board.rs), and [`apps/gpui/src/home/cards.rs`](../../apps/gpui/src/home/cards.rs) onto `ely::forms::{Input, InputEvent, TextInput}`, and migrate `Checkbox` in [`editor/block_view.rs`](../../apps/gpui/ui/src/editor/block_view.rs) onto `ely::forms::Checkbox`.
2. **Scratchpad, Clipper, Posts, and Music textareas:** Migrate multi-line inputs in [`scratchpad.rs`](../../apps/gpui/ui/src/scratchpad.rs), [`clipper/composer.rs`](../../apps/gpui/ui/src/clipper/composer.rs), [`clipper/settings.rs`](../../apps/gpui/ui/src/clipper/settings.rs), [`posts/mod.rs`](../../apps/gpui/ui/src/posts/mod.rs), and [`music/forms.rs`](../../apps/gpui/ui/src/music/forms.rs) onto `ely::forms::{Textarea, TextInput}`.

## Track 10: Documents & Clipper Overhaul + Evidence-Backed Document Proposals

Overhaul the Documents queue and viewer onto Ely components and satisfy [`gpui-document-proposals`](../tasks/gpui-document-proposals.md):

1. **Evidence-backed proposal model & corpus verification (`sjel_gpui_core::document_queue`):** Represent document summary provenance (engine, revision, evidence page, evidence span, generated-at, data class, review outcome) and keep rule vs. model category proposals as distinct rows with explicit agreement/conflict calculation and c2/c3 data-class guards, verified against a frozen synthetic German/English corpus.
2. **Documents queue & viewer UI ([`documents/mod.rs`](../../apps/gpui/ui/src/documents/mod.rs), [`documents/viewer.rs`](../../apps/gpui/ui/src/documents/viewer.rs)):** Render provenance metadata, rule/model proposal comparison rows, agreement/conflict badges, and honest absent-summary states with Ely `Badge`, `DescriptionList`, `Card`, and `InlineMessage`.

## Track 11: People Detail Overhaul & Sourced Interaction History

Upgrade the People detail inspector and satisfy [`gpui-people-history`](../tasks/gpui-people-history.md):

1. **Sourced interaction history (`sjel_gpui_core::people` + [`people/detail.rs`](../../apps/gpui/ui/src/people/detail.rs)):** Derive dated interaction history strictly from explicit owner links (`note_ref`, project/record links, dated place facts, and verified introductions) while respecting data-class boundaries and never inferring identity from lexical similarity.
2. **Ely timeline & detail presentation:** Render the sourced interaction history and person attributes in [`people/detail.rs`](../../apps/gpui/ui/src/people/detail.rs) using Ely `Badge`, `DescriptionList`, and `Section`.

## Track 12: Finder Search Quality & Packs Matrix Cost

Close the two P2 measurement and quality tasks ([`gpui-search-quality`](../tasks/gpui-search-quality.md) and [`gpui-packs-performance`](../tasks/gpui-packs-performance.md)):

1. **Finder non-ASCII & diacritic case-folding ([`apps/gpui/core/src/search.rs`](../../apps/gpui/core/src/search.rs)):** Reproduce consequential non-ASCII/diacritic case-folding misses (German umlauts/`ß`, Nordic `ø`/`å`/`æ`, accented Latin names and places) and normalize folding in `sjel_gpui_core::search` while preserving local-first ranking and per-source bounds.
2. **Packs matrix frame-cost measurement & Ely controls ([`apps/gpui/ui/src/packs/matrix.rs`](../../apps/gpui/ui/src/packs/matrix.rs), [`packs/mod.rs`](../../apps/gpui/ui/src/packs/mod.rs)):** Measure p50/p95/max render frame times for the Packs pinned-header matrix against the 8.3 ms (120 Hz) budget, retain row identity, and consolidate Packs badges and filter pills onto Ely.

## Track 13: Travel Historical Place Backfill & Compound Stop Disambiguation

Protect historical trip backfill against compound multi-stop destination strings ([`gpui-travel-backfill`](../tasks/gpui-travel-backfill.md)):

1. **Compound stop detection & backfill classification (`sjel_gpui_core::travel::places`):** Detect compound destination strings (`"Berlin / Potsdam"`, `"Tokyo & Kyoto"`, `"München → Salzburg"`, `"Wien und Graz"`) before coordinate backfill so automatic geocoding never guesses a single point, keeping one owner coordinate per resolved stop.
2. **Travel overview disambiguation UI ([`travel/overview.rs`](../../apps/gpui/ui/src/travel/overview.rs), [`travel/mod.rs`](../../apps/gpui/ui/src/travel/mod.rs)):** Surface an Ely warning `Badge` (`"Compound destination · split before locating"`), candidate stop names, and a `"Choose split in Trip Editor"` action on compound stops.

## Track 14: Agent-Driven Navigation & Follow Mode

Define and wire read-only workspace follow intents from the agent step stream ([`gpui-agent-follow`](../tasks/gpui-agent-follow.md)):

1. **Typed read-only follow intents (`sjel_gpui_core::agent`):** Derive `AgentFollowIntent` (`OpenTarget`, `FinderSearch`, `FocusProposal`) from read-only search/navigation tool calls and A2UI owner-mode proposals while ignoring write or shell-execution calls.
2. **Agent seat follow bar ([`agent_seat.rs`](../../apps/gpui/ui/src/agent_seat.rs)):** Add an opt-in `"Follow: On/Off"` toggle and `"Open in column"` action chip emitting `AgentSeatEvent::Follow`.

## Track 15: Complete Projects `InputState` Migration & Kanban Board Polish

Complete the migration of all single-line inputs in [`apps/gpui/ui/src/projects/`](../../apps/gpui/ui/src/projects/mod.rs) onto `ely::forms::TextInput`:

1. **Projects inputs (`ely::forms::{Input, InputEvent, TextInput}`):** Migrate `search_input`, `notes_search_input`, `title_edit`, `new_task`, `due_edit`, `new_project`, `new_note`, and `task_title_edit` across [`projects/mod.rs`](../../apps/gpui/ui/src/projects/mod.rs), [`projects/tables.rs`](../../apps/gpui/ui/src/projects/tables.rs), [`projects/board.rs`](../../apps/gpui/ui/src/projects/board.rs), and [`projects/render.rs`](../../apps/gpui/ui/src/projects/render.rs).
2. **Kanban board Ely badges ([`projects/board.rs`](../../apps/gpui/ui/src/projects/board.rs)):** Replace raw subtask progress chips and column count pills with Ely `Badge`.

## Track 16: Phone Deep Links, Signed Share Outbox, Trip Edit Replay & Synthetic Demo Pairing

Close the remaining Priority 4 phone contract and sync tasks ([`gpui-phone-links`](../tasks/gpui-phone-links.md), [`gpui-share-extension`](../tasks/gpui-share-extension.md), [`gpui-travel-phone-writes`](../tasks/gpui-travel-phone-writes.md), [`gpui-demo-phone`](../tasks/gpui-demo-phone.md)):

1. **Phone deep links (`sjel_gpui_core::route`):** Route `sjel://` and `calendar:YYYY-MM-DD` reminder links into `PhoneDeepLink` with known-first-segment fallback and safe `Page::Home` fallback.
2. **Signed share outbox, trip edit replay, and synthetic demo seed (`sjel_gpui_core::sync`):** Enforce paired-device keychain signatures and at-most-once delivery in `ShareOutbox`, replay `TripEditEnvelope` with explicit HTTP 409 stale-revision refusal, and validate `SyntheticDemoSeed` so production snapshots never seed demo pairing.

## Track 17: Eliminate Legacy `TextareaState` in Block Editor (`editor/`)

Remove the last `gpui_kit::component::input` widgets in `apps/gpui/`:

1. **Block & preview editors (`ely::forms::{TextInput, Textarea}`):** Migrate [`editor/types.rs`](../../apps/gpui/ui/src/editor/types.rs), [`editor/mod.rs`](../../apps/gpui/ui/src/editor/mod.rs), [`editor/block_view.rs`](../../apps/gpui/ui/src/editor/block_view.rs), and [`editor/preview.rs`](../../apps/gpui/ui/src/editor/preview.rs) from `gpui_kit::component::input::{TextareaState, Textarea, InputEvent}` onto `ely::forms::{TextInput, Textarea, InputEvent}`.

## Track 18: Calendar, Docs, Scout & Backup Surface Polish onto Ely

Consolidate remaining hand-styled cards, status pills, and empty states onto Ely primitives:

1. **Docs, Scout, Backup, and Calendar (`ely::layout::Card`, `ely::data_display::{Badge, DescriptionList, EmptyState}`, `ely::feedback::InlineMessage`):** Migrate [`docs/mod.rs`](../../apps/gpui/ui/src/docs/mod.rs), [`scout/mod.rs`](../../apps/gpui/ui/src/scout/mod.rs), [`backup/mod.rs`](../../apps/gpui/ui/src/backup/mod.rs), and [`calendar/page.rs`](../../apps/gpui/ui/src/calendar/page.rs) off ad-hoc status pills and hand-styled containers onto Ely primitives.

## Track 19: Projects Weekly Review & `gpui-project-tasks` Acceptance

Polish [`projects/review.rs`](../../apps/gpui/ui/src/projects/review.rs) onto Ely `Card`, `Badge`, and `DescriptionList`, and verify and record acceptance evidence in [`gpui-project-tasks`](../tasks/gpui-project-tasks.md).

## Track 20: `packs-gpui-sync` & `tools/work` Task Status Cleanup

Complete [`packs-gpui-sync`](../tasks/packs-gpui-sync.md) (refresh GPUI guidance copies) and reconcile `docs/tasks/gpui-*.md` frontmatter statuses cleanly where acceptance evidence is complete.

## Track 21: Remove Remaining `gpui_kit::component::input::Escape` & Inline Banner Sweep

Remove leftover `gpui_kit::component::input::Escape` imports in [`projects/mod.rs`](../../apps/gpui/ui/src/projects/mod.rs) and [`projects/tables.rs`](../../apps/gpui/ui/src/projects/tables.rs), and replace raw inline error/warning divs in `inventory/`, `finance/`, `interior/`, and `music/` with `ely::feedback::InlineMessage`.

## Track 22: `systems/`, `media/`, `backup/`, & `self_view/` Status & Telemetry Consolidation

Upgrade raw status labels and summary indicators in [`systems/overview.rs`](../../apps/gpui/ui/src/systems/overview.rs), [`media/runs.rs`](../../apps/gpui/ui/src/media/runs.rs), [`media/inspector.rs`](../../apps/gpui/ui/src/media/inspector.rs), [`backup/contracts.rs`](../../apps/gpui/ui/src/backup/contracts.rs), and [`self_view/mod.rs`](../../apps/gpui/ui/src/self_view/mod.rs) onto Ely `Badge` and `InlineMessage`.

## Track 23: `phone_mail` `TextView` Cleanup & `scratchpad` Input Audit

Clean up `gpui_kit::component::text::TextView` in [`phone_mail/view.rs`](../../apps/gpui/ui/src/phone_mail/view.rs) and audit [`scratchpad.rs`](../../apps/gpui/ui/src/scratchpad.rs) `TextareaState` usage.

## Track 24: Shared `live_view` Renderer (`sjel_gpui_ui::live_view`)

Extract `render_live_view` and `A2uiClientState` wiring out of [`agent_seat.rs`](../../apps/gpui/ui/src/agent_seat.rs) into a shared public [`live_view.rs`](../../apps/gpui/ui/src/live_view.rs) module so **Agent Seat**, **Mac Home/Today**, and **Phone Today** render the same validated `ValidatedA2uiView` / `SavedA2uiView` catalog onto Ely primitives (`Card`, `MetricStrip`, `MetricTile`, `DescriptionList`, `DataTable`, `Badge`, `Button`).

## Track 25: Pin & Reload Saved Generative Views on Home (`"Pin to Home"`)

Add `"Pin to Home"` / `"Unpin"` actions on validated live A2UI views, persist pinned `SavedA2uiView` records in [`home/prefs.rs`](../../apps/gpui/src/home/prefs.rs), and render them in [`home/mod.rs`](../../apps/gpui/src/home/mod.rs) with zero model calls on startup, preserving data-class/read-route badges and owner-mode proposal routing.

## Track 26: Adaptive `HomeMode::Auto` Context Engine & Explainability

Move Home card ordering into a deterministic planner in `sjel-gpui-core` driven by live signals (imminent travel window, urgent/approval/degraded pressure, Focus mode) and surface a concise adaptive reason badge in [`home/mod.rs`](../../apps/gpui/src/home/mod.rs).

## Track 27: Endel-Style Audio-Reactive Hero & Mood Tiles (`soundscape/controls.rs`, `soundscape/mod.rs`)

Embed the audio-reactive `viz::Field` canvas into the shared `Soundscape` transport hero with a floating tactile Play/Pause orb, Ely `Badge` state indicators, `InlineMessage` device error notices, `crate::ely::primitives::IconName`, and 2-column mood preset cards.

## Track 28: Focus Session Timer, Energy Control & Mixer Polish (`soundscape/mod.rs`, `soundscape/controls.rs`)

Add a timed focus session strip (`∞`, `15m`, `25m`, `45m`) with an Ely `ProgressBar` countdown that auto-pauses playback on completion, an Intensity/Energy Ely `Slider`, and an active-layer count `Badge` on the Mixer header.

## Track 29: Desktop Slideover Immersive Canvas & Circadian Context (`soundscape/slideover.rs`)

Expand the slideover's `viz::Field` hero (`190px` → `220px`) with floating circadian/listening-context `Badge` pills (`Preset`, `Scenario`, `Phase`) and frosted bottom captioning.

## Track 30: Phone Today Adaptive Parity (`phone_today/view.rs`)

Wire `sjel_gpui_core::home::plan_home_layout` into `PhoneToday` so iPhone Today surfaces the adaptive reason `Badge` and renders pinned `SavedA2uiView` cards via `crate::live_view::render_live_view`.

## Track 31: Soundscape Slideover Focus Timer & Mood Parity (`soundscape/slideover.rs`)

Bring the timed Focus Session strip (`∞`, `15m`, `25m`, `45m` with `ProgressBar` countdown and auto-pause) and preset mood subtitles into macOS `SoundscapeSlideover`.

## Track 32: Interactive Generative `TextField` in `live_view.rs`

Upgrade `ComponentKind::TextField` in [`live_view.rs`](../../apps/gpui/ui/src/live_view.rs) to support interactive `ely::forms::Input` (`TextInput`) draft editing synced with `A2uiClientState`.

## Related tasks

- [Prove read-only Ely Mail parity](../tasks/gpui-mail-adapter.md).
- [Evaluate block editor round-trip parity](../tasks/gpui-editor-adapter.md).
- [Accept task and project controls](../tasks/gpui-project-tasks.md).
- [Fit shared views to the phone width](../tasks/gpui-phone-polish.md).
- [Validate A2UI view documents against the registry in Rust](../tasks/task-336bf8f4-9597-436e-84bb-ef6ff052ce59.md).
- [Render live views in the agent panel](../tasks/gpui-live-views.md).
- [Reload saved views without a model](../tasks/gpui-saved-views.md).
- [Accept evidence-backed document proposals](../tasks/gpui-document-proposals.md).
- [Show sourced person interaction history](../tasks/gpui-people-history.md).
- [Measure Finder search misses](../tasks/gpui-search-quality.md).
- [Measure remaining Packs matrix cost](../tasks/gpui-packs-performance.md).
- [Choose safe historical place resolution](../tasks/gpui-travel-backfill.md).
- [Specify agent-driven navigation](../tasks/gpui-agent-follow.md).
- [Register phone deep links](../tasks/gpui-phone-links.md).
- [Capture phone shares exactly once](../tasks/gpui-share-extension.md).
- [Add phone trip edits through the owner](../tasks/gpui-travel-phone-writes.md).
- [Pair the phone to synthetic demo data](../tasks/gpui-demo-phone.md).

## Status

1. **Track 1 — Shared Primitives & Chrome Extraction into Ely:** Extracted `ShortcutHint`, `KeyBar`, `ProgressBar`, `MetricTile`, `MetricStrip`, `QuickActionTile`, `BottomTabItem`, `BottomTabBar`, and `NavHeader` into `ely-gpui-component` (`2451b65f8729abbe95284ff9f989e0e44a39d1ca`), tested on both `gpui-pre` and `gpui-fast`, and routed [`apps/gpui/ui/src/primitives/`](../../apps/gpui/ui/src/primitives/mod.rs) and [`apps/gpui/ui/src/nav.rs`](../../apps/gpui/ui/src/nav.rs) through them.
2. **Track 2 — Mail, Feed & Inbox Triage Overhaul + Ely Mail Adapter:** Added [`apps/gpui/ui/src/mail/adapter.rs`](../../apps/gpui/ui/src/mail/adapter.rs) (`AdaptedMailProposal`, `adapt_mail_list`, `AdaptedMailReader`, `sanitize_preview_body`), upgraded [`mail/`](../../apps/gpui/ui/src/mail/mod.rs), [`feed/reading.rs`](../../apps/gpui/ui/src/feed/reading.rs), and [`inbox/triage.rs`](../../apps/gpui/ui/src/inbox/triage.rs) to Ely `Badge`, `Button`, `InlineMessage`, `filter_pill`, and `key_bar`, and recorded evidence in [`gpui-mail-adapter`](../tasks/gpui-mail-adapter.md).
3. **Track 3 — Block Editor & Projects Wave 2:** Added [`apps/gpui/ui/src/editor/adapter.rs`](../../apps/gpui/ui/src/editor/adapter.rs) comparing `sjel_gpui_core::blocks::Doc` against `ely::documents::{BlockData, BlockKind}`, migrated [`editor/mod.rs`](../../apps/gpui/ui/src/editor/mod.rs) status bar and footer to Ely `Badge`, `WordCount`, `ReadingTime`, and `key_bar`, migrated [`projects/task_panel.rs`](../../apps/gpui/ui/src/projects/task_panel.rs) to Ely `Button` and `DescriptionList`, added project status and inventory readiness badges to [`projects/render.rs`](../../apps/gpui/ui/src/projects/render.rs), and recorded evidence in [`gpui-editor-adapter`](../tasks/gpui-editor-adapter.md).
4. **Track 4 — Travel Overhaul & Mobile-Width Polish:** Migrated [`travel/overview.rs`](../../apps/gpui/ui/src/travel/overview.rs) and [`travel/itinerary.rs`](../../apps/gpui/ui/src/travel/itinerary.rs) pills and day filters to Ely `Badge`, `Tag`, and `Button`, implemented mobile-width fixes and headless tests across Calendar (`mobile_week_strip`), Projects (`render.rs`, `tables.rs`), and `NoteEditor` (`editor/mod.rs`), and recorded evidence in [`gpui-phone-polish`](../tasks/gpui-phone-polish.md).
5. **Track 5 — Desktop Shell, Home/Today & Settings Consolidation:** Added `SettingRow` (`ely::forms`) and `Kbd::{from_keystroke, for_action}` (`ely::typography`) to `ely-gpui-component` (`b0600bc5bf5d0d6fc05fc2d37e1af4a2def11155`), and migrated [`header.rs`](../../apps/gpui/src/header.rs), [`sidebar/nav.rs`](../../apps/gpui/src/sidebar/nav.rs), [`home/mod.rs`](../../apps/gpui/src/home/mod.rs), [`home/focus_tasks.rs`](../../apps/gpui/src/home/focus_tasks.rs), [`home/needs_you.rs`](../../apps/gpui/src/home/needs_you.rs), [`settings/mod.rs`](../../apps/gpui/src/settings/mod.rs), [`settings/appearance.rs`](../../apps/gpui/src/settings/appearance.rs), and [`settings/shortcuts.rs`](../../apps/gpui/src/settings/shortcuts.rs) onto Ely `Button`, `IconButton`, `Badge`, `Checkbox`, `Switch`, `SettingRow`, and `Kbd`.
6. **Track 6 — Remaining `gpui_kit` Form & Control Elimination in `sjel-gpui-ui`:** Migrated [`people/forms.rs`](../../apps/gpui/ui/src/people/forms.rs) from `gpui_kit::component::select::Select` / `SearchableVec` to Ely `Combobox` and `Choice`, [`soundscape/mod.rs`](../../apps/gpui/ui/src/soundscape/mod.rs) and [`soundscape/controls.rs`](../../apps/gpui/ui/src/soundscape/controls.rs) from `gpui_kit::component::slider::Slider` to Ely `Slider`, and [`projects/links.rs`](../../apps/gpui/ui/src/projects/links.rs) and [`projects/mail.rs`](../../apps/gpui/ui/src/projects/mail.rs) from `gpui_kit::component::button::Button` to Ely `Button`.
7. **Track 7 — Agent Panel Live Views & Saved Views:** Implemented [`apps/gpui/core/src/a2ui.rs`](../../apps/gpui/core/src/a2ui.rs) (`A2uiDocument`, `ValidatedA2uiView`, `SavedA2uiView`, `A2uiClientState`, `A2uiProposalIntent`, `validate_document`, `sanitize_inert_text`), wired live and saved A2UI views into [`agent.rs`](../../apps/gpui/core/src/agent.rs) and [`agent_seat.rs`](../../apps/gpui/ui/src/agent_seat.rs), and recorded evidence in [`task-336bf8f4-9597-436e-84bb-ef6ff052ce59`](../tasks/task-336bf8f4-9597-436e-84bb-ef6ff052ce59.md), [`gpui-saved-views`](../tasks/gpui-saved-views.md), and [`gpui-live-views`](../tasks/gpui-live-views.md).
8. **Track 8 — Phone Surface Consolidation:** Upgraded [`phone_today/view.rs`](../../apps/gpui/ui/src/phone_today/view.rs), [`phone_mail/view.rs`](../../apps/gpui/ui/src/phone_mail/view.rs), [`phone_people/view.rs`](../../apps/gpui/ui/src/phone_people/view.rs), [`phone_finance/view.rs`](../../apps/gpui/ui/src/phone_finance/view.rs), [`phone_inventory/view.rs`](../../apps/gpui/ui/src/phone_inventory/view.rs), and [`phone_settings/view.rs`](../../apps/gpui/ui/src/phone_settings/view.rs) onto Ely `MetricStrip`, `MetricTile`, `Badge`, `DescriptionList`, `InlineMessage`, `SettingRow`, and `AdaptedMailProposal` / `AdaptedMailReader`.
9. **Track 9 — Final `gpui_kit::component::input` & `Checkbox` Elimination:** Migrated `Checkbox` in [`editor/block_view.rs`](../../apps/gpui/ui/src/editor/block_view.rs), `quick_input` in [`scratchpad.rs`](../../apps/gpui/ui/src/scratchpad.rs) and [`home/cards.rs`](../../apps/gpui/src/home/cards.rs), inspector inputs in [`projects/task_panel.rs`](../../apps/gpui/ui/src/projects/task_panel.rs), and multi-line text inputs in [`posts/mod.rs`](../../apps/gpui/ui/src/posts/mod.rs), [`clipper/composer.rs`](../../apps/gpui/ui/src/clipper/composer.rs), [`clipper/settings.rs`](../../apps/gpui/ui/src/clipper/settings.rs), and [`music/`](../../apps/gpui/ui/src/music/mod.rs) onto `ely::forms::{Checkbox, Input, InputEvent, TextInput}`.
10. **Track 10 — Documents & Clipper Overhaul + Evidence-Backed Document Proposals:** Added `ProposalAgreement`, `ProposalEvidenceView`, `can_leave_host`, `can_enter_model_prompt`, and `evaluate_proposal_run` in [`apps/gpui/core/src/document_queue.rs`](../../apps/gpui/core/src/document_queue.rs), upgraded [`documents/proposals.rs`](../../apps/gpui/ui/src/documents/proposals.rs) onto Ely `Badge`, `DescriptionList`, and `InlineMessage`, and recorded evidence in [`gpui-document-proposals`](../tasks/gpui-document-proposals.md).
11. **Track 11 — People Detail Overhaul & Sourced Interaction History:** Added `SourcedInteraction`, `SourcedGraphInteraction`, `CandidateNoteMention`, and `Person::sourced_interactions` in [`apps/gpui/core/src/people.rs`](../../apps/gpui/core/src/people.rs), rendered the `"SOURCED INTERACTION HISTORY"` table and Ely `Badge` pills in [`people/detail.rs`](../../apps/gpui/ui/src/people/detail.rs), and recorded evidence in [`gpui-people-history`](../tasks/gpui-people-history.md).
12. **Track 12 — Finder Search Quality & Packs Matrix Cost:** Added non-ASCII case/diacritic/ligature folding (`fold_search_text`) in [`apps/gpui/core/src/search.rs`](../../apps/gpui/core/src/search.rs), upgraded [`packs/mod.rs`](../../apps/gpui/ui/src/packs/mod.rs) and [`packs/cards.rs`](../../apps/gpui/ui/src/packs/cards.rs) onto Ely `InlineMessage` and `Badge` with headless 120 Hz matrix frame-cost verification, and recorded evidence in [`gpui-search-quality`](../tasks/gpui-search-quality.md) and [`gpui-packs-performance`](../tasks/gpui-packs-performance.md).
13. **Track 13 — Travel Historical Place Backfill & Compound Stop Disambiguation:** Added `compound_candidates`, `is_compound_place`, `BackfillClassification`, and `classify_backfill` in [`apps/gpui/core/src/travel/places.rs`](../../apps/gpui/core/src/travel/places.rs), surfaced compound stop split badges and Trip Editor actions in [`travel/overview.rs`](../../apps/gpui/ui/src/travel/overview.rs) and [`travel/mod.rs`](../../apps/gpui/ui/src/travel/mod.rs), and recorded evidence in [`gpui-travel-backfill`](../tasks/gpui-travel-backfill.md).
14. **Track 14 — Agent-Driven Navigation & Follow Mode:** Added `AgentFollowIntent` and `derive_follow_intent` in [`apps/gpui/core/src/agent.rs`](../../apps/gpui/core/src/agent.rs), wired `AgentSeatEvent::Follow` and the follow toggle/bar in [`apps/gpui/ui/src/agent_seat.rs`](../../apps/gpui/ui/src/agent_seat.rs), and recorded evidence in [`gpui-agent-follow`](../tasks/gpui-agent-follow.md).
15. **Track 15 — Complete Projects `InputState` Migration & Kanban Board Polish:** Migrated all remaining single-line `InputState` fields in [`projects/mod.rs`](../../apps/gpui/ui/src/projects/mod.rs), [`projects/tables.rs`](../../apps/gpui/ui/src/projects/tables.rs), [`projects/board.rs`](../../apps/gpui/ui/src/projects/board.rs), and [`projects/render.rs`](../../apps/gpui/ui/src/projects/render.rs) onto `ely::forms::{Input, InputEvent, TextInput}`, and upgraded Kanban board subtask and count badges onto Ely `Badge`.
16. **Track 16 — Phone Deep Links, Signed Share Outbox, Trip Edit Replay & Synthetic Demo Pairing:** Implemented `PhoneDeepLink` and `resolve_phone_deep_link` in [`apps/gpui/core/src/route.rs`](../../apps/gpui/core/src/route.rs), added `SignedShareItem`, `ShareOutbox`, `TripEditEnvelope`, `trip_edit`, and `SyntheticDemoSeed` in [`apps/gpui/core/src/sync.rs`](../../apps/gpui/core/src/sync.rs), and recorded evidence in [`gpui-phone-links`](../tasks/gpui-phone-links.md), [`gpui-share-extension`](../tasks/gpui-share-extension.md), [`gpui-travel-phone-writes`](../tasks/gpui-travel-phone-writes.md), and [`gpui-demo-phone`](../tasks/gpui-demo-phone.md).
17. **Track 17 — Legacy `TextareaState` Assessment in Block Editor (`editor/`):** Audited [`editor/types.rs`](../../apps/gpui/ui/src/editor/types.rs), [`editor/mod.rs`](../../apps/gpui/ui/src/editor/mod.rs), [`editor/block_view.rs`](../../apps/gpui/ui/src/editor/block_view.rs), and [`editor/preview.rs`](../../apps/gpui/ui/src/editor/preview.rs); kept `gpui_kit::component::input::TextareaState` until `ely::forms::TextInput` adds `selected_range()`, `marked_text_range()`, `auto_grow()`, `submit_on_enter()`, and block-crossing key actions (`Backspace`, `MoveUp`, `MoveDown`, `Escape`) needed by `LiveBlock`.
18. **Track 18 — Calendar & Scout Surface Polish onto Ely:** Migrated [`scout/mod.rs`](../../apps/gpui/ui/src/scout/mod.rs) notices, errors, and status filter toggles onto `InlineMessage` and `filter_pill`, and migrated the Day/Week/Month switcher in [`calendar/page.rs`](../../apps/gpui/ui/src/calendar/page.rs) onto `sub_tabs`.
19. **Track 19 — Projects Weekly Review Polish:** Upgraded overdue due-date, blocker, and active-project count chips in [`projects/review.rs`](../../apps/gpui/ui/src/projects/review.rs) onto Ely `Badge`.
20. **Track 20 — `packs-gpui-sync` Harness Sync:** Synced `Packs/coding/skills/gpui` across Claude Code, Codex, opencode, and GitHub Copilot CLI via `tools/harnesses sync coding` with zero remaining drift, and updated [`packs-gpui-sync`](../tasks/packs-gpui-sync.md).
21. **Track 21 — `gpui_kit::component::input::Escape` Removal & Inline Banner Sweep:** Removed leftover `Escape` imports in [`projects/mod.rs`](../../apps/gpui/ui/src/projects/mod.rs) and [`projects/tables.rs`](../../apps/gpui/ui/src/projects/tables.rs) (routing `InputEvent::Cancel` on `TextInput`), and migrated inline error/warning banners in [`finance/ideas.rs`](../../apps/gpui/ui/src/finance/ideas.rs), [`finance/import.rs`](../../apps/gpui/ui/src/finance/import.rs), [`music/mod.rs`](../../apps/gpui/ui/src/music/mod.rs), and [`interior/planning.rs`](../../apps/gpui/ui/src/interior/planning.rs) onto `InlineMessage`.
22. **Track 22 — `systems/`, `media/`, & `self_view/` Status Consolidation:** Migrated status/error notices in [`systems/overview.rs`](../../apps/gpui/ui/src/systems/overview.rs) and [`self_view/mod.rs`](../../apps/gpui/ui/src/self_view/mod.rs) onto `InlineMessage`, and migrated `refused`/`failed` pills in [`media/runs.rs`](../../apps/gpui/ui/src/media/runs.rs) onto Ely `Badge`.
23. **Track 23 — `phone_mail` `TextView` & `scratchpad` Input Audit:** Confirmed `TextView::markdown` in [`phone_mail/view.rs`](../../apps/gpui/ui/src/phone_mail/view.rs) is required for sanitized Markdown email bodies and `TextareaState` in [`scratchpad.rs`](../../apps/gpui/ui/src/scratchpad.rs) is required for auto-growing multi-line note capture.
24. **Track 24 — Shared `live_view` Renderer (`sjel_gpui_ui::live_view`):** Extracted `render_live_view`, `render_a2ui_node`, `tone_of`, and `ProposeHandler` out of [`agent_seat.rs`](../../apps/gpui/ui/src/agent_seat.rs) into [`live_view.rs`](../../apps/gpui/ui/src/live_view.rs) and exported it from [`apps/gpui/ui/src/lib.rs`](../../apps/gpui/ui/src/lib.rs).
25. **Track 25 — Pin & Reload Saved Generative Views on Home (`"Pin to Home"`):** Added `"Pin to Home"` / `AgentSeatEvent::PinToHome` in [`agent_seat.rs`](../../apps/gpui/ui/src/agent_seat.rs), persisted `pinned_views: Vec<SavedA2uiView>` in [`home/prefs.rs`](../../apps/gpui/src/home/prefs.rs), wired `FinderEvent::PinToHome` through [`finder/`](../../apps/gpui/src/finder/mod.rs) and [`workspace/mod.rs`](../../apps/gpui/src/workspace/mod.rs), and rendered pinned views with `"Unpin"` and owner-mode proposal routing in [`home/mod.rs`](../../apps/gpui/src/home/mod.rs).
26. **Track 26 — Adaptive `HomeMode::Auto` Context Engine & Explainability:** Added deterministic `plan_home_layout` (`HomeSignals`, `HomeRailTab`, `HomeAdaptivePlan`) in [`apps/gpui/core/src/home.rs`](../../apps/gpui/core/src/home.rs) and wired dynamic rail ordering and the adaptive reason `Badge` into [`home/mod.rs`](../../apps/gpui/src/home/mod.rs).
27. **Track 27 — Endel-Style Audio-Reactive Hero & Mood Tiles:** Embedded `viz::Field` (`viz::frame(viz::canvas(...))`) into the shared `Soundscape` transport hero in [`soundscape/controls.rs`](../../apps/gpui/ui/src/soundscape/controls.rs) and [`soundscape/mod.rs`](../../apps/gpui/ui/src/soundscape/mod.rs) with a floating Play/Pause orb, Ely `Badge` status pills, `InlineMessage` error notices, `crate::ely::primitives::IconName`, and 2-column mood preset cards.
28. **Track 28 — Focus Session Timer, Energy Control & Mixer Polish:** Added `session_timer` (`∞`, `15m`, `25m`, `45m` with Ely `ProgressBar` countdown and auto-pause), an Intensity/Energy Ely `Slider`, and an active-layer count `Badge` on the Mixer header in [`soundscape/mod.rs`](../../apps/gpui/ui/src/soundscape/mod.rs) and [`soundscape/controls.rs`](../../apps/gpui/ui/src/soundscape/controls.rs).
29. **Track 29 — Desktop Slideover Immersive Canvas & Circadian Context:** Expanded `FIELD_HEIGHT` to `220px` and overlaid floating `Live`/`Paused`, preset, and circadian phase `Badge` pills in [`soundscape/slideover.rs`](../../apps/gpui/ui/src/soundscape/slideover.rs).
30. **Track 30 — Phone Today Adaptive Parity:** Wired `sjel_gpui_core::home::plan_home_layout` (`Badge` adaptive reason) and pinned `SavedA2uiView` rendering (`crate::live_view::render_live_view`) into [`phone_today/view.rs`](../../apps/gpui/ui/src/phone_today/view.rs).
31. **Track 31 — Soundscape Slideover Mood Parity:** Enriched the primary mode `ToggleItem` labels in [`soundscape/slideover.rs`](../../apps/gpui/ui/src/soundscape/slideover.rs) with mood subtitles (`Focus · Flow`, `Relax · Unwind`, `Sleep · Drift`).
32. **Track 32 — Interactive Generative `TextField` in `live_view.rs`:** Added `DraftInputs` and `render_live_view_interactive` in [`live_view.rs`](../../apps/gpui/ui/src/live_view.rs) so `ComponentKind::TextField` mounts live `ely::forms::Input` (`TextInput`) controls when draft bindings are provided.
33. **Track 33 — Soundscape `preset_mood` Consolidation:** Shared `preset_mood` from [`soundscape/controls.rs`](../../apps/gpui/ui/src/soundscape/controls.rs) across both the shared `Soundscape` drawer and [`soundscape/slideover.rs`](../../apps/gpui/ui/src/soundscape/slideover.rs).
34. **Track 34 — Remove Last `gpui_kit::component::IconName` Usages:** Migrated `QuickAction.icon` in [`primitives/cards.rs`](../../apps/gpui/ui/src/primitives/cards.rs) and [`home/mod.rs`](../../apps/gpui/src/home/mod.rs) onto `ely::primitives::{Icon, IconName}`, leaving zero `gpui_kit::component::IconName` imports across `apps/gpui/`.
16. **Track 16 — Phone Deep Links, Signed Share Outbox, Trip Edit Replay & Synthetic Demo Pairing:** Added `PhoneDeepLink` and `resolve_phone_deep_link` in [`apps/gpui/core/src/route.rs`](../../apps/gpui/core/src/route.rs), added `SignedShareItem`, `ShareOutbox`, `TripEditEnvelope`, and `SyntheticDemoSeed` in [`apps/gpui/core/src/sync.rs`](../../apps/gpui/core/src/sync.rs), and recorded evidence in [`gpui-phone-links`](../tasks/gpui-phone-links.md), [`gpui-share-extension`](../tasks/gpui-share-extension.md), [`gpui-travel-phone-writes`](../tasks/gpui-travel-phone-writes.md), and [`gpui-demo-phone`](../tasks/gpui-demo-phone.md).
