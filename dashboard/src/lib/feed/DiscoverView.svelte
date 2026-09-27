<script lang="ts">
  import { link } from "$lib/nav";
  import { onMount } from "svelte";
  import EntryForm from "$lib/calendar/EntryForm.svelte";
  import Icon from "$lib/Icon.svelte";
  import {
    axonStatus,
    calendar,
    type CalendarCandidateVerdict,
    scouting,
    type CalendarNewEntry,
    type CalendarUpdateEntry,
    type DiscoverResponse,
    type OpportunityStatus,
    type ScoutingOpportunity,
    type ScoutingSource,
  } from "$lib/api";

  type StatusFilter = OpportunityStatus | "all";

  const FILTERS: { value: StatusFilter; label: string }[] = [
    { value: "new", label: "New" },
    { value: "saved", label: "Saved" },
    { value: "dismissed", label: "Dismissed" },
    { value: "all", label: "All" },
  ];

  let sources = $state<ScoutingSource[]>([]);
  let adapter = $state("");
  let location = $state("");
  let query = $state("");
  let opportunities = $state<ScoutingOpportunity[]>([]);
  let storeTotal = $state(0);
  let statusFilter = $state<StatusFilter>("new");
  let scan = $state<DiscoverResponse | null>(null);
  let loading = $state(true);
  let scanning = $state(false);
  let busy = $state<string | null>(null);
  let error = $state<string | null>(null);
  let calendarDraft = $state<CalendarNewEntry | null>(null);
  let calendarCandidate = $state<ScoutingOpportunity | null>(null);
  let calendarMessage = $state<string | null>(null);
  let promoted = $state<Set<string>>(new Set());
  let calendarVerdicts = $state<Map<string, CalendarCandidateVerdict>>(new Map());
  let verdictRequest = 0;

  const visible = $derived(
    statusFilter === "all"
      ? opportunities
      : opportunities.filter((opportunity) => opportunity.status === statusFilter),
  );

  function count(status: StatusFilter): number {
    return status === "all"
      ? opportunities.length
      : opportunities.filter((opportunity) => opportunity.status === status).length;
  }

  function sourceLabel(source: ScoutingSource): string {
    return source.id
      .split(/[-_]/)
      .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
      .join(" ");
  }

  function typeLabel(type: string): string {
    const labels: Record<string, string> = {
      event: "Event",
      hackathon: "Hackathon",
      scholarship: "Scholarship",
      conference: "Conference",
      meetup: "Meetup",
      cfp: "Call for Papers",
    };
    return labels[type.toLowerCase()] ?? type;
  }

  function dateLabel(value: string): string {
    if (!value) return "";
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) return value;
    return date.toLocaleDateString("en-GB", {
      day: "numeric",
      month: "short",
      year: "numeric",
    });
  }

  function rationaleText(value: string): string {
    return value
      .split("\n")
      .filter((line) => !line.includes("vault link:"))
      .join(" ")
      .trim();
  }

  function shiftDate(value: string, days: number): string {
    const shifted = new Date(`${value}T12:00:00`);
    shifted.setDate(shifted.getDate() + days);
    return [
      shifted.getFullYear(),
      String(shifted.getMonth() + 1).padStart(2, "0"),
      String(shifted.getDate()).padStart(2, "0"),
    ].join("-");
  }

  function normalizedInstant(
    value: string | null | undefined,
  ): { value: string; dateOnly: boolean } | null {
    if (!value) return null;
    const match = value
      .trim()
      .match(/^(\d{4}-\d{2}-\d{2})(?:[T ](\d{2}):(\d{2}))?/);
    if (!match) return null;
    if (!match[2] || !match[3]) return { value: match[1], dateOnly: true };
    return { value: `${match[1]}T${match[2]}:${match[3]}:00`, dateOnly: false };
  }

  function oneHourAfter(value: string): string {
    const shifted = new Date(value);
    shifted.setMinutes(shifted.getMinutes() + 60);
    return [
      shifted.getFullYear(),
      String(shifted.getMonth() + 1).padStart(2, "0"),
      String(shifted.getDate()).padStart(2, "0"),
    ].join("-") + `T${String(shifted.getHours()).padStart(2, "0")}:${String(shifted.getMinutes()).padStart(2, "0")}:00`;
  }

  function meaningfulLocation(value: string | null | undefined): string | null {
    const trimmed = value?.trim() ?? "";
    return /[\p{L}\p{N}]/u.test(trimmed) ? trimmed : null;
  }

  function isCalendarCandidate(opportunity: ScoutingOpportunity): boolean {
    return ["local", "online"].includes(opportunity.event_route?.route ?? "")
      && normalizedInstant(opportunity.starts_at) !== null;
  }

  function routeLabel(opportunity: ScoutingOpportunity): string | null {
    switch (opportunity.event_route?.route) {
      case "local": return "Local";
      case "travel_candidate": return "Travel candidate";
      case "online": return "Online";
      case "unresolved": return "Route unresolved";
      default: return null;
    }
  }

  function draftFor(opportunity: ScoutingOpportunity): CalendarNewEntry | null {
    const start = normalizedInstant(opportunity.starts_at);
    if (!start) return null;
    const sourceEnd = normalizedInstant(opportunity.ends_at);

    let endsAt: string;
    if (start.dateOnly) {
      endsAt = sourceEnd?.dateOnly
        ? shiftDate(sourceEnd.value, 1)
        : sourceEnd?.value.slice(0, 10) ?? shiftDate(start.value, 1);
      if (endsAt <= start.value) endsAt = shiftDate(start.value, 1);
    } else {
      endsAt = sourceEnd && !sourceEnd.dateOnly && sourceEnd.value > start.value
        ? sourceEnd.value
        : oneHourAfter(start.value);
    }

    return {
      kind: "event",
      title: opportunity.title,
      starts_at: start.value,
      ends_at: endsAt,
      all_day: start.dateOnly,
      location:
        meaningfulLocation(opportunity.location) ??
        meaningfulLocation(opportunity.city),
      notes: null,
      source: "scouting",
      external_id: opportunity.id,
      payload: {
        producer: "scouting",
        opportunity_id: opportunity.id,
        opportunity_type: opportunity.opportunity_type,
        source: opportunity.source,
        url: opportunity.url,
        event_route: opportunity.event_route,
      },
    };
  }

  function openCalendar(opportunity: ScoutingOpportunity): void {
    const draft = draftFor(opportunity);
    if (!draft) {
      error = "This opportunity has no usable date.";
      return;
    }
    calendarCandidate = opportunity;
    calendarDraft = draft;
    calendarMessage = null;
  }

  function verdictFor(opportunity: ScoutingOpportunity): CalendarCandidateVerdict | null {
    return calendarVerdicts.get(opportunity.id) ?? null;
  }

  function verdictLabel(verdict: CalendarCandidateVerdict): string {
    if (verdict.already_in_calendar) return "Already in calendar";
    switch (verdict.verdict) {
      case "free": return "Fits the calendar";
      case "needs-travel-day": return "Needs coordination";
      case "conflicts": return "Calendar conflict";
    }
  }

  function verdictExplanation(verdict: CalendarCandidateVerdict): string {
    if (verdict.already_in_calendar) {
      return "This opportunity already has its own Sjel calendar entry.";
    }
    const strongest = verdict.evidence.find((entry) => entry.impact === verdict.verdict);
    if (!strongest) return verdict.verdict === "free" ? "No blocking overlap." : "Check the calendar.";
    if (verdict.verdict === "conflicts") return `Conflicts with “${strongest.title}”.`;
    if (verdict.verdict === "needs-travel-day") return `“${strongest.title}” requires a travel or remote day.`;
    return `“${strongest.title}” does not block this time.`;
  }

  async function loadVerdicts(next: ScoutingOpportunity[]): Promise<void> {
    const candidates = next
      .filter(isCalendarCandidate)
      .map((opportunity) => ({
        id: opportunity.id,
        starts_at: opportunity.starts_at,
        ends_at: opportunity.ends_at || null,
      }));
    const token = ++verdictRequest;
    if (candidates.length === 0) {
      calendarVerdicts = new Map();
      return;
    }
    try {
      await axonStatus.start("calendar");
      const response = await calendar.verdicts(candidates);
      if (token !== verdictRequest) return;
      calendarVerdicts = new Map(response.verdicts.map((verdict) => [verdict.id, verdict]));
    } catch {
      // Calendar context must illuminate discovery when available, never make
      // the scouting inbox unusable when the local service is restarting.
      if (token === verdictRequest) calendarVerdicts = new Map();
    }
  }

  async function promoteToCalendar(
    data: CalendarNewEntry | CalendarUpdateEntry,
  ): Promise<void> {
    if (!calendarCandidate || busy) return;
    const candidate = calendarCandidate;
    busy = candidate.id;
    error = null;
    try {
      await axonStatus.start("calendar");
      await calendar.entries.upsertExternal(data as CalendarNewEntry);
      if (candidate.status !== "saved") {
        await scouting.setStatus(candidate.id, "saved");
        opportunities = opportunities.map((opportunity) =>
          opportunity.id === candidate.id ? { ...opportunity, status: "saved" } : opportunity,
        );
      }
      promoted = new Set([...promoted, candidate.id]);
      calendarMessage = `“${candidate.title}” was added to the calendar.`;
      calendarCandidate = null;
      calendarDraft = null;
      void loadVerdicts(opportunities);
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
      throw cause;
    } finally {
      busy = null;
    }
  }

  async function loadOpportunities(): Promise<void> {
    const response = await scouting.opportunities(true);
    opportunities = response.opportunities;
    storeTotal = response.store_total;
    void loadVerdicts(response.opportunities);
  }

  async function initialise(): Promise<void> {
    loading = true;
    error = null;
    try {
      await axonStatus.start("scouting").catch(() => undefined);
      const [sourceResponse] = await Promise.all([
        scouting.sources(),
        loadOpportunities(),
      ]);
      sources = sourceResponse.sources.filter((source) => source.enabled);
      if (!sources.some((source) => source.id === adapter)) {
        adapter = sources[0]?.id ?? "";
      }
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    void initialise();
  });

  async function run(): Promise<void> {
    if (!adapter || scanning) return;
    scanning = true;
    error = null;
    try {
      scan = await scouting.discover({
        adapter,
        location: location.trim() || undefined,
        query: query.trim() || undefined,
      });
      await loadOpportunities();
      statusFilter = "new";
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      scanning = false;
    }
  }

  async function setStatus(id: string, status: OpportunityStatus): Promise<void> {
    const previous = opportunities.find((opportunity) => opportunity.id === id)?.status;
    if (!previous || busy) return;
    busy = id;
    error = null;
    opportunities = opportunities.map((opportunity) =>
      opportunity.id === id ? { ...opportunity, status } : opportunity,
    );
    try {
      await scouting.setStatus(id, status);
    } catch (cause) {
      opportunities = opportunities.map((opportunity) =>
        opportunity.id === id ? { ...opportunity, status: previous } : opportunity,
      );
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busy = null;
    }
  }
</script>

<form
  class="card controls"
  onsubmit={(event) => {
    event.preventDefault();
    void run();
  }}
>
  <label class="source">
    <span>Source</span>
    <select class="input" bind:value={adapter} disabled={loading || sources.length === 0}>
      {#each sources as source (source.id)}
        <option value={source.id}>
          {sourceLabel(source)} · {typeLabel(source.opportunity_type)}
        </option>
      {/each}
    </select>
  </label>
  <label>
    <span>Location, optional</span>
    <input class="input" bind:value={location} placeholder="Berlin" />
  </label>
  <label>
    <span>Search term, optional</span>
    <input class="input" bind:value={query} placeholder="AI, Security, Mobility …" />
  </label>
  <button class="btn btn-primary" type="submit" disabled={scanning || !adapter}>
    {#if scanning}
      <Icon name="loader" size={14} /> scanning…
    {:else}
      <Icon name="refresh" size={14} /> Scan source
    {/if}
  </button>
</form>

{#if error}
  <p class="notice">
    <Icon name="wifi-off" />
    <span>{error} <a href={link("/capabilities")}>Check Capabilities</a></span>
  </p>
{/if}

{#if calendarMessage}
  <p class="calendar-message" aria-live="polite">
    <Icon name="calendar" size={13} /> {calendarMessage}
  </p>
{/if}

{#if scan}
  <p class="scan-result mono">
    Last scan: {scan.total_scored} scored · {scan.new_count} new · {scan.vault_links} Obsidian links
  </p>
{/if}

<div class="inbox-bar">
  <div class="segmented" aria-label="Status filter">
    {#each FILTERS as filter (filter.value)}
      <button
        type="button"
        class:active={statusFilter === filter.value}
        onclick={() => (statusFilter = filter.value)}
      >
        {filter.label} <span class="mono">{count(filter.value)}</span>
      </button>
    {/each}
  </div>
  <span class="store-count mono">
    {opportunities.length === storeTotal
      ? `${storeTotal} in the opportunity store`
      : `${opportunities.length} of ${storeTotal} loaded`}
  </span>
</div>

{#if loading}
  <p class="empty"><Icon name="loader" size={14} /> Loading opportunity store…</p>
{:else if sources.length === 0}
  <p class="empty">No active Scouting source is configured.</p>
{:else if visible.length === 0}
  <p class="empty">
    {#if statusFilter === "new"}
      No new opportunities. Scan a source or open the saved entries.
    {:else}
      No entries with this status.
    {/if}
  </p>
{:else}
  <ul class="opportunities">
    {#each visible as opportunity (opportunity.id)}
      {@const verdict = verdictFor(opportunity)}
      <li class="card item" class:dismissed={opportunity.status === "dismissed"}>
        <div class="head">
          <div class="identity">
            <div class="tags">
              <span class="tag mono">{typeLabel(opportunity.opportunity_type)}</span>
              {#if routeLabel(opportunity)}
                <span
                  class="route {opportunity.event_route?.route} mono"
                  title={opportunity.event_route?.reason}
                >
                  {routeLabel(opportunity)}
                </span>
              {/if}
              <span class="source-name mono">{opportunity.source}</span>
            </div>
            <a href={opportunity.url} target="_blank" rel="noreferrer">{opportunity.title}</a>
          </div>
          <span
            class="score mono"
            class:positive={opportunity.score >= 0.35}
            title="Uncalibrated similarity to the interest profile"
          >
            Match {Math.round(opportunity.score * 100)}
          </span>
        </div>

        {#if rationaleText(opportunity.rationale)}
          <p class="why">{rationaleText(opportunity.rationale)}</p>
        {/if}

        <div class="foot">
          <p class="meta mono">
            {[dateLabel(opportunity.starts_at), opportunity.city, opportunity.matched_focus]
              .filter(Boolean)
              .join(" · ")}
          </p>
          {#if verdict}
            <span
              class="availability {verdict.verdict}"
              class:adopted={verdict.already_in_calendar}
              title={verdictExplanation(verdict)}
            >
              {verdictLabel(verdict)}
            </span>
          {/if}
          <div class="actions">
            {#if opportunity.vault_link}
              <span class="vault" title={opportunity.vault_link}>
                <Icon name="database" size={12} /> Obsidian
              </span>
            {/if}
            <a class="btn icon-btn" href={opportunity.url} target="_blank" rel="noreferrer" aria-label="Open original">
              <Icon name="external" size={13} />
            </a>
            {#if isCalendarCandidate(opportunity)}
              <button
                class="btn calendar-btn"
                class:saved={promoted.has(opportunity.id)}
                disabled={busy === opportunity.id || promoted.has(opportunity.id)}
                onclick={() => openCalendar(opportunity)}
                aria-label={promoted.has(opportunity.id) ? "In calendar" : "Add to calendar"}
              >
                <Icon name="calendar" size={13} />
                {promoted.has(opportunity.id) ? "In calendar" : "Calendar"}
              </button>
            {/if}
            <button
              class="btn icon-btn"
              class:saved={opportunity.status === "saved"}
              disabled={busy === opportunity.id}
              onclick={() => setStatus(opportunity.id, opportunity.status === "saved" ? "new" : "saved")}
              aria-label={opportunity.status === "saved" ? "Mark as new" : "Save"}
            >
              {#if busy === opportunity.id}
                <Icon name="loader" size={13} />
              {:else}
                <Icon name="check" size={13} />
              {/if}
            </button>
            <button
              class="btn icon-btn"
              disabled={busy === opportunity.id}
              onclick={() => setStatus(opportunity.id, opportunity.status === "dismissed" ? "new" : "dismissed")}
              aria-label={opportunity.status === "dismissed" ? "Mark as new" : "Dismiss"}
            >
              <Icon name={opportunity.status === "dismissed" ? "refresh" : "close"} size={13} />
            </button>
          </div>
        </div>
      </li>
    {/each}
  </ul>
{/if}

{#if calendarDraft && calendarCandidate}
  <EntryForm
    draft={calendarDraft}
    notice={calendarCandidate.ends_at
      ? "These details come from Discover. Check the time and location before adding the entry."
      : "The source provides no end time. The draft therefore uses one hour or one day initially; check it before adding the entry."}
    onSave={promoteToCalendar}
    onClose={() => {
      calendarDraft = null;
      calendarCandidate = null;
    }}
  />
{/if}

<style>
  .controls {
    display: grid;
    gap: 0.75rem;
    padding: 1rem;
    margin-bottom: 0.75rem;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    font-size: var(--text-2xs);
    font-weight: 600;
    color: var(--text-tertiary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .notice,
  .empty {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.75rem;
    border-radius: var(--radius-md);
    font-size: var(--text-sm);
  }

  .notice {
    background-color: var(--warning-soft);
    color: var(--warning-ink);
  }

  .calendar-message {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    margin: 0.75rem 0;
    color: var(--success);
    font-size: var(--text-xs);
  }

  .notice a {
    color: inherit;
    text-decoration: underline;
  }

  .scan-result {
    margin: 0.75rem 0;
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
  }

  .inbox-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    margin: 1rem 0 0.75rem;
  }

  .segmented {
    display: inline-flex;
    flex-wrap: wrap;
    gap: 0.125rem;
    padding: 0.125rem;
    border-radius: var(--radius-md);
    background-color: var(--surface);
  }

  .segmented button {
    display: inline-flex;
    gap: 0.35rem;
    border: 0;
    border-radius: var(--radius-sm);
    padding: 0.3rem 0.6rem;
    background: transparent;
    color: var(--text-secondary);
    font: inherit;
    font-size: var(--text-xs);
    font-weight: 500;
    cursor: pointer;
  }

  .segmented button.active {
    background-color: var(--card-bg);
    color: var(--primary);
    box-shadow: var(--card-shadow);
  }

  .segmented .mono {
    color: var(--text-tertiary);
  }

  .store-count {
    font-size: 0.625rem;
    color: var(--text-tertiary);
  }

  .empty {
    justify-content: center;
    min-height: 6rem;
    color: var(--text-tertiary);
  }

  .opportunities {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .item {
    padding: 0.85rem;
  }

  .item.dismissed {
    opacity: 0.55;
  }

  .head,
  .foot {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 0.75rem;
  }

  .identity {
    min-width: 0;
  }

  .tags {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    margin-bottom: 0.35rem;
  }

  .source-name {
    overflow: hidden;
    color: var(--text-tertiary);
    font-size: 0.5625rem;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .route {
    flex-shrink: 0;
    border-radius: 999px;
    padding: 0.12rem 0.32rem;
    background-color: var(--surface);
    color: var(--text-tertiary);
    font-size: 0.5625rem;
  }

  .route.local,
  .route.online {
    background-color: var(--success-soft);
    color: var(--success);
  }

  .route.travel_candidate {
    background-color: var(--warning-soft);
    color: var(--warning-ink);
  }

  .route.unresolved {
    background-color: var(--danger-soft);
    color: var(--danger);
  }

  .identity > a {
    font-size: 0.875rem;
    font-weight: 550;
  }

  .identity > a:hover {
    color: var(--primary);
  }

  .score {
    min-width: 3.5rem;
    border-radius: var(--radius-sm);
    padding: 0.2rem 0.35rem;
    background-color: var(--surface);
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
    text-align: center;
  }

  .score.positive {
    background-color: var(--success-soft);
    color: var(--success);
  }

  .why {
    margin: 0.45rem 0;
    color: var(--text-secondary);
    font-size: var(--text-sm);
  }

  .foot {
    align-items: center;
  }

  .meta {
    min-width: 0;
    margin: 0;
    color: var(--text-tertiary);
    font-size: 0.625rem;
  }

  .availability {
    flex-shrink: 0;
    border-radius: 999px;
    padding: 0.2rem 0.38rem;
    background-color: var(--surface);
    color: var(--text-secondary);
    font-size: 0.625rem;
    white-space: nowrap;
  }

  .availability.free { color: var(--success); background-color: var(--success-soft); }
  .availability.needs-travel-day { color: var(--warning-ink); background-color: var(--warning-soft); }
  .availability.conflicts { color: var(--danger); background-color: var(--danger-soft); }
  .availability.adopted { color: var(--primary); background-color: var(--primary-soft); }

  .actions {
    display: flex;
    align-items: center;
    gap: 0.15rem;
    flex-shrink: 0;
  }

  .icon-btn {
    padding: 0.3rem;
  }

  .icon-btn.saved {
    color: var(--success);
  }

  .calendar-btn {
    gap: 0.3rem;
    padding-inline: 0.55rem;
    font-size: var(--text-2xs);
  }

  .calendar-btn.saved {
    color: var(--success);
  }

  .vault {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    margin-right: 0.25rem;
    color: var(--text-tertiary);
    font-size: 0.625rem;
  }

  @media (width >= 48rem) {
    .controls {
      grid-template-columns: minmax(12rem, 1.2fr) 1fr 1fr auto;
      align-items: end;
    }
  }
</style>
