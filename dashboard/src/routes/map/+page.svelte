<script lang="ts">
  import { onMount } from "svelte";
  import { page } from "$app/state";
  import Icon from "$lib/Icon.svelte";
  import PageHeader from "$lib/PageHeader.svelte";
  import MapSurface from "$lib/map/MapSurface.svelte";
  import type { MapFeatureCollection, MapLayerSpec } from "$lib/map/surface";
  import { link } from "$lib/nav";
  import {
    MARK_ACCENT,
    MARK_HALO,
    MARK_INK,
    PHASE_PAST,
    PHASE_UPCOMING_LINE,
    PHASE_UPCOMING_POINT,
    PRESENCE_COLOR,
    SPEND_COLOR,
    STATION_COLOR,
  } from "$lib/map/style";
  import {
    places,
    type GeocodeResult,
    type PeopleLayer,
    type PersonPlaceProposal,
    type RegistryPlace,
    type SpendCitySummary,
    type SpendCityProperties,
    type SpendLayer,
    type SpendVenueProperties,
    type TravelLayer,
    type TravelPointProperties,
    type PeoplePinProperties,
    type UnplacedGroup,
  } from "$lib/api";

  // Every mark colour is $lib/map/style.ts now, shared with /travel: one palette for the
  // two surfaces rather than a copy per route. People pins wear the accent.
  const PEOPLE_COLOR = MARK_ACCENT;

  let spend = $state<SpendLayer | null>(null);
  let travel = $state<TravelLayer | null>(null);
  let people = $state<PeopleLayer | null>(null);
  let proposals = $state<PersonPlaceProposal[]>([]);
  let unplaced = $state<UnplacedGroup[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let proposalNotice = $state<string | null>(null);
  let reviewing = $state<string | null>(null);

  // The place-this flow. One group's assign form is open at a time, so its
  // search state is singular and reset on every open/close.
  let unplacedOpen = $state(false);
  let openGroup = $state<string | null>(null);
  let searchText = $state("");
  let registryResults = $state<RegistryPlace[]>([]);
  let geocodeResult = $state<GeocodeResult | null>(null);
  /** The exact text the preview was geocoded from — what an assign re-sends, so
   *  the server's cached resolve answers it without a second provider request. */
  let geocodeQuery = $state("");
  let geocoding = $state(false);
  let assignBusy = $state(false);
  let assignError = $state<string | null>(null);
  let assignNotice = $state<string | null>(null);
  let noticeTimer: ReturnType<typeof setTimeout> | undefined;

  let showSpend = $state(true);
  let showTravel = $state(true);
  let showPeople = $state(true);
  let panelOpen = $state(false);
  let mapView: MapSurface | undefined = $state();

  const EMPTY: MapFeatureCollection = { type: "FeatureCollection", features: [] };

  const money = (cents: number) =>
    (cents / 100).toLocaleString("en-GB", { style: "currency", currency: "EUR" });

  onMount(() => {
    void (async () => {
      const [spendResult, travelResult, peopleResult, proposalResult, unplacedResult] =
        await Promise.allSettled([
          places.spendLayer(),
          places.travelLayer(),
          places.peopleLayer(),
          places.proposals(),
          places.unplaced(),
        ]);
      if (spendResult.status === "fulfilled") spend = spendResult.value;
      if (travelResult.status === "fulfilled") travel = travelResult.value;
      if (peopleResult.status === "fulfilled") people = peopleResult.value;
      if (proposalResult.status === "fulfilled") proposals = proposalResult.value.proposals;
      if (unplacedResult.status === "fulfilled") unplaced = unplacedResult.value.groups;
      // Every call failing is one condition, not five: places is not reachable, and
      // the first rejection already carries the standard "not running" hint.
      if (
        [spendResult, travelResult, peopleResult, proposalResult, unplacedResult].every(
          (result) => result.status === "rejected",
        )
      ) {
        const cause = (spendResult as PromiseRejectedResult).reason;
        error = cause instanceof Error ? cause.message : String(cause);
      }
      loading = false;
    })();
    return () => clearTimeout(noticeTimer);
  });

  const hasTargetQuery = $derived(
    Boolean(
      page.url.searchParams.get("q") ||
        page.url.searchParams.get("city") ||
        page.url.searchParams.get("person") ||
        page.url.searchParams.get("lat"),
    ),
  );

  let autoFocusedQuery = $state<string | null>(null);

  $effect(() => {
    if (loading || !mapView) return;
    const q =
      page.url.searchParams.get("q") ||
      page.url.searchParams.get("city") ||
      page.url.searchParams.get("person");
    const latStr = page.url.searchParams.get("lat");
    const lonStr = page.url.searchParams.get("lon");

    if (latStr && lonStr) {
      const lat = parseFloat(latStr);
      const lon = parseFloat(lonStr);
      if (!Number.isNaN(lat) && !Number.isNaN(lon)) {
        mapView.flyTo([lon, lat], 12);
        return;
      }
    }

    if (!q || q === autoFocusedQuery) return;
    autoFocusedQuery = q;
    const term = q.trim().toLowerCase();

    // 1. Check People pins
    const personPin = people?.features.find(
      (f) =>
        f.properties.person.toLowerCase().includes(term) ||
        f.properties.place_name.toLowerCase().includes(term),
    );
    if (personPin) {
      mapView.flyTo(personPin.geometry.coordinates, 11);
      return;
    }

    // 2. Check Spend cities
    const cityMatch = spend?.summary.cities.find((c) =>
      c.city.toLowerCase().includes(term),
    );
    if (cityMatch && cityMatch.latitude !== null && cityMatch.longitude !== null) {
      mapView.flyTo([cityMatch.longitude, cityMatch.latitude], 10);
      return;
    }

    // 3. Check Travel points
    const travelPoint = travel?.points.features.find((f) =>
      f.properties.name.toLowerCase().includes(term),
    );
    if (travelPoint) {
      mapView.flyTo(travelPoint.geometry.coordinates, 11);
      return;
    }

    // 4. Fallback to places geocode
    void places
      .geocode({ query: q })
      .then((res) => {
        if (res.place && res.place.latitude !== null && res.place.longitude !== null) {
          mapView?.flyTo([res.place.longitude, res.place.latitude], 11);
        }
      })
      .catch(() => {});
  });

  const sources = $derived<Record<string, MapFeatureCollection>>({
    "travel-routes": showTravel && travel ? travel.routes : EMPTY,
    "spend-cities": showSpend && spend ? spend.cities : EMPTY,
    "spend-venues": showSpend && spend ? spend.venues : EMPTY,
    "travel-points": showTravel && travel ? travel.points : EMPTY,
    "people-pins": showPeople && people ? people : EMPTY,
  });

  const featureCount = $derived(
    Object.values(sources).reduce((total, collection) => total + collection.features.length, 0),
  );

  // Venue radius ~ sqrt(total_cents), so area tracks money. City aggregates are
  // hollow and damped — never mistakable for a venue pin (places README, D1).
  const venueRadius = [
    "interpolate", ["linear"], ["sqrt", ["get", "total_cents"]],
    0, 3, 22, 4, 100, 8, 316, 14, 1000, 22,
  ];
  const cityRadius = [
    "interpolate", ["linear"], ["sqrt", ["get", "total_cents"]],
    0, 4, 100, 9, 316, 15, 1000, 24,
  ];

  const layers: MapLayerSpec[] = [
    {
      id: "travel-routes",
      type: "line",
      source: "travel-routes",
      paint: {
        "line-color": ["match", ["get", "phase"], "past", PHASE_PAST, PHASE_UPCOMING_LINE],
        "line-width": 2.5,
        "line-opacity": 0.6,
        "line-dasharray": [1.5, 1.5],
      },
    },
    {
      id: "spend-cities",
      type: "circle",
      source: "spend-cities",
      paint: {
        "circle-radius": cityRadius,
        "circle-color": SPEND_COLOR,
        "circle-opacity": 0.08,
        "circle-stroke-color": SPEND_COLOR,
        "circle-stroke-width": 1.5,
        "circle-stroke-opacity": 0.6,
      },
    },
    {
      id: "spend-venues",
      type: "circle",
      source: "spend-venues",
      paint: {
        "circle-radius": venueRadius,
        "circle-color": SPEND_COLOR,
        "circle-opacity": 0.78,
        "circle-stroke-color": MARK_HALO,
        "circle-stroke-width": 1.5,
      },
    },
    {
      id: "travel-points",
      type: "circle",
      source: "travel-points",
      paint: {
        "circle-radius": ["match", ["get", "kind"], "trip-destination", 6, "station", 3.5, 2.5],
        "circle-color": [
          "case",
          ["==", ["get", "kind"], "spend-presence"], PRESENCE_COLOR,
          ["==", ["get", "kind"], "station"], STATION_COLOR,
          ["==", ["get", "phase"], "past"], PHASE_PAST,
          PHASE_UPCOMING_POINT,
        ],
        "circle-opacity": ["match", ["get", "kind"], "spend-presence", 0.45, 0.9],
        "circle-stroke-color": MARK_HALO,
        "circle-stroke-width": ["match", ["get", "kind"], "spend-presence", 0, "station", 1, 2],
      },
    },
    {
      id: "people-pins",
      type: "circle",
      source: "people-pins",
      paint: {
        "circle-radius": 7,
        "circle-color": PEOPLE_COLOR,
        "circle-stroke-color": MARK_HALO,
        "circle-stroke-width": 2,
      },
    },
    {
      id: "people-labels",
      type: "symbol",
      source: "people-pins",
      layout: {
        "text-field": ["get", "person"],
        "text-size": 12,
        "text-offset": [0, 1.3],
        "text-anchor": "top",
      },
      paint: {
        "text-color": MARK_INK,
        "text-halo-color": MARK_HALO,
        "text-halo-width": 1.5,
      },
    },
  ];

  function esc(value: unknown): string {
    return String(value)
      .replaceAll("&", "&amp;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;")
      .replaceAll('"', "&quot;")
      .replaceAll("'", "&#39;");
  }

  const visits = (n: number) => `${n} ${n === 1 ? "visit" : "visits"}`;

  function popupHtml(layerId: string, props: Record<string, unknown>): string | null {
    if (layerId === "spend-venues") {
      const p = props as unknown as SpendVenueProperties;
      const lines = [
        `${esc(money(p.total_cents))} · ${visits(p.transactions)} · ${esc(money(p.avg_cents))} avg`,
      ];
      if (p.top_category) lines.push(`Mostly ${esc(p.top_category)}`);
      lines.push(`${esc(p.first)} → ${esc(p.last)}`);
      const city = p.city ? ` · ${esc(p.city)}` : "";
      return `<strong>${esc(p.name)}</strong>${city}<br>${lines.join("<br>")}<br><a class="map-popup-link" href="${link("/finance")}">View in Finance &rarr;</a>`;
    }
    if (layerId === "spend-cities") {
      const p = props as unknown as SpendCityProperties;
      const country = p.country_code ? ` · ${esc(p.country_code)}` : "";
      return (
        `<strong>${esc(p.city)}</strong>${country}<br>` +
        `${esc(money(p.total_cents))} · ${visits(p.transactions)}<br>` +
        `<em>City-level aggregate — no exact venue known.</em><br>` +
        `<a class="map-popup-link" href="${link("/finance")}">View in Finance &rarr;</a>`
      );
    }
    if (layerId === "travel-points") {
      const p = props as unknown as TravelPointProperties;
      const kind =
        p.kind === "trip-destination" ? "Trip destination"
        : p.kind === "station" ? "Station"
        : "Spend presence";
      const lines = [`${kind}${p.phase ? ` · ${esc(p.phase)}` : ""}`];
      if (p.first || p.last) lines.push(`${esc(p.first ?? "…")} → ${esc(p.last ?? "…")}`);
      if (p.visits !== null && p.visits !== undefined) lines.push(visits(p.visits));
      const travelHref = p.plan_id ? link(`/travel?plan=${encodeURIComponent(p.plan_id)}`) : link("/travel");
      const linkText = p.kind === "trip-destination" ? "View in Travel &rarr;" : "Open Travel &rarr;";
      return `<strong>${esc(p.name)}</strong><br>${lines.join("<br>")}<br><a class="map-popup-link" href="${travelHref}">${linkText}</a>`;
    }
    if (layerId === "people-pins") {
      const p = props as unknown as PeoplePinProperties;
      const lines = [esc(p.place_name)];
      if (p.since) lines.push(`Since ${esc(p.since)}`);
      lines.push(`${Math.round(p.confidence_bp / 100)}% · ${esc(p.source)}`);
      const personHref = p.id
        ? link(`/people?id=${encodeURIComponent(p.id)}`)
        : link(`/people?search=${encodeURIComponent(p.person)}`);
      return `<strong>${esc(p.person)}</strong><br>${lines.join("<br>")}<br><a class="map-popup-link" href="${personHref}">View person dossier &rarr;</a>`;
    }
    return null;
  }

  const maxCityTotal = $derived(
    spend ? Math.max(1, ...spend.summary.cities.map((city) => city.total_cents)) : 1,
  );

  function flyToCity(city: SpendCitySummary): void {
    if (city.latitude === null || city.longitude === null) return;
    mapView?.flyTo([city.longitude, city.latitude], 10);
    panelOpen = false;
  }

  const travelCounts = $derived.by(() => {
    if (!travel) return null;
    const byKind = { "trip-destination": 0, station: 0, "spend-presence": 0 };
    for (const feature of travel.points.features) byKind[feature.properties.kind] += 1;
    return { ...byKind, legs: travel.routes.features.length };
  });

  const spendHasData = $derived(
    spend !== null && (spend.summary.transactions > 0 || spend.summary.cities.length > 0),
  );
  const travelHasData = $derived(
    travel !== null && (travel.points.features.length > 0 || travel.routes.features.length > 0),
  );
  const peopleHasData = $derived(people !== null && people.features.length > 0);

  async function review(id: string, decision: "confirm" | "dismiss"): Promise<void> {
    const before = proposals;
    proposals = proposals.filter((proposal) => proposal.id !== id);
    proposalNotice = null;
    reviewing = id;
    try {
      if (decision === "confirm") {
        await places.confirmProposal(id);
        // A confirmed row is now a pin; refresh the layer rather than fabricating one.
        people = await places.peopleLayer().catch(() => people);
      } else {
        await places.dismissProposal(id);
      }
    } catch (cause) {
      proposals = before;
      proposalNotice = cause instanceof Error ? cause.message : String(cause);
    } finally {
      reviewing = null;
    }
  }

  // ── Place-this: link an unplaced description group to a registry place ──────

  function closeAssign(): void {
    openGroup = null;
    searchText = "";
    registryResults = [];
    geocodeResult = null;
    geocodeQuery = "";
    assignError = null;
  }

  function toggleAssign(group: UnplacedGroup): void {
    if (assignBusy) return;
    if (openGroup === group.description) {
      closeAssign();
      return;
    }
    closeAssign();
    openGroup = group.description;
  }

  // Registry search is debounced per keystroke; it only touches the local
  // registry. The geocoder is behind the explicit "Search the map" button,
  // never a keystroke — it is throttled to 1 request/s upstream (places D3).
  $effect(() => {
    const q = searchText.trim();
    if (openGroup === null || q.length < 2) {
      registryResults = [];
      return;
    }
    // The cleanup aborts the in-flight request too, not just the pending
    // timer: a slower stale response must never repopulate the list after
    // closeAssign cleared it or another group's search replaced it.
    const aborter = new AbortController();
    const timer = setTimeout(async () => {
      try {
        registryResults = (await places.list(q, undefined, aborter.signal)).places.slice(0, 8);
      } catch {
        // No suggestions is not a broken page; assign reports real failures.
        // An abort lands here too and must not clear a newer run's results.
        if (!aborter.signal.aborted) registryResults = [];
      }
    }, 200);
    return () => {
      clearTimeout(timer);
      aborter.abort();
    };
  });

  async function searchMap(): Promise<void> {
    const q = searchText.trim();
    if (!q || geocoding) return;
    geocoding = true;
    geocodeResult = null;
    geocodeQuery = q;
    assignError = null;
    try {
      geocodeResult = await places.geocode({ query: q });
    } catch (cause) {
      assignError = cause instanceof Error ? cause.message : String(cause);
    } finally {
      geocoding = false;
    }
  }

  function flashNotice(text: string): void {
    assignNotice = text;
    clearTimeout(noticeTimer);
    noticeTimer = setTimeout(() => (assignNotice = null), 6000);
  }

  async function assign(
    group: UnplacedGroup,
    target: { place_id: string } | { geocode_query: string },
    precision: "venue" | "city",
  ): Promise<void> {
    if (assignBusy) return;
    const before = unplaced;
    unplaced = unplaced.filter((g) => g.description !== group.description);
    assignBusy = true;
    assignError = null;
    try {
      const result = await places.assignPlace({
        description: group.description,
        place_id: "place_id" in target ? target.place_id : null,
        geocode_query: "geocode_query" in target ? target.geocode_query : null,
        precision,
      });
      flashNotice(
        `${result.linked} ${result.linked === 1 ? "transaction" : "transactions"} linked to ${result.place.name}`,
      );
      closeAssign();
      // The new pin is places' verdict, not ours: refetch the layer rather than
      // fabricating a feature (same rule as the people layer after a confirm).
      spend = await places.spendLayer().catch(() => spend);
    } catch (cause) {
      unplaced = before;
      assignError = cause instanceof Error ? cause.message : String(cause);
    } finally {
      assignBusy = false;
    }
  }

  // A registry row that IS a city must not be linked at venue precision — a
  // city bubble never pretends to be a venue (places README, D1).
  const pickPrecision = (place: RegistryPlace): "venue" | "city" =>
    place.kind === "city" ? "city" : "venue";
</script>

<PageHeader
  badge="Places"
  title="One map, three layers"
  desc="Where money went, where travel led, and where the people in the register are."
/>

{#if error}
  <p class="err page-err"><Icon name="alert" size={14} /> {error}</p>
{/if}

<div class="map-shell">
  {#if !error}
  <button
    type="button"
    class="btn btn-outline panel-toggle"
    aria-expanded={panelOpen}
    onclick={() => (panelOpen = !panelOpen)}
  >
    <Icon name={panelOpen ? "close" : "globe"} size={14} />
    {panelOpen ? "Close" : "Layers"}
  </button>

  <aside class="panel card" class:open={panelOpen}>
      <div class="toggles" role="group" aria-label="Map layers">
        <label class="toggle">
          <input type="checkbox" bind:checked={showSpend} />
          <span class="swatch spend"></span> Spend
        </label>
        <label class="toggle">
          <input type="checkbox" bind:checked={showTravel} />
          <span class="swatch travel"></span> Travel
        </label>
        <label class="toggle">
          <input type="checkbox" bind:checked={showPeople} />
          <span class="swatch people"></span> People
        </label>
      </div>

      {#if loading}
        <p class="muted">Loading layers…</p>
      {/if}

      {#if showSpend && spendHasData && spend}
        <section class="layer-section">
          <h2>Spend</h2>
          <div class="tiles">
            <div class="tile">
              <span class="tile-label">Total spent</span>
              <span class="tile-value">{money(spend.summary.total_cents)}</span>
            </div>
            <!-- All three tiles describe the PLACED population: total_cents is
                 summed over linked rows only (places layers.rs, spend_layer), so
                 linked — never the all-transactions count — is the visit count
                 and the average's denominator. Coverage is the line below. -->
            <div class="tile">
              <span class="tile-label">Visits</span>
              <span class="tile-value">{spend.summary.linked}</span>
            </div>
            <div class="tile">
              <span class="tile-label">Avg / visit</span>
              <span class="tile-value">
                {money(
                  spend.summary.linked > 0
                    ? Math.round(spend.summary.total_cents / spend.summary.linked)
                    : 0,
                )}
              </span>
            </div>
          </div>
          <p class="muted small">
            {spend.summary.venues}
            {spend.summary.venues === 1 ? "venue" : "venues"} pinned ·
            {spend.summary.linked} of {spend.summary.transactions} placed
          </p>
          <p class="legend">
            <span class="legend-item"><span class="dot venue-dot"></span> venue</span>
            <span class="legend-item"><span class="dot city-dot"></span> city aggregate</span>
          </p>
          {#if spend.summary.cities.length > 0}
            <ol class="cities">
              {#each spend.summary.cities as city (`${city.city}|${city.country_code ?? ""}`)}
                <li>
                  <button
                    type="button"
                    class="city-row"
                    disabled={city.latitude === null || city.longitude === null}
                    onclick={() => flyToCity(city)}
                  >
                    <span class="city-line">
                      <span class="city-name">
                        {city.city}{city.country_code ? ` · ${city.country_code}` : ""}
                      </span>
                      <span class="city-total mono">{money(city.total_cents)}</span>
                    </span>
                    <span class="city-track">
                      <span
                        class="city-bar"
                        style="width: {(city.total_cents / maxCityTotal) * 100}%"
                      ></span>
                    </span>
                  </button>
                </li>
              {/each}
            </ol>
          {/if}

          <!-- Guarded on the notice too: assigning the last group must not
               swallow its own confirmation with the section. -->
          {#if unplaced.length > 0 || assignNotice}
            <div class="unplaced">
              {#if unplaced.length > 0}
                <button
                  type="button"
                  class="unplaced-toggle"
                  aria-expanded={unplacedOpen}
                  onclick={() => (unplacedOpen = !unplacedOpen)}
                >
                  <span class="chev" class:open={unplacedOpen}>
                    <Icon name="chevron" size={12} />
                  </span>
                  Unplaced · {unplaced.length}
                  {unplaced.length === 1 ? "group" : "groups"}
                </button>
              {/if}
              {#if assignNotice}
                <p class="assign-notice"><Icon name="check" size={13} /> {assignNotice}</p>
              {/if}
              {#if unplacedOpen}
                <ul class="groups">
                  {#each unplaced as group (group.description)}
                    <li class="group" class:active={openGroup === group.description}>
                      <button
                        type="button"
                        class="group-row"
                        onclick={() => toggleAssign(group)}
                      >
                        <span class="city-line">
                          <span class="city-name">{group.description}</span>
                          <span class="city-total mono">{money(group.total_cents)}</span>
                        </span>
                        <span class="group-meta">
                          {group.transactions}
                          {group.transactions === 1 ? "transaction" : "transactions"}
                          · {group.first} → {group.last}
                        </span>
                      </button>
                      {#if openGroup === group.description}
                        <div class="assign">
                          <!-- The placeholder is not the name. It disappears the moment a
                               character is typed, and a screen reader reaching this field
                               after that is told only "search". -->
                          <input
                            class="input assign-input"
                            type="search"
                            aria-label="Search places to assign this merchant to"
                            placeholder="Search places…"
                            autocomplete="off"
                            bind:value={searchText}
                          />
                          {#if registryResults.length > 0}
                            <ul class="results">
                              {#each registryResults as place (place.id)}
                                <li class="result">
                                  <button
                                    type="button"
                                    class="result-pick"
                                    disabled={assignBusy}
                                    onclick={() =>
                                      assign(group, { place_id: place.id }, pickPrecision(place))}
                                  >
                                    <Icon name="map-pin" size={12} />
                                    <span class="result-name">
                                      {place.name}{place.city && place.kind !== "city"
                                        ? ` · ${place.city}`
                                        : ""}
                                    </span>
                                    <span class="tag">{place.kind}</span>
                                  </button>
                                  {#if place.kind !== "city"}
                                    <button
                                      type="button"
                                      class="btn btn-outline btn-mini"
                                      disabled={assignBusy}
                                      onclick={() => assign(group, { place_id: place.id }, "city")}
                                    >
                                      City only
                                    </button>
                                  {/if}
                                </li>
                              {/each}
                            </ul>
                          {/if}
                          <button
                            type="button"
                            class="btn btn-outline btn-mini map-search"
                            disabled={geocoding || assignBusy || searchText.trim().length === 0}
                            onclick={searchMap}
                          >
                            <Icon name={geocoding ? "loader" : "search"} size={13} />
                            Search the map
                          </button>
                          {#if geocodeResult}
                            {#if geocodeResult.status === "ok" && geocodeResult.place}
                              {@const found = geocodeResult.place}
                              <div class="geocoded">
                                <span class="result-name">
                                  {found.name}{found.city ? ` · ${found.city}` : ""}
                                  <span class="tag">{found.kind}</span>
                                </span>
                                <div class="geocoded-actions">
                                  <!-- Same rule as pickPrecision on registry rows: a
                                       city-kind result gets no venue button (places D1);
                                       the server enforces the downgrade regardless. -->
                                  {#if found.kind !== "city"}
                                    <button
                                      type="button"
                                      class="btn btn-outline btn-mini"
                                      disabled={assignBusy}
                                      onclick={() =>
                                        assign(group, { geocode_query: geocodeQuery }, "venue")}
                                    >
                                      <Icon name="map-pin" size={12} /> Pin venue
                                    </button>
                                  {/if}
                                  <button
                                    type="button"
                                    class="btn btn-outline btn-mini"
                                    disabled={assignBusy}
                                    onclick={() =>
                                      assign(group, { geocode_query: geocodeQuery }, "city")}
                                  >
                                    {found.kind === "city" ? "Pin city" : "City only"}
                                  </button>
                                </div>
                              </div>
                            {:else}
                              <p class="muted small">The map found nothing for that search.</p>
                            {/if}
                          {/if}
                          {#if assignError}
                            <p class="err"><Icon name="alert" size={14} /> {assignError}</p>
                          {/if}
                        </div>
                      {/if}
                    </li>
                  {/each}
                </ul>
              {/if}
            </div>
          {/if}
        </section>
      {/if}

      {#if showTravel && travelHasData && travelCounts}
        <section class="layer-section">
          <h2>Travel</h2>
          <p class="legend">
            <span class="legend-item"><span class="dot upcoming-dot"></span> upcoming</span>
            <span class="legend-item"><span class="dot past-dot"></span> past</span>
          </p>
          <p class="muted small">
            {travelCounts["trip-destination"]} destinations ·
            {travelCounts.station} stations ·
            {travelCounts.legs} legs{travelCounts["spend-presence"] > 0
              ? ` · ${travelCounts["spend-presence"]} spend traces`
              : ""}
          </p>
        </section>
      {/if}

      {#if showPeople && (peopleHasData || proposals.length > 0)}
        <section class="layer-section">
          <h2>People</h2>
          {#if peopleHasData && people}
            <ul class="people">
              {#each people.features as pin (pin.properties.id)}
                <li>
                  <button
                    type="button"
                    class="person-row"
                    onclick={() => {
                      mapView?.flyTo(pin.geometry.coordinates, 9);
                      panelOpen = false;
                    }}
                  >
                    <strong>{pin.properties.person}</strong>
                    <span class="muted">
                      {pin.properties.place_name}{pin.properties.since
                        ? ` · since ${pin.properties.since}`
                        : ""}
                    </span>
                  </button>
                </li>
              {/each}
            </ul>
          {/if}
          {#if proposals.length > 0}
            <h3>Proposals</h3>
            <ul class="proposals">
              {#each proposals as proposal (proposal.id)}
                <li class="proposal">
                  <div class="proposal-body">
                    <strong>{proposal.person}</strong>
                    <span class="muted">
                      {proposal.place_name}{proposal.city ? ` · ${proposal.city}` : ""}
                    </span>
                    <span class="muted small">
                      {Math.round(proposal.confidence_bp / 100)}% · {proposal.source}
                    </span>
                  </div>
                  <div class="proposal-actions">
                    <button
                      type="button"
                      class="btn btn-outline"
                      disabled={reviewing !== null}
                      onclick={() => review(proposal.id, "confirm")}
                    >
                      Confirm
                    </button>
                    <button
                      type="button"
                      class="btn btn-danger"
                      disabled={reviewing !== null}
                      onclick={() => review(proposal.id, "dismiss")}
                    >
                      Dismiss
                    </button>
                  </div>
                </li>
              {/each}
            </ul>
          {/if}
          {#if proposalNotice}
            <p class="err"><Icon name="alert" size={14} /> {proposalNotice}</p>
          {/if}
        </section>
      {/if}
  </aside>
  {/if}

  <div class="map-area">
    <MapSurface
      bind:this={mapView}
      {sources}
      {layers}
      interactive={["people-pins", "travel-points", "spend-venues", "spend-cities"]}
      {popupHtml}
      eager
      autoFit={!hasTargetQuery}
      deferredLabel={`${featureCount} ${featureCount === 1 ? "place" : "places"} on the map`}
    />
  </div>
</div>

<style>
  .map-shell {
    position: relative;
    display: flex;
    gap: 1rem;
    height: clamp(28rem, calc(100dvh - 19rem), 75rem);
  }

  .panel {
    width: 21.5rem;
    flex-shrink: 0;
    overflow-y: auto;
    padding: 1rem;
  }

  .map-area {
    flex: 1;
    min-width: 0;
  }

  .panel-toggle {
    display: none;
  }

  .toggles {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
    margin-bottom: 0.75rem;
  }

  .toggle {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.3rem 0.6rem;
    border: 1px solid var(--card-border);
    border-radius: 999px;
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-secondary);
    cursor: pointer;
  }

  .toggle:has(input:checked) {
    border-color: var(--primary);
    color: var(--text-primary);
    background-color: var(--primary-soft);
  }

  .toggle input {
    position: absolute;
    opacity: 0;
    pointer-events: none;
  }

  .swatch {
    width: 0.6rem;
    height: 0.6rem;
    border-radius: 999px;
  }

  .swatch.spend {
    background-color: #0e7490;
  }

  .swatch.travel {
    background-color: #06b6d4;
  }

  .swatch.people {
    background-color: #d97706;
  }

  .layer-section {
    padding-top: 0.75rem;
    margin-top: 0.75rem;
    border-top: 1px solid var(--card-border);
  }

  .layer-section h2 {
    margin: 0 0 0.5rem;
    font-size: var(--text-2xs);
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-tertiary);
  }

  .layer-section h3 {
    margin: 0.9rem 0 0.4rem;
    font-size: var(--text-2xs);
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-tertiary);
  }

  .tiles {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 0.5rem;
  }

  .tile {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    padding: 0.55rem 0.6rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    background-color: var(--surface);
  }

  .tile-label {
    font-size: 0.5625rem;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-tertiary);
  }

  .tile-value {
    font-size: var(--text-base);
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    letter-spacing: -0.01em;
    color: var(--text-primary);
  }

  .legend {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin: 0.5rem 0 0;
    font-size: var(--text-2xs);
    color: var(--text-secondary);
  }

  .legend-item {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
  }

  .dot {
    width: 0.55rem;
    height: 0.55rem;
    border-radius: 999px;
    flex-shrink: 0;
  }

  .venue-dot {
    background-color: #0e7490;
  }

  .city-dot {
    border: 1.5px solid #0e7490;
    background-color: transparent;
  }

  .upcoming-dot {
    background-color: #06b6d4;
  }

  .past-dot {
    background-color: #71717a;
  }

  .cities {
    list-style: none;
    margin: 0.6rem 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }

  .city-row {
    display: block;
    width: 100%;
    padding: 0.3rem 0.35rem;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    font: inherit;
    color: inherit;
    text-align: left;
    cursor: pointer;
  }

  .city-row:hover:not(:disabled) {
    background-color: var(--primary-soft);
  }

  .city-row:disabled {
    cursor: default;
  }

  .city-line {
    display: flex;
    justify-content: space-between;
    gap: 0.5rem;
    font-size: var(--text-xs);
  }

  .city-name {
    font-weight: 600;
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .city-total {
    color: var(--text-secondary);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .city-track {
    display: block;
    height: 0.3rem;
    margin-top: 0.2rem;
    border-radius: 999px;
    background-color: var(--surface);
    overflow: hidden;
  }

  .city-bar {
    display: block;
    height: 100%;
    border-radius: 999px;
    background-color: var(--primary);
    opacity: 0.55;
  }

  .people,
  .proposals {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }

  .person-row {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    width: 100%;
    padding: 0.35rem 0.35rem;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    font: inherit;
    font-size: var(--text-xs);
    color: inherit;
    text-align: left;
    cursor: pointer;
  }

  .person-row:hover {
    background-color: var(--accent-soft);
  }

  .proposal {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    padding: 0.45rem 0.35rem;
    border: 1px dashed var(--card-border);
    border-radius: var(--radius-sm);
  }

  .proposal-body {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    font-size: var(--text-xs);
    min-width: 0;
  }

  .proposal-actions {
    display: flex;
    gap: 0.25rem;
    flex-shrink: 0;
  }

  .muted {
    margin: 0.4rem 0 0;
    font-size: var(--text-xs);
    color: var(--text-tertiary);
  }

  .small {
    font-size: var(--text-2xs);
  }

  .err {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    margin: 0.5rem 0 0;
    font-size: var(--text-xs);
    color: var(--danger);
  }

  .page-err {
    margin: 0 0 0.75rem;
  }

  .unplaced {
    margin-top: 0.75rem;
  }

  .unplaced-toggle {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.2rem 0.25rem;
    border: 0;
    background: transparent;
    font: inherit;
    font-size: var(--text-2xs);
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-tertiary);
    cursor: pointer;
  }

  .unplaced-toggle:hover {
    color: var(--text-primary);
  }

  .chev {
    display: flex;
    transition: transform var(--motion-fast) ease;
  }

  .chev.open {
    transform: rotate(90deg);
  }

  .assign-notice {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    margin: 0.4rem 0 0;
    font-size: var(--text-2xs);
    color: var(--success);
  }

  .groups {
    list-style: none;
    margin: 0.4rem 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .group.active {
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    padding-bottom: 0.5rem;
  }

  .group-row {
    display: block;
    width: 100%;
    padding: 0.3rem 0.35rem;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    font: inherit;
    color: inherit;
    text-align: left;
    cursor: pointer;
  }

  .group-row:hover {
    background-color: var(--primary-soft);
  }

  .group-meta {
    display: block;
    margin-top: 0.1rem;
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
  }

  .assign {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 0.4rem;
    padding: 0.1rem 0.35rem 0;
  }

  .assign-input {
    font-size: var(--text-xs);
    padding: 0.35rem 0.5rem;
  }

  .results {
    list-style: none;
    margin: 0;
    padding: 0;
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
  }

  .result {
    display: flex;
    align-items: center;
    gap: 0.3rem;
  }

  .result-pick {
    display: flex;
    flex: 1;
    min-width: 0;
    align-items: center;
    gap: 0.4rem;
    padding: 0.3rem 0.35rem;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    font: inherit;
    font-size: var(--text-xs);
    color: var(--text-primary);
    text-align: left;
    cursor: pointer;
  }

  .result-pick:hover:not(:disabled) {
    background-color: var(--primary-soft);
  }

  .result-pick:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .result-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Panel-scale buttons: the .btn primitive at list density, not a new button. */
  .btn-mini {
    font-size: var(--text-2xs);
    padding: 0.25rem 0.5rem;
    flex-shrink: 0;
  }

  .geocoded {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    width: 100%;
    padding: 0.4rem 0.45rem;
    border: 1px dashed var(--card-border);
    border-radius: var(--radius-sm);
    font-size: var(--text-xs);
  }

  .geocoded-actions {
    display: flex;
    gap: 0.25rem;
    flex-wrap: wrap;
  }

  @media (width < 48rem) {
    .map-shell {
      height: clamp(24rem, calc(100dvh - 15rem), 60rem);
    }

    .panel-toggle {
      display: inline-flex;
      position: absolute;
      top: 0.75rem;
      left: 0.75rem;
      z-index: 10;
    }

    .panel {
      display: none;
      position: absolute;
      inset: 0;
      z-index: 9;
      width: auto;
      padding-top: 3.25rem;
    }

    .panel.open {
      display: block;
    }
  }

  :global(.map-popup-link) {
    display: inline-block;
    margin-top: 0.35rem;
    font-size: var(--text-2xs);
    font-weight: 600;
    color: var(--primary);
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  :global(.map-popup-link:hover) {
    color: var(--primary-hover);
  }
</style>
