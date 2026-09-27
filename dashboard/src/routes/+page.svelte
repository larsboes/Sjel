<script lang="ts">
  import { link } from "$lib/nav";
  import { goto } from "$app/navigation";
  import { onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import { createBandDisclosure } from "$lib/home/band-disclosure.svelte";
  import StateLine from "$lib/StateLine.svelte";
  import {
    axonStatus,
    macmon,
    panelUrl,
    type AxonStatusHealth,
    type CalendarContext,
    type CalendarEntry,
    type CapabilityView,
    type FeedEntry,
    type MacmonSample,
    type ScoutingOpportunity,
    type TripPlan,
  } from "$lib/api";
  import { capabilities } from "$lib/capabilities.svelte";
  import { createListCursor } from "$lib/list-cursor.svelte";
  import RailSection from "$lib/rail/RailSection.svelte";
  import PinnedLinks from "$lib/PinnedLinks.svelte";
  import RepoStatusCard from "$lib/RepoStatusCard.svelte";
  import HomeHorizon from "$lib/home/HomeHorizon.svelte";
  import AxonGlance from "$lib/home/AxonGlance.svelte";
  import LocationView from "$lib/home/LocationView.svelte";
  import SourcesView from "$lib/home/SourcesView.svelte";
  import {
    KINDS,
    createStarter,
    decisionsFrom,
    rowComponent,
    runKinds,
    type KindState,
  } from "$lib/home/registry";
  import {
    bandLabel,
    bandTone,
    compareDecisions,
    type ActOptions,
    type Decision,
    type ScoreContext,
  } from "$lib/home/decisions";
  import type { CalendarSource } from "$lib/home/kinds/calendar";
  import type { OpportunitySource } from "$lib/home/kinds/opportunity";
  import { countLabel, daysUntil, localDateKey, sentenceCase } from "$lib/home/format";

  type HomeView = "now" | "locations" | "sources";

  /// Only `demo` is read. The root layout's load puts it on every page's data, and it is
  /// what keeps the per-kind autostart from posting a start route a demo build does not
  /// serve — the same rule +layout.svelte states for its own.
  let { data } = $props();

  /// The day, recomputed at local midnight. The old page read `new Date()` once at module
  /// scope, so a tab left open overnight ranked every date one day too urgent and kept
  /// yesterday's heading until it was reloaded.
  let today = $state(new Date());
  const todayKey = $derived(localDateKey(today));
  const horizonEndKey = $derived(
    localDateKey(new Date(today.getFullYear(), today.getMonth() + 4, today.getDate())),
  );
  const todayLabel = $derived(
    today.toLocaleDateString("en-GB", { weekday: "long", day: "numeric", month: "long" }),
  );

  /// Per kind, not per page. This is the whole of the "Home renders nothing until every
  /// read settles" fix: one `await Promise.allSettled` over seven reads held `loading`
  /// true until the slowest capability answered, and the baseline capture at four seconds
  /// shows the header and nothing else.
  let sources = $state<Record<string, KindState>>({});
  let dismissed = $state<ReadonlySet<string>>(new Set());
  let macmonSample = $state<MacmonSample | null>(null);
  let macmonErr = $state(false);
  let busy = $state<string | null>(null);
  let busyProject = $state<string | null>(null);
  let actionError = $state<string | null>(null);
  let showAll = $state(false);
  let showReading = $state(false);
  let homeView = $state<HomeView>("now");
  let reloadToken = $state(0);

  /// How much reading is worth showing before it becomes a scroll. Past this the Feed page
  /// is the better surface, so the list links out instead.
  const READING_PREVIEW = 6;

  /* Titles only. Each view used to carry a kicker above its heading — "Focus" over "Up
   * next", "Places" over "By location" — and the kicker never said anything the heading
   * and the tab strip beside it did not already say. A label above a label is chrome. */
  const viewHeadings: Record<HomeView, { title: string }> = {
    now: { title: "Up next" },
    locations: { title: "By location" },
    sources: { title: "Sources" },
  };

  const scoreContext = $derived<ScoreContext>({
    todayKey,
    horizonEndKey,
    nowMs: today.getTime(),
    daysUntil: (value: string) => daysUntil(value, today),
    peer: <Row,>(key: string) => (sources[key]?.rows ?? []) as readonly Row[],
    peerSource: <Source,>(key: string) => (sources[key]?.source as Source | undefined) ?? null,
  });

  const settledCount = $derived(Object.keys(sources).length);
  const loading = $derived(settledCount < KINDS.length);

  /// The views below LocationView and SourcesView read a kind's whole source, not its
  /// gated rows — they are different readings of the same column, not the ladder.
  const calendarSource = $derived((sources.calendar?.source as CalendarSource | undefined) ?? null);
  const calendarEntries = $derived<CalendarEntry[]>(calendarSource?.entries ?? []);
  const calendarContexts = $derived<CalendarContext[]>(calendarSource?.contexts ?? []);
  const scoutingSource = $derived(
    (sources.opportunity?.source as OpportunitySource | undefined) ?? null,
  );
  const opportunities = $derived<ScoutingOpportunity[]>(scoutingSource?.opportunities ?? []);
  const scoutingSources = $derived(scoutingSource?.sources ?? []);
  const plans = $derived((sources.trip?.source as TripPlan[] | undefined) ?? []);
  const feedEntries = $derived((sources.feed?.source as FeedEntry[] | undefined) ?? []);
  const health = $derived((sources.system?.source as AxonStatusHealth | undefined) ?? null);

  const upcomingEntries = $derived(
    calendarEntries
      .filter((entry) => entry.starts_at.slice(0, 10) >= todayKey && entry.commitment !== "possible")
      .sort((a, b) => a.starts_at.localeCompare(b.starts_at)),
  );

  const decisions = $derived(
    decisionsFrom(sources, scoreContext, dismissed).sort(compareDecisions),
  );

  /// Two lists, not one ranked pile. A trip stage and a calendar proposal are dated
  /// commitments — they expire whether or not you look at them. Feed is optional reading
  /// that never expires. Interleaving them by score put 53 articles between three
  /// decisions that actually needed a call.
  const commitments = $derived(decisions.filter((d) => (d.kind.lane ?? "commitment") === "commitment"));
  const reading = $derived(decisions.filter((d) => d.kind.lane === "reading"));

  type PriorityLens = "all" | "focus" | "schedule" | "people" | "tasks";
  let priorityLens = $state<PriorityLens>("all");

  const filteredCommitments = $derived.by(() => {
    if (priorityLens === "all") return commitments;
    if (priorityLens === "focus") {
      return commitments.filter((d) => {
        if (d.kind.band >= 800) return true;
        if (d.startOrDueAt) {
          const days = daysUntil(d.startOrDueAt, today);
          if (days <= 2) return true;
        }
        if (d.kind.key === "people") return true;
        return false;
      });
    }
    if (priorityLens === "people") return commitments.filter((d) => d.kind.key === "people");
    if (priorityLens === "schedule") return commitments.filter((d) => d.kind.key === "calendar" || d.kind.key === "trip");
    if (priorityLens === "tasks") return commitments.filter((d) => d.kind.key === "task" || d.kind.key === "finance");
    return commitments;
  });

  const visibleReading = $derived(showAll ? reading : reading.slice(0, READING_PREVIEW));

  /// The ladder, grouped by the band each row already carries. Order is the sort
  /// order — `commitments` is ranked, so the first group is the most urgent band
  /// that has anything in it, and that is the one that opens on a first visit.
  const bands = $derived.by(() => {
    const groups: { label: string; tone: string; rows: Decision[] }[] = [];
    for (const decision of filteredCommitments) {
      const label = bandLabel(decision.kind.band);
      const last = groups[groups.length - 1];
      if (last?.label === label) last.rows.push(decision);
      else groups.push({ label, tone: bandTone(decision.kind.band), rows: [decision] });
    }
    return groups;
  });

  const leadingBand = $derived(bands[0]?.label ?? "");
  const disclosure = createBandDisclosure();
  const openBands = $derived(
    bands.filter((band) => disclosure.isOpen(band.label, leadingBand)),
  );

  /// Only rows the reader can actually see. J/K must not walk into a collapsed
  /// band and move focus to something that is not on screen — the cursor sets real
  /// DOM focus, so an off-screen target scrolls the page to nothing.
  const visibleDecisions = $derived<Decision[]>([
    ...openBands.flatMap((band) => band.rows),
    ...(showReading ? visibleReading : []),
  ]);

  const readingToday = $derived(
    reading.filter((d) => today.getTime() - new Date(d.startOrDueAt ?? 0).getTime() < 86_400_000)
      .length,
  );

  const unavailable = $derived(
    KINDS.filter((kind) => sources[kind.key]?.status === "failed").map((kind) => kind.label),
  );

  /// What the page is for, in one sentence: the nearest thing that expires. A count of the
  /// backlog ("57 open items") reads as debt and names nothing you can act on — and 53 of
  /// those 57 were unread articles.
  const brief = $derived.by(() => {
    if (loading && commitments.length === 0) {
      return "Bringing together saved information, opportunities, and travel plans.";
    }
    const next = commitments[0];
    if (!next) {
      return reading.length === 0
        ? "There are no open decisions right now. You can start something new."
        : `Nothing is waiting on a decision. ${countLabel(reading.length, "unread item")} below.`;
    }
    const title = next.kind.title?.(next.row);
    const why = next.kind.whyHere(next.row, scoreContext);
    if (!title) return sentenceCase(why || next.kind.label);
    return why ? `${title}: ${sentenceCase(why)}` : `${title} is waiting for a call.`;
  });

  const rowId = (decision: Decision) => `decision-${decision.key.replace(/[^\w-]/g, "-")}`;

  const kindOf = (key: string) => KINDS.find((kind) => kind.key === key);

  const cursor = createListCursor({
    count: () => visibleDecisions.length,
    elFor: (index) => {
      const decision = visibleDecisions[index];
      return decision ? document.getElementById(rowId(decision)) : null;
    },
    onOpen: (index) => openDecision(visibleDecisions[index]),
  });

  onMount(() => {
    const stop = capabilities.subscribe();

    // One-shot macmon sample for the sidebar compact card; no aggressive polling since
    // /systems is the real live dashboard.
    const pollMac = () => {
      macmon.json().then((d) => { macmonSample = d; macmonErr = false; }).catch(() => { macmonErr = true; });
    };
    pollMac();
    const macTimer = setInterval(pollMac, 30_000);

    // Fires once at the next local midnight and then daily, so `todayKey` and every
    // "in N days" on the page move with the calendar rather than with a reload.
    let midnightTimer: ReturnType<typeof setTimeout>;
    const scheduleMidnight = () => {
      const next = new Date(today);
      next.setHours(24, 0, 5, 0);
      midnightTimer = setTimeout(() => {
        today = new Date();
        scheduleMidnight();
      }, Math.max(1000, next.getTime() - Date.now()));
    };
    scheduleMidnight();

    return () => {
      clearInterval(macTimer);
      clearTimeout(midnightTimer);
      stop();
    };
  });

  /// Each kind writes its own slice the moment it settles, so the host and calendar rows
  /// paint while comms is still reading. The AbortController is released on unmount and on
  /// a retry, so a slow read from the previous pass cannot write over a newer one.
  $effect(() => {
    void reloadToken;
    const controller = new AbortController();
    sources = {};

    void (async () => {
      // Local and fast, and awaited once before the fan-out because the capability list is
      // empty on a cold load — every "is it already up" test would be vacuously false.
      await capabilities.refresh();
      if (controller.signal.aborted) return;

      await runKinds({
        base: {
          todayKey,
          horizonEndKey,
          nowMs: today.getTime(),
          daysUntil: (value: string) => daysUntil(value, today),
          signal: controller.signal,
        },
        start: createStarter(Boolean(data?.demo)),
        onSettled: (key, state) => {
          if (controller.signal.aborted) return;
          sources = { ...sources, [key]: state };
        },
      });
    })();

    return () => controller.abort();
  });

  /// Awaits the capability write FIRST, and only on resolution does the key leave the
  /// ladder AND the row leave its kind's source. An optimistic dismissal would show a
  /// decision as made that the capability never recorded — the dashboard contradicting
  /// the owner of the record.
  ///
  /// Both halves are needed. `dismissed` only hides the row from the ladder, and the
  /// ladder is not the only reading of these rows: Locations lists every new opportunity,
  /// Sources counts them, and the horizon reads every dated calendar entry. Patching the
  /// source too is what the base page did by hand in each of its three action handlers.
  function act(
    kindKey: string,
    key: string,
    run: () => Promise<void>,
    options?: ActOptions<unknown>,
  ): void {
    if (busy) return;
    busy = key;
    actionError = null;
    void run()
      .then(() => {
        const patch = options?.patch;
        const state = sources[kindKey];
        if (patch && state) {
          const source = patch(state.source);
          sources = {
            ...sources,
            // `rows` is recomputed from the patched source through the kind's own gate, so
            // a row the write took out of scope leaves every view at once.
            [kindKey]: { ...state, source, rows: kindOf(kindKey)?.rows(source, scoreContext) ?? state.rows },
          };
        }
        if (options?.dismiss === false) return;
        // Reassigned, never mutated: `$state` proxies plain objects and arrays and does
        // not intercept Set methods, so `dismissed.add(key)` would leave the derived
        // ladder unrecomputed and the buttons would look inert.
        dismissed = new Set(dismissed).add(key);
      })
      .catch((caught: unknown) => {
        actionError = caught instanceof Error ? caught.message : String(caught);
      })
      .finally(() => {
        busy = null;
      });
  }

  /// The only schemes a capability-supplied destination may carry.
  ///
  /// `obsidian:` is here because a task's destination is a note, and `window.open` refuses
  /// a non-http scheme. Everything else is refused rather than assigned to `location`:
  /// `opportunity.href` is a URL harvested from a third-party feed and no scouting adapter
  /// constrains its scheme, so `location.assign("javascript:…")` would run that feed's
  /// script in the origin that renders this page's mail subjects and snippets. The base
  /// page reached these rows through `window.open`, which browsers refuse for such a URL;
  /// keeping the keyboard path narrower than the mouse path is the actual requirement.
  const OPENABLE_SCHEME = /^(https?|obsidian):/i;

  function openDecision(decision: Decision | undefined): void {
    if (!decision) return;
    const href = decision.kind.href(decision.row);
    if (decision.kind.external?.(decision.row)) {
      if (!OPENABLE_SCHEME.test(href)) {
        actionError = `${decision.kind.label} gave a destination this page will not open.`;
        return;
      }
      if (/^https?:/i.test(href)) window.open(href, "_blank", "noopener,noreferrer");
      else window.location.assign(href);
      return;
    }
    // A client navigation, not `location.href`. Five of the seven old branches assigned a
    // raw absolute path, which is issue #170 reopened through the keyboard — invisible
    // because clicking the same row's anchor works — and it tore down the SPA, stopping
    // the soundscape dock mid-track. `href` is already base-aware: every kind builds it
    // through link().
    void goto(href);
  }

  async function startProject(project: CapabilityView): Promise<void> {
    if (busyProject) return;
    busyProject = project.name;
    actionError = null;
    try {
      await axonStatus.start(project.name);
      await capabilities.refresh();
    } catch (caught) {
      actionError = caught instanceof Error ? caught.message : String(caught);
    } finally {
      busyProject = null;
    }
  }

  function projectTitle(project: CapabilityView): string {
    if (project.name === "server") return "Home-Server & Local AI";
    return project.name.charAt(0).toUpperCase() + project.name.slice(1);
  }
</script>

<!-- Armed on the Now view only. The queue is the only thing J/K/Enter address, and it
     renders under `homeView === "now"`; leaving the handler on the window meant an Enter
     pressed on Locations or Sources navigated to a commitment that was not on screen. -->
<svelte:window onkeydown={homeView === "now" ? cursor.handleKeydown : undefined} />

<div class="home">
  <header class="briefing">
    <div>
      <p class="date">{todayLabel}</p>
      <h1>
        {#if loading && commitments.length === 0}
          Sjel is organising the day.
        {:else if commitments.length === 0}
          Nothing to decide.
        {:else}
          Here is what to do next.
        {/if}
      </h1>
      <p class="brief">{brief}</p>
    </div>
    <a class="library-link" href={link("/feed/library")}>
      Library
    </a>
  </header>

  {#if actionError}
    <div class="notice error" role="alert">
      <Icon name="alert" size={15} />
      <span>{actionError}</span>
      <button type="button" aria-label="Dismiss error" onclick={() => (actionError = null)}>
        <Icon name="close" size={13} />
      </button>
    </div>
  {/if}

  <div class="workspace">
    <section class="next">
      {#if homeView === "now"}
        <AxonGlance
          entries={upcomingEntries}
          plans={plans}
          macmon={macmonSample}
        />
        <HomeHorizon contexts={calendarContexts} entries={upcomingEntries} />
      {/if}

      <!-- One header for the whole main column. The view switcher lives here rather than
           above the page, because these are three readings of the same column, not three
           modes of the page. -->
      <div class="section-head">
        <div>
          <h2>{viewHeadings[homeView].title}</h2>
        </div>
        <nav class="home-views" aria-label="Home view">
          <button class:active={homeView === "now"} onclick={() => (homeView = "now")}>Now</button>
          <button class:active={homeView === "locations"} onclick={() => (homeView = "locations")}>
            Locations
          </button>
          <button class:active={homeView === "sources"} onclick={() => (homeView = "sources")}>
            Sources
          </button>
        </nav>
      </div>

      {#if homeView === "now"}
        <div class="priority-lens-bar" role="group" aria-label="Priority focus">
          <button
            type="button"
            class="lens-pill"
            class:active={priorityLens === "all"}
            onclick={() => (priorityLens = "all")}
          >
            All Priorities <span class="pill-count">{commitments.length}</span>
          </button>
          <button
            type="button"
            class="lens-pill"
            class:active={priorityLens === "focus"}
            onclick={() => (priorityLens = "focus")}
          >
            <Icon name="sparkles" size={12} />
            Today's Focus
          </button>
          <button
            type="button"
            class="lens-pill"
            class:active={priorityLens === "schedule"}
            onclick={() => (priorityLens = "schedule")}
          >
            <Icon name="calendar" size={12} />
            Schedule & Trips
          </button>
          <button
            type="button"
            class="lens-pill"
            class:active={priorityLens === "people"}
            onclick={() => (priorityLens = "people")}
          >
            <Icon name="users" size={12} />
            People
          </button>
          <button
            type="button"
            class="lens-pill"
            class:active={priorityLens === "tasks"}
            onclick={() => (priorityLens = "tasks")}
          >
            <Icon name="check" size={12} />
            Tasks & Spend
          </button>
        </div>

        {#if filteredCommitments.length === 0 && commitments.length > 0}
          <div class="priority-empty">
            <p>No items in this priority lens.</p>
            <button class="btn btn-soft" type="button" onclick={() => (priorityLens = "all")}>
              Show all priorities ({commitments.length})
            </button>
          </div>
        {/if}

        {#if visibleDecisions.length > 1}
          <p class="key-hint"><kbd>J</kbd><kbd>K</kbd> select<span></span><kbd>Enter</kbd> open</p>
        {/if}

      <!-- role="list" and rows as listitems, not a listbox. An option must not contain
           focusable descendants and every row here holds a title link and up to three
           buttons, so the cursor moves real DOM focus onto the row instead — which is
           also what makes the selection audible to a screen reader. -->
      <!-- One band open, the rest counted. The band break used to be a decorative
           `aria-hidden` rule between rows; it is the control now, which is why it is a
           real <button> with aria-expanded rather than a <li> with a label in it. -->
      <div class="ladder" aria-busy={loading}>
        {#if loading && visibleDecisions.length === 0}
          <div class="ladder-skeletons" aria-label="Loading decisions">
            <div class="skeleton-band">
              <div class="skeleton-bar title"></div>
              <div class="skeleton-row">
                <div class="skeleton-dot"></div>
                <div class="skeleton-line-wrap">
                  <div class="skeleton-line full"></div>
                  <div class="skeleton-line half"></div>
                </div>
              </div>
              <div class="skeleton-row">
                <div class="skeleton-dot"></div>
                <div class="skeleton-line-wrap">
                  <div class="skeleton-line three-quarter"></div>
                  <div class="skeleton-line third"></div>
                </div>
              </div>
            </div>
          </div>
        {/if}

        {#each bands as band (band.label)}
          {@const open = disclosure.isOpen(band.label, leadingBand)}
          <section class="band tone-{band.tone}">
            <h3>
              <button
                type="button"
                class="band-summary"
                aria-expanded={open}
                aria-controls="band-{band.tone}"
                onclick={() => disclosure.toggle(band.label, leadingBand)}
              >
                <span class="chevron" class:open aria-hidden="true">
                  <Icon name="chevron" size={12} />
                </span>
                <span class="band-name">{band.label}</span>
                <span class="band-count">{band.rows.length}</span>
              </button>
            </h3>
            {#if open}
              <ul id="band-{band.tone}" class="queue" role="list">
                {#each band.rows as decision (decision.key)}
                  {@render decisionRow(decision)}
                {/each}
              </ul>
            {/if}
          </section>
        {/each}
      </div>

      <!-- Loading is a state, and it is the only one that renders. When every kind has
           settled and nothing is owed, the queue shows nothing at all: PRD §8.1 rules that
           a dashboard blank on a quiet day is working correctly and that a "0 items"
           placeholder destroys the signal. -->
      <StateLine
        state={loading && commitments.length === 0 ? "loading" : "ready"}
        message="Reading current work…"
      />

      <!-- No index parameter: the cursor's own position decides `current`, and the two
           call sites were computing an offset that nothing read. -->
      {#snippet decisionRow(decision: Decision)}
        {@const Row = rowComponent(decision.kind)}
        {#if Row}
          <Row
            row={decision.row}
            id={rowId(decision)}
            current={visibleDecisions[cursor.index]?.key === decision.key}
            tone={bandTone(decision.kind.band)}
            href={decision.kind.href(decision.row)}
            busy={busy === decision.key}
            whyHere={decision.kind.whyHere(decision.row, scoreContext)}
            dataClass={decision.kind.dataClass(decision.row)}
            processingRoute={decision.kind.processingRoute(decision.row)}
            candidateStatus={decision.kind.candidateStatus(decision.row)}
            act={(run: () => Promise<void>, options?: ActOptions<unknown>) =>
              act(decision.kind.key, decision.key, run, options)}
          />
        {/if}
      {/snippet}

      <!-- Reading is the other 93% of what used to be one queue, and none of it expires.
           It gets a count and a disclosure, not a rank: the lane split exists because
           interleaving 53 articles with three decisions was the original failure, and a
           visible band spine does not fix that. -->
      {#if reading.length > 0}
        <div class="reading">
          <button
            class="reading-toggle"
            type="button"
            aria-expanded={showReading}
            onclick={() => (showReading = !showReading)}
          >
            <Icon name="feed" size={13} />
            <span>
              <strong>{countLabel(reading.length, "unread item")}</strong>
              {#if readingToday > 0}<small>{readingToday} today</small>{/if}
            </span>
            <em>{showReading ? "Hide" : "Read"}</em>
          </button>

          {#if showReading}
            <ul class="queue" role="list">
              {#each visibleReading as decision (decision.key)}
                {@render decisionRow(decision)}
              {/each}
            </ul>
            {#if reading.length > READING_PREVIEW}
              <button class="show-all" type="button" onclick={() => (showAll = !showAll)}>
                {showAll
                  ? `Show ${READING_PREVIEW} at a time`
                  : `Show ${reading.length - READING_PREVIEW} more`}
                <Icon name={showAll ? "close" : "plus"} size={12} />
              </button>
            {/if}
          {/if}
        </div>
      {/if}

      {#if unavailable.length > 0}
        <p class="unavailable">
          <Icon name="wifi-off" size={12} />
          Unavailable: {unavailable.join(", ")}
          <button type="button" onclick={() => (reloadToken += 1)}>Try again</button>
        </p>
      {/if}
      {:else if homeView === "locations"}
        <LocationView
          entries={calendarEntries}
          opportunities={opportunities.filter((opportunity) => opportunity.status === "new")}
          plans={plans.filter((plan) => plan.status !== "archived")}
        />
      {:else}
        <SourcesView
          {feedEntries}
          {opportunities}
          {scoutingSources}
          {calendarEntries}
          contexts={calendarContexts}
          {plans}
        />
      {/if}
    </section>

    <aside>
      <RailSection label="Quick actions" open>
        <nav class="quick-list" aria-label="Quick actions">
          <a href={link("/feed")}>
            <Icon name="plus" size={15} />
            <span><strong>Add a link</strong><small>Article, video, or repository</small></span>
            <Icon name="arrow-right" size={13} />
          </a>
          <a href={link("/travel")}>
            <Icon name="map-pin" size={15} />
            <span><strong>Plan travel</strong><small>Places, connections, and dates</small></span>
            <Icon name="arrow-right" size={13} />
          </a>
          <a href={link("/feed?view=discover")}>
            <Icon name="compass" size={15} />
            <span><strong>Scan sources</strong><small>Look deliberately for new opportunities</small></span>
            <Icon name="arrow-right" size={13} />
          </a>
        </nav>
      </RailSection>

      {#if capabilities.panels.length > 0}
        <RailSection label="Continue working" count={capabilities.panels.length} open>
          {#snippet action()}
            <!-- Navigates rather than toggles: without this the press does both, and the
                 section the reader left open is closed behind them. -->
            <a
              class="small-link"
              href={link("/projects")}
              onclick={(event) => event.stopPropagation()}>All</a
            >
          {/snippet}
          <ul class="continue">
            {#each capabilities.panels as project (project.name)}
              <li>
                <span class="project-mark">
                  <Icon name={project.name === "server" ? "server" : "graduation"} size={15} />
                </span>
                <span class="project-copy">
                  <strong>{projectTitle(project)}</strong>
                  <small>{project.up === true ? "running" : "starts on demand"}</small>
                </span>
                {#if project.up}
                  <a
                    class="btn icon-action"
                    href={panelUrl(project)}
                    target="_blank"
                    rel="noreferrer"
                    aria-label={`Open ${projectTitle(project)}`}
                    title="Open"
                  >
                    <Icon name="external" size={13} />
                  </a>
                {:else}
                  <button
                    class="btn project-start"
                    type="button"
                    disabled={busyProject === project.name}
                    onclick={() => void startProject(project)}
                  >
                    {#if busyProject === project.name}
                      <Icon name="loader" size={13} />
                    {:else}
                      Start
                    {/if}
                  </button>
                {/if}
              </li>
            {/each}
          </ul>
        </RailSection>
      {/if}

      <PinnedLinks />

      <!-- Machine status, not work. It stays one line until asked: health,
           temperature and memory are things you check, not things you do, and
           three sections of them outweighed the queue they sat beside. A
           <details> keeps the disclosure in CSS with no state to track. -->
      <details class="status">
        <summary>
          <span class="status-dot" class:ok={health?.ok} class:problem={health !== null && !health.ok}></span>
          <span class="status-line">
            {health === null ? "Status unknown" : health.ok ? "Systems healthy" : "Needs attention"}
            {#if macmonSample}
              <span class="mono">
                · {macmonSample.temp.cpu_temp_avg.toFixed(0)}°
                · {(macmonSample.memory.ram_usage / 1073741824).toFixed(1)} GB
              </span>
            {/if}
          </span>
          <Icon name="chevron" size={12} />
        </summary>

        <div class="status-body">
          {#if macmonErr}
            <p class="mc-offline">
              <Icon name="alert" size={12} />
              macmon is off — <a href={link("/systems")}>Details</a>
            </p>
          {:else if macmonSample}
            <div class="macmon-compact">
              <div class="mc-temps">
                <span class="mc-temp" class:warm={macmonSample.temp.cpu_temp_avg >= 60} class:hot={macmonSample.temp.cpu_temp_avg >= 80}>
                  {macmonSample.temp.cpu_temp_avg.toFixed(0)}° CPU
                </span>
                <span class="mc-temp">
                  {macmonSample.temp.gpu_temp_avg.toFixed(0)}° GPU
                </span>
                <span class="mc-power">{macmonSample.all_power.toFixed(1)} W</span>
              </div>
              <div class="mc-mem">
                <span class="mc-mem-label">RAM</span>
                <div class="mc-bar">
                  <div class="mc-fill" style="width:{(macmonSample.memory.ram_usage / macmonSample.memory.ram_total * 100).toFixed(0)}%"></div>
                </div>
                <span class="mc-mem-num mono">{(macmonSample.memory.ram_usage / 1073741824).toFixed(1)} GB</span>
              </div>
              <a class="mc-detail" href={link("/systems")}>Details</a>
            </div>
          {/if}

          <RepoStatusCard />

          <a class="capabilities-link" href={link("/capabilities")}>
            Capabilities
          </a>
        </div>
      </details>
    </aside>
  </div>
</div>

<style>
  .home {
    animation: fade-up 0.2s ease-out both;
  }

  .briefing {
    display: flex;
    align-items: end;
    justify-content: space-between;
    gap: var(--space-7);
    padding: var(--space-1) 0 var(--space-6);
    border-bottom: 1px solid var(--rule);
  }

  /* Sentence case, like PageHeader's badge and the drawer's section labels. A
     tracked-out all-caps line above every heading is template chrome, and Home carried
     five of them: the date and four section kickers. */
  .date,

  h1 {
    max-width: 48rem;
    margin: 0;
    font-size: clamp(var(--text-xl), 2.6vw, var(--text-2xl));
    font-weight: 620;
    line-height: 1.08;
    letter-spacing: -0.035em;
  }

  .brief {
    max-width: var(--measure);
    margin: var(--space-2) 0 0;
    color: var(--text-secondary);
    font-size: var(--text-sm);
  }

  .library-link,
  .small-link {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    color: var(--text-secondary);
    font-size: var(--text-xs);
    font-weight: 600;
    white-space: nowrap;
  }

  .library-link:hover,
  .small-link:hover {
    color: var(--primary);
  }

  .notice {
    display: flex;
    align-items: center;
    gap: 0.55rem;
    margin-top: 1rem;
    padding: 0.7rem 0.85rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    font-size: var(--text-xs);
  }

  .notice.error {
    color: var(--danger);
    border-color: var(--danger);
    background: var(--danger-soft);
  }

  .notice span {
    flex: 1;
  }

  .notice button {
    display: grid;
    place-items: center;
    padding: 0.2rem;
    border: 0;
    background: transparent;
    color: inherit;
    cursor: pointer;
  }

  /* Three readings of one column, not three modes of the page — so text links
     that sit beside the heading, rather than a filled control above it that
     implied the page had three top-level states. */
  .home-views {
    display: inline-flex;
    gap: 0.85rem;
  }

  .home-views button {
    padding: 0 0 0.2rem;
    border: 0;
    border-bottom: 1.5px solid transparent;
    background: transparent;
    color: var(--text-tertiary);
    font: 600 0.7rem var(--font-sans);
    cursor: pointer;
  }

  .home-views button:hover {
    color: var(--text-secondary);
  }

  .home-views button.active {
    border-bottom-color: var(--primary);
    color: var(--text-primary);
  }

  .workspace {
    display: grid;
    gap: clamp(2rem, 4vw, 4rem);
    padding-top: 1.2rem;
  }

  .section-head {
    display: flex;
    align-items: end;
    justify-content: space-between;
    gap: 1rem;
    margin-bottom: 0.8rem;
  }

  .priority-lens-bar {
    display: flex;
    gap: 0.4rem;
    flex-wrap: wrap;
    align-items: center;
    margin-bottom: var(--space-3);
  }

  .lens-pill {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.25rem 0.6rem;
    border-radius: var(--radius-full);
    font-size: var(--text-xs);
    font-weight: 500;
    background: var(--card-bg);
    border: 1px solid var(--rule);
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 120ms ease;
  }

  .lens-pill:hover {
    color: var(--text-primary);
    border-color: var(--primary);
  }

  .lens-pill.active {
    background: var(--primary-soft);
    color: var(--primary);
    border-color: var(--primary);
    font-weight: 600;
  }

  .pill-count {
    padding: 0 0.35rem;
    border-radius: var(--radius-full);
    background: var(--rule-soft, rgba(125, 125, 125, 0.15));
    font-size: var(--text-2xs);
  }

  .priority-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.75rem;
    padding: 1.5rem;
    margin-bottom: 1rem;
    text-align: center;
    border: 1px dashed var(--rule);
    border-radius: var(--radius-md);
    color: var(--text-tertiary);
    font-size: var(--text-xs);
  }

  h2 {
    margin: 0;
    font-size: 1rem;
    font-weight: 650;
    letter-spacing: -0.015em;
  }

  /* Two hints, separated by space. The empty span is the gap the middle dot used to
     be — one flex child wide, nothing to read. */
  .key-hint {
    display: flex;
    align-items: center;
    margin: 0 0 var(--space-3);
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  .key-hint span {
    width: var(--space-5);
  }

  /* The reading band. Deliberately quieter than a decision row: one line with
     a count, and the articles only when asked for. */
  .reading {
    margin-top: 0.9rem;
  }

  .reading-toggle {
    display: flex;
    align-items: center;
    gap: 0.55rem;
    width: 100%;
    padding: 0.6rem 0.1rem;
    border: 0;
    border-top: 1px solid var(--card-border);
    border-bottom: 1px solid var(--card-border);
    background: transparent;
    color: var(--text-secondary);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .reading-toggle:hover {
    color: var(--text-primary);
  }

  .reading-toggle span {
    display: flex;
    flex: 1;
    align-items: baseline;
    gap: 0.45rem;
    min-width: 0;
  }

  .reading-toggle strong {
    font-size: var(--text-sm);
    font-weight: 600;
  }

  .reading-toggle small {
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  .reading-toggle em {
    color: var(--primary);
    font-size: var(--text-2xs);
    font-style: normal;
    font-weight: 600;
  }

  .reading .queue {
    border-top: 0;
  }

  kbd {
    min-width: 1.25rem;
    padding: 0.1rem 0.25rem;
    border: 1px solid var(--card-border);
    border-bottom-color: var(--card-border-hover);
    border-radius: 3px;
    background: var(--surface);
    color: var(--text-secondary);
    font: 600 0.5625rem var(--font-mono);
    text-align: center;
  }

  .queue {
    margin: 0;
    padding: 0;
    border-top: 1px solid var(--rule);
    border-bottom: 1px solid var(--rule);
    list-style: none;
  }

  /* On a quiet day the queue really does render nothing. Without this, an empty <ul> still
     painted its two rules as a pair of hairlines across the column — a "0 items" marker
     drawn in CSS, which is the form PRD §8.1 names as the one to avoid. */
  .queue:empty {
    border: 0;
  }

  /* A hairline break where the band changes, carrying the band's NAME.
   *
   * The spine on each row is two pixels of colour and nothing else; this is what makes it
   * mean something, and it is why no band is ever identified by colour alone. Suppressed
   * when the ladder holds a single band, where a heading over every row says nothing. */
  /* The band summary. It was a decorative rule with a label; it is the control now,
     so it has to look pressable without becoming a button-shaped object: full-bleed
     hit area, the tone mark where each row's spine already sits, and the count doing
     the work a "14 more" link would otherwise do. */
  .band-summary {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    padding: var(--space-4) var(--space-2) var(--space-3) 0;
    border: 0;
    background: transparent;
    color: var(--text-tertiary);
    font: inherit;
    font-size: var(--text-2xs);
    text-align: left;
    cursor: pointer;
  }

  .band h3 {
    margin: 0;
    font-size: inherit;
    font-weight: inherit;
  }

  .band + .band {
    border-top: 1px solid var(--rule);
  }

  .band-name {
    position: relative;
    flex: 1;
    padding-left: var(--space-4);
    color: var(--text-secondary);
  }

  /* The tone mark. Never the only channel — the name is right beside it, which is
     the rule Q87 set when four tones had to carry thirteen bands. */
  .band-name::before {
    content: "";
    position: absolute;
    inset: 0.1em auto 0.1em 0;
    width: 2px;
  }

  .band-summary:hover .band-name,
  .band-summary:hover .band-count {
    color: var(--text-primary);
  }

  .band-count {
    min-width: 1.5rem;
    padding: 0.05rem 0.4rem;
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums lining;
    text-align: center;
  }

  .chevron {
    display: flex;
    color: var(--text-tertiary);
    transition: transform var(--motion-fast) ease;
  }

  .chevron.open {
    transform: rotate(90deg);
  }

  @media (prefers-reduced-motion: reduce) {
    .chevron {
      transition: none;
    }
  }

  .band.tone-alarm .band-name::before { background: var(--band-alarm); }
  .band.tone-now .band-name::before { background: var(--band-now); }
  .band.tone-owed .band-name::before { background: var(--band-owed); }
  .band.tone-offer .band-name::before { background: var(--band-offer); }

  .show-all {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    margin: 0.65rem 0 0 auto;
    padding: 0.3rem 0;
    border: 0;
    background: transparent;
    color: var(--text-secondary);
    font: 600 0.6875rem var(--font-sans);
    cursor: pointer;
  }

  .show-all:hover {
    color: var(--primary);
  }

  .unavailable {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.35rem;
    margin: 0.8rem 0 0;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  .unavailable button {
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--primary);
    font: inherit;
    cursor: pointer;
  }

  /* The rail is the page's one floating pane, so it is the one surface here that gets
   * glass. Everything to its left is the sheet: opaque, hairline-ruled, and read rather
   * than looked at. Frosting the content too would cost legibility on the half of the
   * page that has the words in it, and buy an effect over a solid colour that has
   * nothing behind it to show through.
   *
   * Sticky is what earns it: the sheet passes underneath while this stays. */
  aside {
    display: flex;
    flex-direction: column;
    gap: var(--space-6);
  }

  /* Both halves are panes now. The reading column used to sit flat on the page while
   * the rail floated, so the page read as one finished surface beside one unfinished
   * one. The content pane does NOT scroll independently and is not sticky — it is the
   * thing being read, and a reading surface that traps its own scroll is a worse
   * reading surface. It is glass for the material, not for the behaviour. */
  @media (width >= 50rem) {
    .next {
      padding: var(--space-6) var(--space-7);
      background-color: var(--glass-bg);
      border: 1px solid var(--card-border);
      border-top-color: var(--glass-border);
      border-radius: var(--radius-xl);
      box-shadow: var(--glass-shadow);
      -webkit-backdrop-filter: var(--glass-blur);
      backdrop-filter: var(--glass-blur);
    }

    aside {
      position: sticky;
      top: calc(var(--header-stack) + var(--space-3));
      max-height: calc(100vh - var(--header-stack) - var(--space-6));
      padding: var(--space-6) var(--space-5);
      overflow-y: auto;
      background-color: var(--glass-bg);
      border: 1px solid var(--card-border);
      border-top-color: var(--glass-border);
      border-radius: var(--radius-xl);
      box-shadow: var(--glass-shadow);
      -webkit-backdrop-filter: var(--glass-blur);
      backdrop-filter: var(--glass-blur);
    }

    /* Translucency without the blur is text over text. Both spellings, because Safari
       implements the prefixed one and a condition naming only the unprefixed property
       would paint these opaque in the browser this surface is actually read in. */
    @supports not ((backdrop-filter: blur(1px)) or (-webkit-backdrop-filter: blur(1px))) {
      .next,
      aside {
        background-color: var(--card-bg);
      }
    }
  }

  .quick-list {
    border-top: 1px solid var(--card-border);
  }

  .quick-list a {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: center;
    gap: 0.7rem;
    padding: 0.8rem 0.1rem;
    border-bottom: 1px solid var(--card-border);
    color: var(--text-secondary);
  }

  .quick-list a > :global(svg):first-child {
    color: var(--primary);
  }

  .quick-list a > :global(svg):last-child {
    color: var(--text-tertiary);
  }

  .quick-list a:hover {
    color: var(--primary);
  }

  .quick-list span,
  .project-copy {
    display: grid;
    min-width: 0;
  }

  .quick-list strong,
  .project-copy strong {
    overflow: hidden;
    color: var(--text-primary);
    font-size: var(--text-xs);
    font-weight: 620;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .quick-list small,
  .project-copy small {
    overflow: hidden;
    color: var(--text-tertiary);
    font-size: 0.625rem;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  ul.continue {
    margin: 0;
    padding: 0;
    border-top: 1px solid var(--card-border);
    list-style: none;
  }

  ul.continue li {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: center;
    gap: 0.65rem;
    padding: 0.65rem 0;
    border-bottom: 1px solid var(--card-border);
  }

  .project-mark {
    display: grid;
    place-items: center;
    width: 1.8rem;
    height: 1.8rem;
    border-radius: var(--radius-sm);
    background: var(--primary-soft);
    color: var(--primary);
  }

  /* ── Compact macmon sidebar card ──────────────────────────── */
  .macmon-compact {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    border-top: 1px solid var(--card-border);
    padding: 0.6rem 0.1rem 0.2rem;
    font-size: 0.72rem;
  }

  .mc-temps {
    display: flex;
    align-items: center;
    gap: 0.65rem;
  }

  .mc-temp {
    font-variant-numeric: tabular-nums;
    font-weight: 500;
  }

  .mc-temp.warm {
    color: var(--warning-ink);
  }

  .mc-temp.hot {
    color: var(--danger);
  }

  .mc-power {
    margin-left: auto;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  .mc-mem {
    display: flex;
    align-items: center;
    gap: 0.45rem;
  }

  /* Sentence case. All-caps is the commonest label tell, and at 0.65rem it also costs
     legibility — capitals lose the ascender/descender shapes a reader scans by. */
  .mc-mem-label {
    flex-shrink: 0;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
    font-weight: 500;
  }

  .mc-bar {
    flex: 1;
    height: 0.3rem;
    border-radius: 999px;
    background: var(--surface);
    overflow: hidden;
  }

  .mc-fill {
    height: 100%;
    border-radius: 999px;
    background: var(--primary);
    transition: width 0.5s ease;
  }

  .mc-mem-num {
    flex-shrink: 0;
    color: var(--text-secondary);
    font-size: var(--text-2xs);
  }

  .mc-detail {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    color: var(--text-tertiary);
    font-size: 0.65rem;
    font-weight: 600;
    margin-top: 0.15rem;
  }

  .mc-detail:hover {
    color: var(--primary);
  }

  .mc-offline {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    margin: 0;
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
    border-top: 1px solid var(--card-border);
    padding: 0.6rem 0.1rem 0;
  }

  .mc-offline a {
    color: var(--primary);
    font-weight: 600;
  }

  .status {
    margin-top: auto;
    padding-top: 0.8rem;
    border-top: 1px solid var(--card-border);
  }

  .status summary {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
    cursor: pointer;
    list-style: none;
  }

  .status summary::-webkit-details-marker {
    display: none;
  }

  .status-line {
    flex: 1;
  }

  .status-line .mono {
    color: var(--text-tertiary);
    font-family: var(--font-mono);
    font-size: 0.625rem;
  }

  .status summary > :global(svg) {
    transition: transform 0.15s ease;
  }

  .status[open] summary > :global(svg) {
    transform: rotate(90deg);
  }

  .status-body {
    display: grid;
    gap: 0.6rem;
    padding-top: 0.6rem;
  }

  .capabilities-link {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  .status-dot {
    width: 0.4rem;
    height: 0.4rem;
    border-radius: 50%;
    background: var(--text-tertiary);
  }

  .status-dot.ok {
    background: var(--success);
  }

  .status-dot.problem {
    background: var(--warning);
  }

  /* The main lane is CAPPED, not proportional.
   *
   * It was `minmax(0, 2.2fr)` against a 2200px shell, so on this display a calendar row
   * ran about 1600px: the date at one end and the venue at the other, with a void
   * between them that the eye has to cross to pair the two. A fraction
   * of an ultrawide is not a measure. 68rem is wider than --measure because these are
   * structured rows rather than prose — a date, a title and a venue, each in its own
   * column — but it is bounded, which is the part that was missing.
   *
   * The width that stops going to the lane goes to the rail and then to the gutters. */
  @media (width >= 50rem) {
    .workspace {
      grid-template-columns: minmax(0, 2.2fr) minmax(17rem, 0.8fr);

      /* The cap goes on the GRID, not on the tracks. Capping the first track with
       * `minmax(0, 68rem)` plus `justify-content: center` sized both tracks to their
       * content instead of to the container — the columns collapsed and the page grew
       * to 15,460px. Bounding the container leaves `fr` doing what `fr` does. The wider
       * cap keeps the pane edges closer to the shell at normal desktop widths without
       * allowing ultrawide rows to become a second reading surface. */
      max-width: 110rem;
      margin-inline: auto;
      width: 100%;
    }
  }

  @media (width < 38rem) {
    .briefing {
      align-items: flex-start;
      flex-direction: column;
      gap: 0.65rem;
      padding-block: 0.15rem 1rem;
    }

    h1 {
      font-size: clamp(1.7rem, 8vw, 2.05rem);
      line-height: 1.05;
    }

    .brief {
      font-size: 0.9rem;
      line-height: 1.45;
    }

    .library-link {
      min-height: 2.5rem;
    }

    .home-views {
      display: grid;
      grid-template-columns: repeat(3, minmax(0, 1fr));
      width: 100%;
      margin-top: 0.85rem;
    }

    .home-views button {
      min-height: 2.75rem;
      padding: 0.5rem 0.35rem;
      font-size: var(--text-xs);
    }

    .workspace {
      gap: 2.75rem;
      padding-top: 1rem;
    }

    .quick-list a {
      min-height: 3.75rem;
    }

    .quick-list strong,
    .project-copy strong {
      font-size: 0.82rem;
    }

    .quick-list small,
    .project-copy small {
      font-size: 0.7rem;
    }

    .key-hint {
      display: none;
    }
  }

  .ladder-skeletons {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: var(--space-3) 0;
  }

  .skeleton-band {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .skeleton-bar.title {
    width: 120px;
    height: 18px;
    border-radius: var(--radius-sm);
    background: linear-gradient(90deg, var(--surface) 25%, color-mix(in srgb, var(--card-border) 40%, var(--surface)) 50%, var(--surface) 75%);
    background-size: 200% 100%;
    animation: shimmer 1.5s infinite;
  }

  .skeleton-row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-md);
    background-color: var(--surface);
  }

  .skeleton-dot {
    width: 24px;
    height: 24px;
    border-radius: var(--radius-sm);
    background-color: var(--card-border);
    flex-shrink: 0;
  }

  .skeleton-line-wrap {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    flex: 1;
  }

  .skeleton-line {
    height: 12px;
    border-radius: var(--radius-sm);
    background: linear-gradient(90deg, var(--card-border) 25%, color-mix(in srgb, var(--card-border-hover) 50%, var(--card-border)) 50%, var(--card-border) 75%);
    background-size: 200% 100%;
    animation: shimmer 1.5s infinite;
  }

  .skeleton-line.full { width: 85%; }
  .skeleton-line.three-quarter { width: 65%; }
  .skeleton-line.half { width: 45%; }
  .skeleton-line.third { width: 30%; }

  @keyframes shimmer {
    0% { background-position: 200% 0; }
    100% { background-position: -200% 0; }
  }

  @media (prefers-reduced-motion: reduce) {
    .skeleton-bar.title,
    .skeleton-line {
      animation: none;
    }
  }
</style>
