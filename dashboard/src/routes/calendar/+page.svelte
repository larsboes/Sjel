<script lang="ts">
  import { eventItem } from "$lib/inspector/connections";
  import { page } from "$app/state";
  import PageHeader from "$lib/PageHeader.svelte";
  import MonthGrid from "$lib/calendar/MonthGrid.svelte";
  import TimeGrid from "$lib/calendar/TimeGrid.svelte";
  import EntryForm from "$lib/calendar/EntryForm.svelte";
  import CalendarRail from "$lib/calendar/CalendarRail.svelte";
  import { goto } from "$app/navigation";
  import {
    VIEWS,
    addDays,
    coversDay,
    dayKey,
    entryReaderLink,
    freeDaysOf,
    monthDates,
    parseDayKey,
    weekDates,
    type CalendarDay,
    type CalendarView,
  } from "$lib/calendar/types";
  import {
    axonStatus,
    calendar,
    trips,
    type CalendarCommitment,
    type CalendarContext,
    type CalendarEntry,
    type CalendarGoogleExportOptIn,
    type CalendarNewContext,
    type CalendarNewEntry,
    type CalendarNewRhythm,
    type CalendarRhythm,
    type CalendarUpdateContext,
    type CalendarUpdateEntry,
    type TripPlan,
  } from "$lib/api";
  import { inspectorStore } from "$lib/inspector/inspector.svelte";

  const MONTH_YEAR = new Intl.DateTimeFormat("en-GB", { month: "long", year: "numeric" });
  const DAY_MONTH = new Intl.DateTimeFormat("en-GB", { day: "numeric", month: "long" });
  const DAY_MONTH_YEAR = new Intl.DateTimeFormat("en-GB", {
    day: "numeric",
    month: "long",
    year: "numeric",
  });
  const WEEKDAY_FULL = new Intl.DateTimeFormat("en-GB", {
    weekday: "long",
    day: "numeric",
    month: "long",
    year: "numeric",
  });

  let entries = $state<CalendarEntry[]>([]);
  let contexts = $state<CalendarContext[]>([]);
  let rhythms = $state<CalendarRhythm[]>([]);
  let tripPlans = $state<TripPlan[]>([]);
  // The capability's own feasibility verdict for the visible range. Fetched
  // rather than recomputed here: which days are genuinely free is calendar's
  // domain, and a second implementation in the UI would drift from it.
  let freeDays = $state<ReadonlySet<string>>(new Set());
  let loading = $state(true);
  let error = $state("");

  // One anchor date for all three views, so switching keeps your place; only
  // navigation moves it, by month, by week or by day.
  /// What /calendar opens on when nothing in the URL says otherwise.
  const DEFAULT_VIEW: CalendarView = "month";
  let view = $state<CalendarView>(DEFAULT_VIEW);
  let anchor = $state(new Date());
  let anchorMonth = $derived(anchor.getMonth());

  let dates = $derived(
    view === "month"
      ? monthDates(anchor.getFullYear(), anchorMonth)
      : view === "week"
        ? weekDates(anchor)
        : [dayKey(anchor)],
  );

  // The service window is exactly what is on screen: `from` inclusive, `to`
  // exclusive, both at day granularity.
  let windowFrom = $derived(dates[0]);
  let windowTo = $derived(dayKey(addDays(parseDayKey(dates[dates.length - 1]), 1)));

  let days = $derived(buildDays(dates, entries, anchorMonth));
  let rangeLabel = $derived(buildRangeLabel(view, anchor, dates));

  // selection state
  let selectedDay = $state<CalendarDay | null>(null);
  let selectedEndDate = $state<string | null>(null);
  let selectedEntry = $state<CalendarEntry | null>(null);
  let selectedDraft = $state<CalendarNewEntry | null>(null);
  let showEntryForm = $state(false);
  // One counter per rail section. Bumped together: any edit here can change what any
  // of the three has pending, and each section re-fetches only its own window.
  let refreshes = $state({ google: 0, proposals: 0, trips: 0 });
  let googleExports = $state<ReadonlyMap<string, CalendarGoogleExportOptIn>>(new Map());

  function bumpReview() {
    refreshes = {
      google: refreshes.google + 1,
      proposals: refreshes.proposals + 1,
      trips: refreshes.trips + 1,
    };
  }

  function buildDays(dates: string[], entries: CalendarEntry[], month: number): CalendarDay[] {
    const todayKey = dayKey(new Date());
    return dates.map((date) => ({
      date,
      day: Number(date.slice(8)),
      entries: entries.filter((entry) => coversDay(entry, date)),
      isCurrentMonth: Number(date.slice(5, 7)) - 1 === month,
      isToday: date === todayKey,
    }));
  }

  function buildRangeLabel(view: CalendarView, anchor: Date, dates: string[]): string {
    if (view === "month") return MONTH_YEAR.format(anchor);
    if (view === "day") return WEEKDAY_FULL.format(anchor);
    const first = parseDayKey(dates[0]);
    const last = parseDayKey(dates[dates.length - 1]);
    const head = first.getMonth() === last.getMonth()
      ? `${first.getDate()}.`
      : DAY_MONTH.format(first);
    return `${head} – ${DAY_MONTH_YEAR.format(last)}`;
  }

  // Navigation and a view switch both change the window, so the load follows
  // the window rather than any single button.
  let loadToken = 0;

  /** Raising or lowering how binding an entry is. Optimistic: the ring is the
   * fastest edit in this workspace and a round-trip per click would make the
   * column feel broken. A failure reloads, so the grid never keeps a state the
   * server refused. */
  async function onCycleCommitment(entry: CalendarEntry, next: CalendarCommitment) {
    const previous = entry.commitment;
    entries = entries.map((e) => (e.id === entry.id ? { ...e, commitment: next } : e));
    try {
      await calendar.entries.update(entry.id, { commitment: next });
    } catch (cause) {
      error = String(cause);
      entries = entries.map((e) => (e.id === entry.id ? { ...e, commitment: previous } : e));
    }
    void load(windowFrom, windowTo);
  }

  async function load(from: string, to: string) {
    const token = ++loadToken;
    loading = true;
    error = "";
    try {
      // calendar's manifest declares it on-demand, so opening this page is what
      // has to start it — the same thing /travel and /feed do for their backends.
      // Without this the page is only ever reachable on a machine where something
      // else already started the service, which is what a watchdog was papering
      // over. A failure here is not fatal on its own: the requests below produce
      // the real, specific error.
      await axonStatus.start("calendar").catch(() => undefined);
      const [nextEntries, nextContexts, nextRhythms, nextWindows, nextGoogleExports, nextTrips] = await Promise.all([
        calendar.entries.list(from, to),
        calendar.contexts.list(from, to),
        calendar.rhythms.list(),
        // A failure here costs the star, never the grid: the recommendation
        // is a nicety, the entries are the page.
        calendar.windows(from, to).catch(() => null),
        // Export availability is useful in the editor, never required to
        // render the calendar itself.
        calendar.google.exports().catch(() => []),
        trips.list().catch(() => []),
      ]);
      if (token !== loadToken) return;
      entries = nextEntries;
      contexts = nextContexts;
      rhythms = nextRhythms;
      freeDays = nextWindows ? freeDaysOf(nextWindows.windows) : new Set();
      googleExports = new Map(nextGoogleExports.map((optIn) => [optIn.entry_id, optIn]));
      tripPlans = nextTrips;
    } catch (cause) {
      if (token === loadToken) error = String(cause);
    } finally {
      if (token === loadToken) loading = false;
    }
  }

  // Deep links from elsewhere in the dashboard — `?date=…&entry=…` for one
  // entry, `?date=…&context=…` for a planning context. Two steps, because the
  // id alone is not enough: the date moves the window first, so the target is
  // inside the range this page loads anyway and nothing has to be fetched by
  // id. This effect stands before the loading one so the window is already
  // right when the first load goes out.
  $effect(() => {
    const date = page.url.searchParams.get("date");
    const entry = page.url.searchParams.get("entry");
    // Symmetric on purpose. Forcing day for a deep link but never releasing it
    // left the view stuck: open an entry from Home, click Calendar in the nav,
    // and a bare /calendar still rendered a single day. A URL with no
    // parameters is a fresh intent, so it gets the default back.
    if (entry) view = "day";
    else if (!date) view = DEFAULT_VIEW;
    // Reads only the search params, never `view`, so switching view by hand
    // does not re-run this and get overwritten.
    if (date) anchor = parseDayKey(date);
  });

  $effect(() => {
    void load(windowFrom, windowTo);
  });

  // Step two: the entry itself, once its window has landed. Consumed once, so
  // closing the form does not reopen it on the next reload of the same window.
  let openedEntry = "";

  $effect(() => {
    if (loading) return;
    const entryId = page.url.searchParams.get("entry");
    if (!entryId || entryId === openedEntry) return;
    const entry = entries.find((candidate) => candidate.id === entryId);
    if (!entry) return;
    openedEntry = entryId;
    openForm(dayFor(entry.starts_at.slice(0, 10)), { entry });
  });

  function shift(direction: number) {
    const next = new Date(anchor);
    if (view === "month") {
      // Keep the day of month across the jump, clamped for short months.
      const day = next.getDate();
      next.setDate(1);
      next.setMonth(next.getMonth() + direction);
      next.setDate(Math.min(day, new Date(next.getFullYear(), next.getMonth() + 1, 0).getDate()));
    } else {
      next.setDate(next.getDate() + (view === "week" ? 7 : 1) * direction);
    }
    anchor = next;
  }

  function focusDate(date: string) {
    anchor = parseDayKey(date);
    view = "day";
  }

  function dayFor(date: string): CalendarDay {
    return (
      days.find((day) => day.date === date) ?? {
        date,
        day: Number(date.slice(8)),
        entries: entries.filter((entry) => coversDay(entry, date)),
        isCurrentMonth: Number(date.slice(5, 7)) - 1 === anchorMonth,
        isToday: date === dayKey(new Date()),
      }
    );
  }

  function openForm(day: CalendarDay | null, options: {
    endDate?: string;
    entry?: CalendarEntry;
    draft?: CalendarNewEntry;
  } = {}) {
    selectedDay = day;
    selectedEndDate = options.endDate ?? null;
    selectedEntry = options.entry ?? null;
    selectedDraft = options.draft ?? null;
    showEntryForm = true;
  }

  function closeForm() {
    showEntryForm = false;
    selectedEndDate = null;
    selectedEntry = null;
    selectedDraft = null;
  }

  // selection: clicking a day opens the entry form with that date prefilled
  function onSelectDay(day: CalendarDay) {
    openForm(day);
  }

  /// Opening a card reads it; changing it is a second, deliberate step.
  ///
  /// Clicking a chip used to drop straight into the edit form, which meant the
  /// only way to see an entry was a set of input fields — no rendered note, and
  /// a ticket link you could not click. The shared reader already renders every
  /// other kind of item, so a calendar entry goes there too and keeps `Edit`
  /// one click away (`?entry=` still opens this form, which is how it gets back).
  function onSelectEntry(entry: CalendarEntry, day: CalendarDay) {
    inspectorStore.open({ ...eventItem(entry), onEdit: () => openForm(day, { entry }) });
  }

  function onSelectRange(startDate: string, endDate: string) {
    openForm(dayFor(startDate), { endDate });
  }

  function onAddEntry(date: string) {
    openForm(dayFor(date));
  }

  /** An empty hour in a week or day column: same form, prefilled as a timed
   * entry in that hour. Ends stay exclusive, so 09:00 opens 09:00–10:00. */
  function onSelectSlot(date: string, hour: number) {
    const start = String(hour).padStart(2, "0");
    const end = String((hour + 1) % 24).padStart(2, "0");
    const endDate = hour === 23 ? dayKey(addDays(parseDayKey(date), 1)) : date;
    openForm(dayFor(date), {
      draft: {
        kind: "busy",
        title: "",
        all_day: false,
        starts_at: `${date}T${start}:00:00`,
        ends_at: `${endDate}T${end}:00:00`,
      },
    });
  }

  async function onSaveEntry(data: CalendarNewEntry | CalendarUpdateEntry) {
    if (selectedEntry) {
      await calendar.entries.update(selectedEntry.id, data as CalendarUpdateEntry);
    } else {
      await calendar.entries.create(data as CalendarNewEntry);
    }
    closeForm();
    bumpReview();
    await load(windowFrom, windowTo);
  }

  async function onDeleteEntry() {
    if (!selectedEntry) return;
    await calendar.entries.delete(selectedEntry.id);
    closeForm();
    bumpReview();
    await load(windowFrom, windowTo);
  }

  async function onSaveRhythm(data: CalendarNewRhythm) {
    await calendar.rhythms.create(data);
    await load(windowFrom, windowTo);
  }

  async function onCreateContext(data: CalendarNewContext) {
    await calendar.contexts.create(data);
    await load(windowFrom, windowTo);
  }

  async function onUpdateContext(id: string, data: CalendarUpdateContext) {
    await calendar.contexts.update(id, data);
    await load(windowFrom, windowTo);
  }

  async function onDeleteContext(id: string) {
    await calendar.contexts.delete(id);
    await load(windowFrom, windowTo);
  }

  /** Any adoption, removal, import or sync in the rail: refresh the pending lists and
   * reload the grid, because an adopted draft becomes a visible entry. */
  async function onReviewChanged() {
    bumpReview();
    await load(windowFrom, windowTo);
  }

  async function onGoogleExportChange(entry: CalendarEntry, optedIn: boolean) {
    if (optedIn) {
      const optIn = await calendar.google.optInExport(entry.id);
      googleExports = new Map(googleExports).set(entry.id, optIn);
    } else {
      await calendar.google.optOutExport(entry.id);
      const next = new Map(googleExports);
      next.delete(entry.id);
      googleExports = next;
    }
  }
</script>

<PageHeader
  badge="Calendar"
  title={rangeLabel}
  desc="Your time windows, rhythms, and events — open until you add something."
/>

<div class="workspace">
  <div class="main">
    <div class="toolbar">
      <div class="nav">
        <button class="btn btn-outline" aria-label="Previous" onclick={() => shift(-1)}>‹</button>
        <button class="btn btn-primary" onclick={() => (anchor = new Date())}>Today</button>
        <button class="btn btn-outline" aria-label="Next" onclick={() => shift(1)}>›</button>
      </div>
      <div class="segmented">
        {#each VIEWS as option (option.value)}
          <button class:active={view === option.value} onclick={() => (view = option.value)}>
            {option.label}
          </button>
        {/each}
      </div>
    </div>

    {#if loading}
      <p class="loading">Loading calendar…</p>
    {:else if error}
      <p class="error">{error}</p>
    {:else if view === "month"}
      <MonthGrid
        {days}
        trips={tripPlans}
        {onSelectDay}
        {onSelectEntry}
        {onSelectRange}
        {onAddEntry}
        {onCycleCommitment}
        {freeDays}
      />
    {:else}
      <TimeGrid
        {days}
        detail={view === "day"}
        onSelectDate={focusDate}
        onSelectAllDay={(date) => onSelectDay(dayFor(date))}
        {onSelectSlot}
        {onSelectEntry}
        {onAddEntry}
      />
    {/if}
  </div>

  <CalendarRail
    {refreshes}
    {onReviewChanged}
    {contexts}
    {rhythms}
    {rangeLabel}
    defaultFrom={windowFrom}
    defaultUntil={dates[dates.length - 1]}
    contextOpenId={page.url.searchParams.get("context")}
    {onCreateContext}
    {onUpdateContext}
    {onDeleteContext}
    {onSaveRhythm}
  />
</div>

<!-- Entry form overlay (create & edit) -->
{#if showEntryForm}
  <EntryForm
    entry={selectedEntry ?? undefined}
    draft={selectedDraft ?? undefined}
    eyebrow={selectedDraft ? "Time window" : undefined}
    date={selectedDay?.date ?? dayKey(new Date())}
    rangeEndDate={selectedEndDate ?? undefined}
    googleExport={selectedEntry ? googleExports.get(selectedEntry.id) : undefined}
    onSave={onSaveEntry}
    onGoogleExportChange={onGoogleExportChange}
    onDelete={selectedEntry ? onDeleteEntry : undefined}
    onClose={closeForm}
  />
{/if}

<style>
  /* The grid is the page; everything else is a rail beside it. `minmax(0, 1fr)` so a
   * wide month grid shrinks instead of pushing the rail off-screen. */
  .workspace {
    --rail-width: 17rem;

    display: grid;
    grid-template-columns: minmax(0, 1fr) var(--rail-width);
    align-items: start;
    gap: 1.25rem;

    /* Clears the grid's floating create button past the rail. */
    --grid-fab-inset: calc(var(--rail-width) + 2.5rem);
  }

  .main {
    min-width: 0;
  }

  .toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 16px;
    gap: 12px;
    flex-wrap: wrap;
  }

  .nav { display: flex; gap: 6px; align-items: center; }

  .segmented {
    display: inline-flex;
    gap: 0.125rem;
    padding: 0.125rem;
    border-radius: var(--radius-md);
    background-color: var(--surface);
  }

  .segmented button {
    font: inherit;
    font-size: var(--text-xs);
    font-weight: 500;
    padding: 0.3rem 0.6rem;
    border: 0;
    border-radius: var(--radius-sm);
    background-color: transparent;
    color: var(--text-secondary);
    cursor: pointer;
  }

  .segmented button.active {
    background-color: var(--card-bg);
    color: var(--primary);
    box-shadow: var(--card-shadow);
  }

  .loading, .error { text-align: center; padding: 48px; color: var(--text-secondary); }
  .error { color: var(--danger); }

  /* Below this the rail stops being a rail: it stacks under the grid, so the calendar
   * still comes first on a phone rather than sitting behind a column of collapsed rows. */
  @media (width < 64rem) {
    .workspace {
      grid-template-columns: minmax(0, 1fr);

      /* No rail beside the grid down here, so the button returns to the edge. */
      --grid-fab-inset: 24px;
    }
  }
</style>
