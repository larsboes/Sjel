<script lang="ts">
  import { tip } from "$lib/tip";
  import { link } from "$lib/nav";
  import { page } from "$app/state";
  import { onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import Overlay from "$lib/Overlay.svelte";
  import PageHeader from "$lib/PageHeader.svelte";
  import Rail from "$lib/rail/Rail.svelte";
  import RailGroup from "$lib/rail/RailGroup.svelte";
  import RailSection from "$lib/rail/RailSection.svelte";
  import RelatedTools from "$lib/RelatedTools.svelte";
  import CompanionRail from "$lib/travel/CompanionRail.svelte";
  import { stageCoordinate } from "$lib/travel/who-is-around";
  import TravelerProfileModal from "$lib/travel/TravelerProfileModal.svelte";
  import PlaceField from "$lib/travel/PlaceField.svelte";
  import {
    TRIP_LAYERS,
    tripFitKey,
    tripSources,
    type MapPoint,
  } from "$lib/travel/trip-layers";
  import TravelHub from "$lib/travel/TravelHub.svelte";
  import NewTripModal from "$lib/travel/NewTripModal.svelte";
  import TripWorkspace from "$lib/travel/TripWorkspace.svelte";
  import TripPastView from "$lib/travel/TripPastView.svelte";
  import {
    climateFor,
    planCost,
    planSearch,
    type ClimateBatch,
    type ClimateResult,
    type PlanCost,
    type RankedCandidate,
  } from "$lib/travel/api";
  import {
    loadNearbyPlaces,
    type NearbyPlace,
  } from "$lib/travel/nearby-places";
  import { loadPlaceImage, placeName, type PlaceImage } from "$lib/travel/place-image";
  import type { Retrospective } from "$lib/api";
  import {
    assessTravelCandidates,
    calendarCandidatesFor,
    type TravelCandidateAssessment,
  } from "$lib/travel/travel-candidates";
  import { seedFromCandidate } from "$lib/travel/plan-search";
  import {
    axonStatus,
    calendar,
    comms,
    places,
    scouting,
    transit,
    trips,
    entities,
    JOURNEY_PRIORITIES,
    type JourneyOverride,
    type Entity,
    type LocatedPerson,
    type PersonPlaceProposal,
    type CalendarEntry,
    type CalendarCandidateVerdict,
    type BaseCandidate,
    type Journey,
    type ObsidianTripCandidate,
    type PlanItem,
    type PlaceRef,
    type ScoredResult,
    type ScoutingOpportunity,
    type TransportMode,
    type TripPlan,
  } from "$lib/api";

  const EVENT_ADAPTERS = ["luma", "meetup", "euro_hackathons"];
  const MODE_OPTIONS: Array<{ id: TransportMode; label: string }> = [
    { id: "train", label: "Train" },
    { id: "flight", label: "Flight" },
    { id: "car", label: "Car" },
    { id: "bus", label: "Bus" },
    { id: "ferry", label: "Ferry" },
    { id: "bike", label: "Bike" },
    { id: "walk", label: "Walk" },
  ];
  const isoDate = (date: Date) =>
    [
      date.getFullYear(),
      String(date.getMonth() + 1).padStart(2, "0"),
      String(date.getDate()).padStart(2, "0"),
    ].join("-");
  const today = new Date();
  const todayKey = isoDate(today);
  const nextWeek = new Date(today);
  nextWeek.setDate(today.getDate() + 7);

  interface DestinationResult {
    place: PlaceRef;
    city: string;
    image: PlaceImage | null;
    journeys: Journey[];
    anchors: CalendarEntry[];
    events: ScoredResult[];
    activities: NearbyPlace[];
    loading: boolean;
    notices: string[];
  }

  let plans = $state<TripPlan[]>([]);
  let activePlan = $state<TripPlan | null>(null);
  let items = $state<PlanItem[]>([]);
  let origin = $state<PlaceRef | null>(null);
  let firstDestination = $state<PlaceRef | null>(null);
  let secondDestination = $state<PlaceRef | null>(null);
  let showSecondDestination = $state(false);
  let startDate = $state(isoDate(today));
  let endDate = $state(isoDate(nextWeek));
  let interests = $state("");
  let travelers = $state("");
  let transportModes = $state<TransportMode[]>(["train"]);
  let results = $state<DestinationResult[]>([]);
  let selectedId = $state("");
  let creating = $state(false);
  let error = $state<string | null>(null);
  let savedNotice = $state<string | null>(null);
  let planNotice = $state<string | null>(null);
  let editingPlan = $state(false);
  let savingPlan = $state(false);
  let deletingPlan = $state(false);
  let planFilter = $state<"upcoming" | "past" | "all">("upcoming");
  let highlightedPlanId = $state<string | null>(null);
  // Ranked is the default because the server has the traveller's own weights and
  // the page does not. The other three stay as manual overrides: a reader who wants
  // cheapest-first for one glance should not have to change a preference to get it.
  let journeySort = $state<"ranked" | "price" | "duration" | "departure">("ranked");
  /// A preset id, or "" for the profile's usual weights.
  let journeyPriority = $state("");
  /// A sentence. Beats the preset, because it is the more specific thing to say.
  let journeyPhrase = $state("");
  let expandedJourneyId = $state<string | null>(null);
  let itineraryView = $state<"timeline" | "list">("timeline");
  let obsidianCandidates = $state<ObsidianTripCandidate[]>([]);
  let obsidianScanning = $state(false);
  let obsidianImporting = $state<string | null>(null);
  let obsidianImportingAll = $state(false);
  let obsidianNotice = $state<string | null>(null);
  let importOrigin = $state<PlaceRef | null>(null);
  let importOriginNeeded = $state(false);
  let importOriginField: HTMLDivElement | undefined = $state();
  let mapOpen = $state(false);
  let travelCandidates = $state<TravelCandidateAssessment[]>([]);
  let travelCandidatesLoading = $state(true);
  let travelCandidateNotice = $state<string | null>(null);
  let travelCandidateSaving = $state<string | null>(null);
  let savedTravelCandidates = $state<Set<string>>(new Set());
  // The planner is a dialog now, not a slab further down the page: a trip is
  // created rarely, the board is what you come here to read.
  let plannerOpen = $state(false);
  let plannerNotice = $state<string | null>(null);
  let profileOpen = $state(false);
  let openedPlanFromLink = "";
  // Seasonality for the open plan's destinations, keyed by "lat,lon" exactly as
  // sent so a result never has to be re-associated with its request.
  let climate = $state<ClimateBatch | null>(null);
  // The open plan's close-out record. Held beside `activePlan` rather than on
  // it, because `activePlan` is typed `TripPlan` and widening it would touch a
  // declaration another stream is also editing tonight.
  let planRetrospective = $state<Retrospective | null>(null);

  const selected = $derived(results.find((result) => result.place.id === selectedId) ?? null);
  const upcomingPlans = $derived(
    plans
      .filter((plan) => plan.date_end >= todayKey)
      .sort((a, b) => a.date_start.localeCompare(b.date_start)),
  );
  const pastPlans = $derived(
    plans
      .filter((plan) => plan.date_end < todayKey)
      .sort((a, b) => b.date_end.localeCompare(a.date_end)),
  );
  const viewingPast = $derived(activePlan !== null && activePlan.date_end < todayKey);
  // The one-line placeholder this replaced summed nothing and said nothing when
  // finance was down. The roll-up is computed by trips and rendered by CostCard,
  // which is the trips README's rule: the frontend renders, the backend computes.
  let planCostRollup = $state<PlanCost | null>(null);
  /**
   * Machine proposes, human confirms — and a proposal is one NAMED quantity in a
   * named unit, which is why the form prints which one it got.
   *
   * `personal_cents` when finance answered, else the booked total when the
   * bookings resolved to a single currency, else nothing. The call site used to
   * pass a literal null, so the form always said "no proposal" and its whole
   * prefill branch was dead.
   */
  const costProposal = $derived.by(() => {
    const rollup = planCostRollup;
    if (!rollup) return null;
    if (rollup.actuals.ok && rollup.actuals.personal_cents !== null) {
      return { cents: rollup.actuals.personal_cents, label: "actuals" };
    }
    if (rollup.booked_cents !== null && rollup.booked_cents > 0 && rollup.by_currency.length === 1) {
      return { cents: rollup.booked_cents, label: "bookings" };
    }
    return null;
  });

  let planCalendarEntries = $state<CalendarEntry[]>([]);
  let intentSentence = $state("");
  let draftingIntent = $state(false);
  let intentFeedback = $state<{ assumptions: string[]; unresolved: string[] } | null>(null);

  async function updateItemDay(item: PlanItem, newDay: string | null) {
    if (!activePlan) return;
    try {
      const updated = await trips.setItemDay(activePlan.id, item.id, newDay);
      items = items.map((it) => (it.id === updated.id ? updated : it));
      savedNotice = `Scheduled "${item.title}" ${newDay ? `for ${newDay}` : "as unscheduled"}`;
      setTimeout(() => {
        if (savedNotice?.includes(item.title)) savedNotice = null;
      }, 3000);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  async function loadCalendarEntries(plan: TripPlan): Promise<void> {
    try {
      planCalendarEntries = await calendar.entries.list(
        plan.date_start,
        dayAfter(plan.date_end),
      );
    } catch {
      planCalendarEntries = [];
    }
  }

  async function handleDraftIntent() {
    if (!intentSentence.trim() || draftingIntent) return;
    draftingIntent = true;
    intentFeedback = null;
    try {
      const res = await trips.draftIntent(intentSentence);
      const d = res.draft;
      if (d.destinations && d.destinations.length > 0) {
        firstDestination = d.destinations[0];
        if (d.destinations.length > 1) {
          secondDestination = d.destinations[1];
          showSecondDestination = true;
        }
      }
      if (d.date_start) startDate = d.date_start;
      if (d.date_end) endDate = d.date_end;
      if (d.interests) interests = d.interests;
      if (d.travelers && d.travelers.length > 0) travelers = d.travelers.join(", ");
      if (d.transport_modes && d.transport_modes.length > 0) {
        transportModes = d.transport_modes;
      }
      intentFeedback = {
        assumptions: res.assumptions,
        unresolved: res.unresolved,
      };
      plannerNotice = "Trip drafted! Review and confirm fields below.";
    } catch (e) {
      plannerNotice = e instanceof Error ? e.message : String(e);
    } finally {
      draftingIntent = false;
    }
  }

  /**
   * The result this destination's coordinate was sent under, and nothing else.
   *
   * `key` is the `lat,lon` pair exactly as sent, published by the route so a
   * caller never re-associates a result with its request. Matching on the
   * MATCHED place's name — and falling back to `results[0]` — rendered one
   * city's twelve months under another city's heading as soon as a plan carried
   * a second destination, or as soon as a coordinate resolved to a nearby place
   * with a different name.
   */
  function climateResultFor(place: PlaceRef): ClimateResult | null {
    if (!climate) return null;
    if (typeof place.latitude !== "number" || typeof place.longitude !== "number") return null;
    const key = `${place.latitude},${place.longitude}`;
    return climate.results.find((result) => result.key === key) ?? null;
  }

  const filteredPlans = $derived(
    planFilter === "upcoming" ? upcomingPlans : planFilter === "past" ? pastPlans : [...upcomingPlans, ...pastPlans],
  );
  const obsidianReadyCount = $derived(
    obsidianCandidates.filter(
      (candidate) => candidate.issues.length === 0 && !candidate.imported_plan_id,
    ).length,
  );
  // What the rail badges count: anything scanned but not yet a trip here.
  const obsidianPendingCount = $derived(
    obsidianCandidates.filter((candidate) => !candidate.imported_plan_id).length,
  );
  // The companion register's waiting decisions, read-only here. `places` is a
  // capability the published demo does not have, so its own 503 sentence is
  // carried into the section rather than a 404 body (demo/demo.toml
  // [absent.places]).
  let companionProposals = $state<PersonPlaceProposal[]>([]);
  let companionLoading = $state(true);
  let companionNotice = $state<string | null>(null);
  async function loadCompanionProposals(): Promise<void> {
    companionLoading = true;
    try {
      const answer = await places.proposals();
      companionProposals = answer.proposals;
      companionNotice = null;
    } catch (caught) {
      companionProposals = [];
      companionNotice = caught instanceof Error ? caught.message : String(caught);
    } finally {
      companionLoading = false;
    }
  }

  // Who is around each leg: entities' /api/located for each leg's day (PRD Q117), joined
  // here against the leg's coordinate (who-is-around.ts).
  let peopleNotice = $state<string | null>(null);
  let legCoordinates = $state<Record<string, [number, number] | null>>({});
  let locatedByDay = $state<Record<string, LocatedPerson[]>>({});
  let knownPeople = $state<Entity[]>([]);
  async function loadWhoIsAround(plan: TripPlan): Promise<void> {
    try {
      const days = [...new Set(plan.stages.map((stage) => stage.date).filter((d): d is string => !!d))];
      const [people, ...answers] = await Promise.all([
        entities.list("person"),
        ...days.map((day) => entities.located(day)),
      ]);
      knownPeople = people;
      locatedByDay = Object.fromEntries(answers.map((answer) => [answer.day, answer.located]));
      peopleNotice = null;
    } catch (caught) {
      locatedByDay = {};
      peopleNotice = caught instanceof Error ? caught.message : String(caught);
    }
    // A leg without its own coordinate borrows the registered city of that name,
    // the way the accommodation flow resolves a stage (capabilities/trips/README.md).
    const resolved: Record<string, [number, number] | null> = {};
    for (const stage of plan.stages) {
      const own = stageCoordinate(stage);
      if (own) {
        resolved[stage.id] = own;
        continue;
      }
      try {
        const found = await places.list(stage.destination.name, "city");
        const wanted = stage.destination.name.trim().toLowerCase();
        const city = found.places.find(
          (place) => place.name.trim().toLowerCase() === wanted && place.latitude != null && place.longitude != null,
        );
        resolved[stage.id] = city ? [city.latitude!, city.longitude!] : null;
      } catch {
        resolved[stage.id] = null;
      }
    }
    legCoordinates = resolved;
  }

  /** Which tab of the planner overlay is showing. */
  let plannerTab = $state<"find" | "new" | "base">("new");

  async function adoptBaseCandidate(
    base: BaseCandidate,
    from: PlaceRef,
    anchor: PlaceRef,
    fromDate: string,
    anchorDate: string,
  ): Promise<void> {
    const basePlace: PlaceRef = {
      id: base.place_id,
      name: base.name,
      kind: "city",
      latitude: base.latitude,
      longitude: base.longitude,
    };
    const title = `${placeName(from)} → ${base.name} → ${placeName(anchor)}`;
    const plan = await trips.create({
      title,
      origin: from,
      destinations: [basePlace, anchor],
      date_start: fromDate,
      date_end: anchorDate,
      interests: `Intermediate base in ${base.name}`,
      travelers: [],
      transport_modes: ["train"],
    });
    plans = [plan, ...plans];
    plannerOpen = false;
    plannerNotice = null;
    await openPlan(plan);
  }

  /**
   * Create the plan the operator chose, then record the whole option space on
   * it. The plan is never rolled back if adopt fails: deleting a durable row to
   * hide a transient error is the wrong way round, and adopt is idempotent on
   * (plan_id, item_type, external_id), so a retry is a correction.
   */
  async function createPlanFromCandidate(job: number, candidate: RankedCandidate): Promise<string> {
    if (!origin) throw new Error("Pick an origin first.");
    const seed = seedFromCandidate(candidate);
    const plan = await trips.create({
      title: placeName(candidate.destination),
      origin,
      destinations: [candidate.destination],
      date_start: seed.startDate,
      date_end: seed.endDate,
      interests: interests.trim(),
      travelers: [],
      transport_modes: [candidate.mode],
    });
    plans = [plan, ...plans];
    await planSearch.adopt(job, plan.id);
    return plan.id;
  }

  const travelReviewCount = $derived(
    travelCandidates.length + obsidianPendingCount + companionProposals.length,
  );
  const overviewMapPoints = $derived.by<MapPoint[]>(() =>
    filteredPlans.flatMap((plan) =>
      [plan.origin, ...plan.destinations]
        .filter(
          (place): place is PlaceRef & { latitude: number; longitude: number } =>
            typeof place.latitude === "number" && typeof place.longitude === "number",
        )
        .map((place, index) => ({
          id: `${plan.id}:${index}:${place.id}`,
          groupId: plan.id,
          routeId: plan.id,
          label: placeName(place),
          latitude: place.latitude,
          longitude: place.longitude,
          kind: index === 0 ? "origin" : "destination",
          phase: plan.date_end < todayKey ? "past" : "upcoming",
          selected: highlightedPlanId === plan.id,
        })),
    ),
  );
  const mapPoints = $derived.by<MapPoint[]>(() => {
    const plan = activePlan;
    if (!plan) return [];
    return [plan.origin, ...plan.destinations]
      .filter(
        (place): place is PlaceRef & { latitude: number; longitude: number } =>
          typeof place.latitude === "number" && typeof place.longitude === "number",
      )
      .map((place, index) => ({
        id: place.id,
        label: placeName(place),
        latitude: place.latitude,
        longitude: place.longitude,
        kind: index === 0 ? "origin" : "destination",
        routeId: plan.id,
        phase: plan.date_end < todayKey ? "past" : "upcoming",
        selected: place.id === selectedId,
      }));
  });

  // Derived rather than called in the template: both point arrays are rebuilt whenever a row
  // is hovered, and a fresh object literal per render would hand MapSurface a new `sources`
  // identity every frame. The fit key is what stops a hover from re-framing the camera.
  const overviewMapSources = $derived(tripSources(overviewMapPoints));
  const overviewMapFitKey = $derived(tripFitKey(overviewMapPoints));
  const tripMapSources = $derived(tripSources(mapPoints));
  const tripMapFitKey = $derived(tripFitKey(mapPoints));

  function planPhase(plan: TripPlan): "In progress" | "Planned" | "Past" {
    if (plan.date_end < todayKey) return "Past";
    if (plan.date_start <= todayKey) return "In progress";
    return "Planned";
  }

  onMount(() => {
    void (async () => {
      // Trips is an on-demand capability. Opening its native workspace is the demand;
      // sjel-status owns process control, while every data call still goes to Trips.
      try {
        await axonStatus.start("trips");
      } catch {
        // The plan request below carries the reader-facing outage if startup failed.
      }
      try {
        plans = await trips.list();
      } catch {
        plans = [];
      }
      await loadTravelCandidates();
      await loadCompanionProposals();
    })();
  });

  async function loadTravelCandidates(): Promise<void> {
    travelCandidatesLoading = true;
    travelCandidateNotice = null;
    await axonStatus.start("scouting").catch(() => undefined);

    let opportunities: ScoutingOpportunity[];
    try {
      opportunities = (await scouting.opportunities(false)).opportunities;
    } catch {
      travelCandidates = [];
      travelCandidateNotice = "Scouting travel candidates are unavailable.";
      travelCandidatesLoading = false;
      return;
    }

    const requests = calendarCandidatesFor(opportunities, plans, todayKey);
    let calendarAvailable = true;
    let verdicts = new Map<string, CalendarCandidateVerdict>();
    if (requests.length > 0) {
      await axonStatus.start("calendar").catch(() => undefined);
      try {
        const response = await calendar.verdicts(requests);
        verdicts = new Map(response.verdicts.map((verdict) => [verdict.id, verdict]));
      } catch {
        calendarAvailable = false;
        travelCandidateNotice =
          "Calendar feasibility is unavailable; candidates remain visible but are not offered as new plans.";
      }
    }

    travelCandidates = assessTravelCandidates(
      opportunities,
      plans,
      verdicts,
      todayKey,
      calendarAvailable,
    );
    travelCandidatesLoading = false;
  }

  function travelCandidateLabel(candidate: TravelCandidateAssessment): string {
    switch (candidate.state) {
      case "matching_trip": return "Matches a trip";
      case "free_window": return "Free window";
      case "needs_travel_day": return "Travel day needed";
      case "conflicts": return "Calendar says no";
      case "date_unresolved": return "Date unresolved";
      case "calendar_unavailable": return "Calendar unavailable";
    }
  }

  async function addTravelCandidate(candidate: TravelCandidateAssessment): Promise<void> {
    const match = candidate.plan_match;
    if (!match || travelCandidateSaving) return;
    travelCandidateSaving = candidate.opportunity.id;
    travelCandidateNotice = null;
    try {
      await trips.addItem(match.plan.id, {
        item_type: "event",
        day: candidate.opportunity.starts_at.slice(0, 10) || null,
        external_id: `scouting:${candidate.opportunity.id}`,
        title: candidate.opportunity.title,
        payload: {
          opportunity_id: candidate.opportunity.id,
          source: candidate.opportunity.source,
          url: candidate.opportunity.url,
          city: candidate.opportunity.city,
          location: candidate.opportunity.location,
          latitude: candidate.opportunity.latitude,
          longitude: candidate.opportunity.longitude,
          event_route: candidate.opportunity.event_route,
          destination_match: {
            destination_id: match.destination.id,
            distance_km: match.distance_km,
          },
        },
      });
      savedTravelCandidates = new Set([...savedTravelCandidates, candidate.opportunity.id]);
      travelCandidateNotice = `“${candidate.opportunity.title}” was added to ${match.plan.title}.`;
    } catch (caught) {
      travelCandidateNotice = caught instanceof Error ? caught.message : String(caught);
    } finally {
      travelCandidateSaving = null;
    }
  }

  function seedPlanFromCandidate(candidate: TravelCandidateAssessment): void {
    if (!["free_window", "needs_travel_day"].includes(candidate.state)) return;
    const opportunity = candidate.opportunity;
    const day = opportunity.starts_at.slice(0, 10);
    const end = opportunity.ends_at.slice(0, 10);
    firstDestination = {
      id: `scouting:${opportunity.id}`,
      name: opportunity.city.trim() || opportunity.location.trim() || opportunity.title,
      kind: opportunity.city.trim() ? "city" : "venue",
      address: opportunity.location.trim() || null,
      latitude: opportunity.latitude,
      longitude: opportunity.longitude,
    };
    secondDestination = null;
    showSecondDestination = false;
    startDate = day;
    endDate = end && end >= day ? end : day;
    interests = opportunity.title;
    plannerNotice = `Carried over from “${opportunity.title}”. Review the origin and route before saving.`;
    plannerOpen = true;
  }

  function openPlanner(): void {
    plannerNotice = null;
    plannerOpen = true;
  }

  function closePlanner(): void {
    if (creating) return;
    plannerOpen = false;
    plannerNotice = null;
    intentSentence = "";
    intentFeedback = null;
  }

  // Calendar materialisation names the new plan in the URL. Resolve it only
  // after the ordinary plan list has arrived, then use the same loading path
  // as a plan the operator clicked themselves.
  $effect(() => {
    const planId = page.url.searchParams.get("plan");
    if (!planId || planId === openedPlanFromLink) return;
    const plan = plans.find((candidate) => candidate.id === planId);
    if (!plan) return;
    openedPlanFromLink = planId;
    void openPlan(plan);
  });

  function uniqueEvents(
    responses: ScoredResult[][],
    from: string,
    to: string,
    anchorUrls: Set<string>,
  ): ScoredResult[] {
    const seen = new Set<string>();
    return responses
      .flat()
      .filter((event) => {
        if (seen.has(event.url) || anchorUrls.has(event.url)) return false;
        seen.add(event.url);
        const day = event.date?.slice(0, 10);
        return Boolean(day && day >= from && day <= to);
      })
      .sort((a, b) => (a.date ?? "9999").localeCompare(b.date ?? "9999"))
      .slice(0, 12);
  }

  function dayAfter(value: string): string {
    const date = new Date(`${value}T12:00:00`);
    date.setDate(date.getDate() + 1);
    return isoDate(date);
  }

  function normalizedPlace(value: string): string {
    return value
      .normalize("NFD")
      .replace(/\p{M}/gu, "")
      .replace(/ß/g, "ss")
      .toLocaleLowerCase("en-GB")
      .trim();
  }

  function calendarCity(entry: CalendarEntry): string {
    if (entry.payload && typeof entry.payload === "object" && "city" in entry.payload) {
      const city = (entry.payload as { city?: unknown }).city;
      if (typeof city === "string" && city.trim()) return city;
    }
    return entry.location ?? "";
  }

  function calendarUrl(entry: CalendarEntry): string | null {
    if (!entry.payload || typeof entry.payload !== "object" || !("url" in entry.payload)) {
      return null;
    }
    const url = (entry.payload as { url?: unknown }).url;
    return typeof url === "string" && url ? url : null;
  }

  function calendarAnchors(entries: CalendarEntry[], place: PlaceRef): CalendarEntry[] {
    const destination = normalizedPlace(placeName(place));
    return entries
      .filter((entry) => ["event", "nightlife"].includes(entry.kind))
      .filter((entry) => entry.commitment !== "possible")
      .filter((entry) => {
        const city = normalizedPlace(calendarCity(entry));
        if (!city) return false;
        return city === destination || city.includes(destination) || destination.includes(city);
      })
      .sort((a, b) => {
        const commitment = { committed: 0, planned: 1, possible: 2 };
        return commitment[a.commitment] - commitment[b.commitment]
          || a.starts_at.localeCompare(b.starts_at);
      });
  }

  function previousStop(plan: TripPlan, place: PlaceRef): PlaceRef {
    const index = plan.destinations.findIndex((destination) => destination.id === place.id);
    return index > 0 ? plan.destinations[index - 1] : plan.origin;
  }

  function stageFor(plan: TripPlan, place: PlaceRef) {
    return plan.stages.find((stage) => stage.destination.id === place.id);
  }

  /// Exactly one override, because the server refuses more than one and the
  /// refusal would arrive as a failed search rather than as a UI mistake.
  function journeyOverride(): JourneyOverride | undefined {
    const phrase = journeyPhrase.trim();
    if (phrase) return { phrase };
    if (journeyPriority) return { priority: journeyPriority };
    return undefined;
  }

  async function exploreDestination(place: PlaceRef): Promise<void> {
    const plan = activePlan;
    if (!plan) return;
    const resultForPlace = results.find((result) => result.place.id === place.id);
    const city = placeName(place);
    const routeOrigin = previousStop(plan, place);
    const stage = stageFor(plan, place);
    const stageModes = stage?.transport_modes ?? plan.transport_modes;
    const canSearchTrain =
      stageModes.includes("train") &&
      routeOrigin.kind === "station" &&
      place.kind === "station";
    const departure = `${stage?.date ?? plan.date_start}T09:00:00`;
    const [journeyResult, imageResult, activityResult, ...eventResults] =
      await Promise.allSettled([
        canSearchTrain
          ? transit.search(routeOrigin.id, place.id, departure, journeyOverride())
          : Promise.resolve<Journey[]>([]),
        loadPlaceImage(place),
        loadNearbyPlaces(place),
      ...EVENT_ADAPTERS.map((adapter) =>
        scouting.discover({
          adapter,
          location: city,
            query: plan.interests || undefined,
        }),
      ),
    ]);

    const notices = [...(resultForPlace?.notices ?? [])];
    // Sliced in the order the server returned. This used to sort by price here
    // unconditionally, which silently threw away the ranking the server had just
    // computed from the traveller's own weights -- the page making a decision it
    // could not explain on top of one it could.
    const journeys =
      journeyResult.status === "fulfilled" ? journeyResult.value.slice(0, 6) : [];
    if (journeyResult.status === "rejected") notices.push("Connections unavailable");
    if (stageModes.includes("train") && !canSearchTrain) {
      notices.push("Select the origin and destination as stations for rail options");
    }
    const pendingProviders = stageModes.filter((mode) =>
      ["flight", "car", "bus", "ferry"].includes(mode),
    );
    if (pendingProviders.length > 0) {
      notices.push(
        `${pendingProviders.map(modeLabel).join(", ")}: provider not connected yet`,
      );
    }

    const eventLists: ScoredResult[][] = [];
    for (const result of eventResults) {
      if (result.status === "fulfilled") eventLists.push(result.value.results);
    }
    if (eventLists.length === 0) notices.push("Event sources unavailable");
    const anchors = resultForPlace?.anchors ?? [];
    const anchorUrls = new Set(
      anchors.map(calendarUrl).filter((url): url is string => url !== null),
    );

    results = results.map((result) =>
      result.place.id === place.id
        ? {
            ...result,
            image: imageResult.status === "fulfilled" ? imageResult.value : null,
            journeys,
            events: uniqueEvents(eventLists, plan.date_start, plan.date_end, anchorUrls),
            activities: activityResult.status === "fulfilled" ? activityResult.value : [],
            loading: false,
            notices,
          }
        : result,
    );
  }

  async function explorePlan(plan: TripPlan): Promise<void> {
    activePlan = plan;
    mapOpen = false;
    selectedId = plan.destinations[0]?.id ?? "";
    expandedJourneyId = null;
    results = plan.destinations.map((place) => ({
      place,
      city: placeName(place),
      image: null,
      journeys: [],
      anchors: [],
      events: [],
      activities: [],
      loading: true,
      notices: [],
    }));
    // The supporting sources remain off until a current plan needs fresh results.
    // Startup failures are reflected by the source notices assembled below.
    await Promise.allSettled([
      axonStatus.start("transit"),
      axonStatus.start("scouting"),
      axonStatus.start("calendar"),
    ]);
    try {
      const entries = await calendar.entries.list(
        plan.date_start,
        dayAfter(plan.date_end),
      );
      results = results.map((result) => ({
        ...result,
        anchors: calendarAnchors(entries, result.place),
      }));
    } catch {
      results = results.map((result) => ({
        ...result,
        notices: [...result.notices, "Calendar anchors unavailable"],
      }));
    }
    await Promise.all(plan.destinations.map(exploreDestination));
  }

  async function createPlan(): Promise<void> {
    const destinations = [
      firstDestination,
      showSecondDestination ? secondDestination : null,
    ].filter((place): place is PlaceRef => place !== null);
    if (!origin || destinations.length === 0) {
      error = "Select an origin and at least one destination.";
      return;
    }
    if (transportModes.length === 0) {
      error = "Select at least one possible transport mode.";
      return;
    }
    if (startDate > endDate) {
      error = "The end date must be after the start date.";
      return;
    }

    creating = true;
    error = null;
    try {
      const title = destinations.map(placeName).join(" → ");
      const plan = await trips.create({
        title,
        origin,
        destinations,
        date_start: startDate,
        date_end: endDate,
        interests: interests.trim(),
        travelers: travelers
          .split(",")
          .map((traveler) => traveler.trim())
          .filter(Boolean),
        transport_modes: transportModes,
      });
      plans = [plan, ...plans];
      items = [];
      plannerOpen = false;
      plannerNotice = null;
      // A new plan inherits nothing from the last one that was open.
      void loadClimate(plan);
      void loadPlanCost(plan.id);
      await explorePlan(plan);
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    } finally {
      creating = false;
    }
  }

  /**
   * The plan's own cost roll-up: intent, committed and actual, per plan.
   *
   * This replaces a page-wide `finance.tripSpending()` that downloaded the whole
   * projection to read one number. `finance.tripSpending()` itself stays defined
   * in `$lib/api` — the demo has a recorded fixture for it and another consumer
   * may still want it.
   *
   * Finance being down is not a failure of this call: trips answers 200 with
   * `actuals.ok: false` and a reason, and the card prints it.
   */
  async function loadPlanCost(planId: string): Promise<void> {
    planCostRollup = null;
    try {
      planCostRollup = await planCost(planId);
    } catch {
      // Deliberate empty state — the card stays absent rather than lying.
    }
  }

  /**
   * Normals for every destination that carries a coordinate, in ONE request.
   *
   * Failure is swallowed on purpose: places being down or refusing the origin
   * must not break the trip view, so the strip simply stays absent. It never
   * degrades into an empty grid — the route distinguishes "no normals yet" from
   * "no such place" and the component prints whichever it got.
   */
  async function loadClimate(plan: TripPlan): Promise<void> {
    climate = null;
    const keys = [...plan.destinations]
      .filter(
        (place): place is PlaceRef & { latitude: number; longitude: number } =>
          typeof place.latitude === "number" && typeof place.longitude === "number",
      )
      .map((place) => ({ latitude: place.latitude, longitude: place.longitude }));
    if (keys.length === 0) return;
    try {
      await axonStatus.start("places").catch(() => undefined);
      climate = await climateFor(keys, { from: plan.date_start, to: plan.date_end });
    } catch {
      // Deliberate empty state — the strip stays absent rather than lying.
    }
  }

  async function openPlan(plan: TripPlan): Promise<void> {
    error = null;
    editingPlan = false;
    void loadPlanCost(plan.id);
    try {
      const details = await trips.get(plan.id);
      items = details.items;
      planRetrospective = details.retrospective;
      void loadWhoIsAround(details);
      void loadClimate(details);
      void loadCalendarEntries(details);
      if (details.date_end < todayKey) {
        activePlan = details;
        selectedId = details.destinations[0]?.id ?? "";
        results = [];
      } else {
        await explorePlan(details);
      }
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    }
  }

  function itemSaved(type: PlanItem["item_type"], externalId: string): boolean {
    return items.some((item) => item.item_type === type && item.external_id === externalId);
  }

  async function saveJourney(journey: Journey, place: PlaceRef): Promise<void> {
    if (!activePlan) return;
    error = null;
    try {
      const routeOrigin = previousStop(activePlan, place);
      const item = await trips.addItem(activePlan.id, {
        item_type: "transport",
        day: activePlan.date_start,
        external_id: journey.id,
        title: `${placeName(routeOrigin)} → ${placeName(place)}`,
        payload: { mode: "train", journey },
      });
      items = [...items.filter((current) => current.id !== item.id), item];
      const stage = stageFor(activePlan, place);
      if (stage) {
        await updateStage(stage.id, {
          selected_option_id: journey.id,
          status: "option_selected",
        });
      }
      savedNotice = "Connection saved to the itinerary.";
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    }
  }

  async function saveEvent(event: ScoredResult): Promise<void> {
    if (!activePlan) return;
    error = null;
    try {
      const item = await trips.addItem(activePlan.id, {
        item_type: "event",
        day: event.date?.slice(0, 10) ?? null,
        external_id: event.url,
        title: event.title,
        payload: event,
      });
      items = [...items.filter((current) => current.id !== item.id), item];
      savedNotice = "Event saved to the itinerary.";
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    }
  }

  async function saveCalendarEvent(entry: CalendarEntry): Promise<void> {
    if (!activePlan) return;
    error = null;
    try {
      const item = await trips.addItem(activePlan.id, {
        item_type: "event",
        day: entry.starts_at.slice(0, 10),
        external_id: `calendar:${entry.id}`,
        title: entry.title,
        payload: {
          calendar_entry_id: entry.id,
          commitment: entry.commitment,
          location: entry.location,
          source: entry.source,
          url: calendarUrl(entry),
        },
      });
      items = [...items.filter((current) => current.id !== item.id), item];
      savedNotice = "Calendar anchor saved to the itinerary.";
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    }
  }

  async function savePlace(place: NearbyPlace): Promise<void> {
    if (!activePlan) return;
    error = null;
    try {
      const item = await trips.addItem(activePlan.id, {
        item_type: "activity",
        day: null,
        external_id: place.id,
        title: place.title,
        payload: {
          url: place.url,
          description: place.description,
          image_url: place.imageUrl,
          latitude: place.latitude,
          longitude: place.longitude,
        },
      });
      items = [...items.filter((current) => current.id !== item.id), item];
      savedNotice = "Activity saved to the itinerary.";
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    }
  }

  async function useAsCover(image: PlaceImage): Promise<void> {
    if (!activePlan) return;
    error = null;
    try {
      const updated = await trips.update(activePlan.id, { cover_image_url: image.url });
      activePlan = updated;
      plans = plans.map((plan) => (plan.id === updated.id ? updated : plan));
      savedNotice = "Trip cover image saved.";
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    }
  }

  async function savePlanEdits(patch: Partial<TripPlan>): Promise<void> {
    if (!activePlan || savingPlan) return;
    savingPlan = true;
    error = null;
    try {
      const updated = await trips.update(activePlan.id, patch);
      plans = plans.map((plan) => (plan.id === updated.id ? updated : plan));
      editingPlan = false;
      // Both reads are about the plan that just changed: the destinations decide
      // which normals the strip shows, and the budget is edited through exactly
      // this path. Without these the strip kept the previous destinations and the
      // card kept the previous budget.
      void loadClimate(updated);
      void loadPlanCost(updated.id);
      if (updated.date_end < todayKey) {
        activePlan = updated;
        results = [];
      } else {
        await explorePlan(updated);
      }
      savedNotice = "Trip details updated. Feed context is being refreshed.";
      void comms.refreshRelevance(365).catch(() => undefined);
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    } finally {
      savingPlan = false;
    }
  }

  async function deleteActivePlan(): Promise<void> {
    if (!activePlan || deletingPlan) return;
    const deleted = activePlan;
    deletingPlan = true;
    error = null;
    try {
      await trips.delete(deleted.id);
      plans = plans.filter((plan) => plan.id !== deleted.id);
      resetPlan();
      planNotice = `“${deleted.title}” was deleted from Sjel.`;
      void comms.refreshRelevance(365).catch(() => undefined);
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    } finally {
      deletingPlan = false;
    }
  }

  async function scanObsidian(): Promise<void> {
    obsidianScanning = true;
    error = null;
    obsidianNotice = null;
    try {
      // Opening Travel normally starts Trips, but a click must also recover after the
      // service was stopped while this page stayed open.
      await axonStatus.start("trips").catch(() => undefined);
      obsidianCandidates = await trips.scanObsidian();
      const ready = obsidianCandidates.filter(
        (candidate) => candidate.issues.length === 0 && !candidate.imported_plan_id,
      ).length;
      const incomplete = obsidianCandidates.filter((candidate) => candidate.issues.length > 0).length;
      obsidianNotice = obsidianCandidates.length === 0
        ? "No Obsidian notes marked as trips were found."
        : `${obsidianCandidates.length} trips found · ${ready} importable${
            incomplete > 0 ? ` · ${incomplete} incomplete` : ""
          }`;
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    } finally {
      obsidianScanning = false;
    }
  }

  async function importObsidian(candidate: ObsidianTripCandidate): Promise<void> {
    if (!importOrigin) {
      requireImportOrigin();
      return;
    }
    obsidianImporting = candidate.reference;
    error = null;
    obsidianNotice = null;
    try {
      const plan = await trips.importObsidian(candidate.reference, importOrigin);
      plans = [plan, ...plans.filter((current) => current.id !== plan.id)];
      obsidianCandidates = obsidianCandidates.map((current) =>
        current.reference === candidate.reference
          ? { ...current, imported_plan_id: plan.id }
          : current,
      );
      obsidianNotice = `“${candidate.title}” was imported.`;
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    } finally {
      obsidianImporting = null;
    }
  }

  async function importAllObsidian(): Promise<void> {
    if (!importOrigin) {
      requireImportOrigin();
      return;
    }
    obsidianImportingAll = true;
    error = null;
    obsidianNotice = null;
    try {
      const result = await trips.importAllObsidian(importOrigin);
      const returnedPlans = [...result.imported, ...result.existing];
      const returnedIds = new Set(returnedPlans.map((plan) => plan.id));
      plans = [...returnedPlans, ...plans.filter((plan) => !returnedIds.has(plan.id))];
      const planIdByReference = new Map(
        returnedPlans
          .filter((plan) => plan.source?.kind === "obsidian")
          .map((plan) => [plan.source!.reference, plan.id]),
      );
      obsidianCandidates = obsidianCandidates.map((candidate) => ({
        ...candidate,
        imported_plan_id:
          planIdByReference.get(candidate.reference) ?? candidate.imported_plan_id,
      }));
      obsidianNotice = `${result.imported.length} trips imported${
        result.existing.length > 0 ? ` · ${result.existing.length} already present` : ""
      }${result.skipped.length > 0 ? ` · ${result.skipped.length} skipped` : ""}.`;
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    } finally {
      obsidianImportingAll = false;
    }
  }

  function requireImportOrigin(): void {
    error = null;
    importOriginNeeded = true;
    obsidianNotice = "A shared origin is required before importing.";
    queueMicrotask(() => importOriginField?.querySelector("input")?.focus());
  }

  function toggleTransportMode(mode: TransportMode): void {
    transportModes = transportModes.includes(mode)
      ? transportModes.filter((current) => current !== mode)
      : [...transportModes, mode];
  }

  function modeLabel(mode: TransportMode): string {
    return MODE_OPTIONS.find((option) => option.id === mode)?.label ?? mode;
  }

  function stageStatusLabel(status: TripPlan["stages"][number]["status"]): string {
    return {
      planning: "Review options",
      option_selected: "Option selected",
      booked: "Booked",
      completed: "Completed",
      open: "Open branch",
    }[status];
  }

  function commitmentLabel(commitment: CalendarEntry["commitment"]): string {
    return {
      possible: "Possible",
      planned: "Planned",
      committed: "Committed",
    }[commitment];
  }

  function calendarTime(entry: CalendarEntry): string {
    if (entry.all_day) return shortDate(entry.starts_at);
    return `${shortDate(entry.starts_at)} · ${entry.starts_at.slice(11, 16)}–${entry.ends_at.slice(11, 16)}`;
  }

  function dateRangeLabel(from: string, to: string): string {
    return from === to ? shortDate(from) : `${shortDate(from)} – ${shortDate(to)}`;
  }

  async function updateStage(
    stageId: string,
    patch: Partial<TripPlan["stages"][number]>,
  ): Promise<void> {
    if (!activePlan) return;
    const stages = activePlan.stages.map((stage) =>
      stage.id === stageId ? { ...stage, ...patch } : stage,
    );
    try {
      const updated = await trips.update(activePlan.id, { stages });
      activePlan = updated;
      plans = plans.map((plan) => (plan.id === updated.id ? updated : plan));
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    }
  }

  function toggleStageMode(stageId: string, mode: TransportMode): void {
    const stage = activePlan?.stages.find((candidate) => candidate.id === stageId);
    if (!stage) return;
    const transport_modes = stage.transport_modes.includes(mode)
      ? stage.transport_modes.filter((current) => current !== mode)
      : [...stage.transport_modes, mode];
    if (transport_modes.length === 0) {
      error = "A leg needs at least one possible transport mode.";
      return;
    }
    void updateStage(stageId, { transport_modes });
  }

  function itemLabel(item: PlanItem): string {
    const labels: Record<PlanItem["item_type"], string> = {
      journey: "Rail connection",
      transport: "Transport",
      event: "Event",
      activity: "Activity",
      place: "Place",
      stay: "Accommodation",
      image: "Image",
      note: "Note",
      option_set: "Options",
      booking: "Booking",
      outcome: "Outcome",
    };
    return labels[item.item_type];
  }

  function itemImage(item: PlanItem): string | null {
    if (!item.payload || typeof item.payload !== "object") return null;
    const value = (item.payload as { image_url?: unknown }).image_url;
    return typeof value === "string" && value.startsWith("https://") ? value : null;
  }

  async function removeItem(item: PlanItem): Promise<void> {
    if (!activePlan) return;
    error = null;
    try {
      await trips.deleteItem(activePlan.id, item.id);
      items = items.filter((current) => current.id !== item.id);
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    }
  }

  function resetPlan(): void {
    activePlan = null;
    items = [];
    planCalendarEntries = [];
    origin = null;
    firstDestination = null;
    secondDestination = null;
    showSecondDestination = false;
    startDate = isoDate(today);
    endDate = isoDate(nextWeek);
    interests = "";
    travelers = "";
    transportModes = ["train"];
    results = [];
    selectedId = "";
    expandedJourneyId = null;
    error = null;
    savedNotice = null;
    editingPlan = false;
    mapOpen = false;
    plannerOpen = false;
    plannerNotice = null;
  }

  function orderedJourneys(journeys: Journey[]): Journey[] {
    return [...journeys].sort((a, b) => {
      if (journeySort === "ranked") {
        // The server's order, carried through. Unranked journeys sort last rather
        // than being dropped: "the weights could not separate this" is not a
        // reason to hide it.
        return (
          (a.ranking?.rank ?? Number.POSITIVE_INFINITY) -
          (b.ranking?.rank ?? Number.POSITIVE_INFINITY)
        );
      }
      if (journeySort === "duration") {
        return a.total_duration_minutes - b.total_duration_minutes;
      }
      if (journeySort === "departure") {
        return (a.legs[0]?.departure_time ?? "").localeCompare(b.legs[0]?.departure_time ?? "");
      }
      return (
        (a.total_price ?? Number.POSITIVE_INFINITY) -
        (b.total_price ?? Number.POSITIVE_INFINITY)
      );
    });
  }

  const shortDate = (date: string | null) =>
    date
      ? new Intl.DateTimeFormat("en-GB", { day: "2-digit", month: "short" }).format(
          new Date(`${date.slice(0, 10)}T12:00:00`),
        )
      : "open";

</script>

<PageHeader
  badge="Travel"
  title="Travel hub"
  desc="Upcoming and past trips, connections, events, and your itinerary."
/>

{#if error}
  <p class="notice error"><Icon name="alert" size={16} /> {error}</p>
{/if}
{#if planNotice}
  <p class="notice success" aria-live="polite"><Icon name="check" size={16} /> {planNotice}</p>
{/if}

{#if !activePlan}
  <nav class="travel-nav" aria-label="Travel sections">
    <a class="active" href={link("/travel")}><Icon name="map-pin" size={14} /> Trip plans</a>
    <a href={link("/travel/connections")}><Icon name="train" size={14} /> Connections</a>
    <button type="button" class="nav-btn" onclick={() => (profileOpen = true)}>
      <Icon name="compass" size={14} /> Profile & preferences
    </button>
  </nav>

  <!-- The board is the page; what is merely waiting for a decision sits in the rail
       beside it, the same way /calendar keeps its review lists. -->
  <div class="hub">
    <div class="hub-main">
      <TravelHub
        {plans}
        {upcomingPlans}
        {pastPlans}
        {highlightedPlanId}
        {overviewMapSources}
        {overviewMapFitKey}
        {todayKey}
        modeOptions={MODE_OPTIONS}
        onOpenPlan={(plan) => void openPlan(plan)}
        onOpenPlanner={openPlanner}
        onHighlightPlan={(id) => (highlightedPlanId = id)}
      />

      <RelatedTools
        context="travel-planning"
        title="When another tool fits better"
        description="Plan together or collect booking confirmations automatically."
      />
    </div>

    <Rail label="Travel review and planning">
      <RailGroup title="Review" count={travelReviewCount}>
        <RailSection label="Candidates" count={travelCandidates.length}>
          <p class="rail-hint">
            Far events matched to dated trip destinations or checked against your Calendar.
          </p>

          {#if travelCandidateNotice}
            <p class="rail-notice" aria-live="polite">{travelCandidateNotice}</p>
          {/if}

          {#if travelCandidatesLoading}
            <p class="rail-empty">Checking Scouting, Trips, and Calendar…</p>
          {:else if travelCandidates.length === 0}
            <p class="rail-empty">No undismissed travel-candidate events are waiting.</p>
          {:else}
            <ol class="candidate-list">
              {#each travelCandidates as candidate (candidate.opportunity.id)}
                <li class:blocked={candidate.state === "conflicts"}>
                  <p class="candidate-when">
                    <strong>{shortDate(candidate.opportunity.starts_at.slice(0, 10))}</strong>
                    <span>{candidate.opportunity.city || candidate.opportunity.location || "Place open"}</span>
                  </p>
                  <span class="candidate-state {candidate.state}">{travelCandidateLabel(candidate)}</span>
                  <a class="candidate-title" href={candidate.opportunity.url} target="_blank" rel="noreferrer">
                    {candidate.opportunity.title}
                  </a>
                  <small use:tip={candidate.reason}>{candidate.reason}</small>
                  <div class="candidate-actions">
                    {#if candidate.plan_match}
                      <button
                        type="button"
                        disabled={travelCandidateSaving !== null || savedTravelCandidates.has(candidate.opportunity.id)}
                        onclick={() => void addTravelCandidate(candidate)}
                      >
                        <Icon
                          name={savedTravelCandidates.has(candidate.opportunity.id) ? "check" : "plus"}
                          size={13}
                        />
                        {savedTravelCandidates.has(candidate.opportunity.id)
                          ? "Added"
                          : travelCandidateSaving === candidate.opportunity.id
                            ? "Adding…"
                            : "Add to trip"}
                      </button>
                      <button type="button" onclick={() => void openPlan(candidate.plan_match!.plan)}>
                        Open trip
                      </button>
                    {:else if ["free_window", "needs_travel_day"].includes(candidate.state)}
                      <button type="button" onclick={() => seedPlanFromCandidate(candidate)}>
                        <Icon name="map-pin" size={13} /> Plan around event
                      </button>
                    {/if}
                  </div>
                </li>
              {/each}
            </ol>
          {/if}

          <button
            class="btn btn-outline rail-action"
            type="button"
            disabled={travelCandidatesLoading}
            onclick={() => void loadTravelCandidates()}
          >
            <Icon name={travelCandidatesLoading ? "loader" : "refresh"} size={13} />
            {travelCandidatesLoading ? "Checking…" : "Refresh"}
          </button>
        </RailSection>

        <RailSection label="Obsidian" count={obsidianPendingCount}>
          <p class="rail-hint">
            Scans only notes marked as trips. Everything remains unchanged until import.
          </p>

          {#if obsidianNotice}
            <p class="rail-notice" aria-live="polite">{obsidianNotice}</p>
          {/if}

          {#if obsidianCandidates.length > 0}
            <div
              class="import-origin"
              class:missing={importOriginNeeded && !importOrigin}
              bind:this={importOriginField}
            >
              <PlaceField
                label="Origin for imports"
                placeholder="Applies to the trips imported now"
                bind:place={importOrigin}
              />
              <p>
                {#if importOriginNeeded && !importOrigin}
                  Enter an origin or select one from the suggestions.
                {:else}
                  Missing origins are not guessed. You can change them later for each trip.
                {/if}
              </p>
            </div>
            <ol class="import-list">
              {#each obsidianCandidates as candidate (candidate.reference)}
                <li>
                  <div>
                    <strong>{candidate.title}</strong>
                    <span>
                      {shortDate(candidate.date_start)} – {shortDate(candidate.date_end)}
                      · {candidate.destination?.name ?? "Destination open"}
                    </span>
                    {#if candidate.issues.length > 0}
                      <small>{candidate.issues.join(" · ")}</small>
                    {:else if candidate.travelers.length > 0}
                      <small>{candidate.travelers.join(", ")}</small>
                    {/if}
                  </div>
                  {#if candidate.imported_plan_id}
                    <span class="imported"><Icon name="check" size={13} /> Imported</span>
                  {:else}
                    <button
                      type="button"
                      disabled={candidate.issues.length > 0 || obsidianImportingAll || obsidianImporting !== null}
                      onclick={() => void importObsidian(candidate)}
                    >
                      {obsidianImporting === candidate.reference ? "Importing…" : "Import"}
                    </button>
                  {/if}
                </li>
              {/each}
            </ol>
          {/if}

          <div class="rail-actions">
            <button
              class="btn btn-outline"
              type="button"
              disabled={obsidianScanning || obsidianImportingAll}
              onclick={() => void scanObsidian()}
            >
              <Icon name={obsidianScanning ? "loader" : "database"} size={13} />
              {obsidianScanning ? "Scanning…" : "Scan"}
            </button>
            {#if obsidianCandidates.length > 0}
              <button
                class="btn btn-primary"
                type="button"
                disabled={obsidianReadyCount === 0 || obsidianImportingAll || obsidianImporting !== null}
                onclick={() => void importAllObsidian()}
              >
                <Icon name={obsidianImportingAll ? "loader" : "check"} size={13} />
                {obsidianImportingAll ? "Importing…" : `Import all (${obsidianReadyCount})`}
              </button>
            {/if}
          </div>
        </RailSection>

        <RailSection label="Companions" count={companionProposals.length}>
          <CompanionRail
            proposals={companionProposals}
            loading={companionLoading}
            notice={companionNotice}
          />
        </RailSection>
      </RailGroup>
    </Rail>
  </div>

  {#if profileOpen}
    <TravelerProfileModal onClose={() => (profileOpen = false)} />
  {/if}

  <NewTripModal
    open={plannerOpen}
    bind:origin
    bind:firstDestination
    bind:secondDestination
    bind:showSecondDestination
    bind:startDate
    bind:endDate
    bind:interests
    bind:travelers
    {transportModes}
    bind:intentSentence
    {intentFeedback}
    {draftingIntent}
    {creating}
    {plannerNotice}
    modeOptions={MODE_OPTIONS}
    onClose={closePlanner}
    onCreatePlan={createPlan}
    onDraftIntent={handleDraftIntent}
    onToggleTransportMode={toggleTransportMode}
    onAdoptSearchCandidate={createPlanFromCandidate}
    onAdoptBaseCandidate={adoptBaseCandidate}
  />

{:else}
  {#if viewingPast}
    <TripPastView
      plan={activePlan}
      {items}
      retrospective={planRetrospective}
      {costProposal}
      {tripMapSources}
      {tripMapFitKey}
      onSavedRetrospective={(row) => (planRetrospective = row)}
    />
  {:else}
    <TripWorkspace
      plan={activePlan}
      bind:items
      modeOptions={MODE_OPTIONS}
      {planCostRollup}
      {planCalendarEntries}
      {knownPeople}
      {peopleNotice}
      {legCoordinates}
      {locatedByDay}
      {results}
      bind:selectedId
      {climate}
      {tripMapSources}
      {tripMapFitKey}
      bind:journeySort
      bind:journeyPriority
      bind:journeyPhrase
      bind:expandedJourneyId
      {orderedJourneys}
      {savingPlan}
      {deletingPlan}
      onBack={resetPlan}
      onSavePlanEdits={savePlanEdits}
      onDeletePlan={deleteActivePlan}
      onUpdateStage={updateStage}
      onToggleStageMode={toggleStageMode}
      onItemChanged={(updated) => (items = items.map((it) => (it.id === updated.id ? updated : it)))}
      onRemoveItem={removeItem}
      onUpdateItemDay={updateItemDay}
      onSaveCalendarEvent={saveCalendarEvent}
      onSaveJourney={saveJourney}
      onSaveEvent={saveEvent}
      onSavePlace={savePlace}
      onReloadWhoIsAround={() => (activePlan ? loadWhoIsAround(activePlan) : Promise.resolve())}
      onUseCover={useAsCover}
    />
  {/if}
{/if}

<style>
  .notice {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin: 0 0 1rem;
    padding: 0.75rem 0.9rem;
    border-radius: var(--radius-md);
    font-size: var(--text-sm);
  }

  .notice.error {
    color: var(--danger);
    background: var(--danger-soft);
  }

  .notice.success {
    color: var(--success);
    background: var(--success-soft);
  }

  .editing-hidden {
    display: none;
  }

  .travel-nav {
    display: flex;
    gap: 0.25rem;
    margin: -0.5rem 0 1.25rem;
    padding-bottom: 0.65rem;
    border-bottom: 1px solid var(--card-border);
  }

  .travel-nav a,
  .travel-nav .nav-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.45rem 0.65rem;
    border: 0;
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text-secondary);
    font: inherit;
    font-size: var(--text-xs);
    font-weight: 600;
    cursor: pointer;
    transition: background-color var(--motion-fast) ease, color var(--motion-fast) ease;
  }

  .travel-nav a:hover,
  .travel-nav a.active,
  .travel-nav .nav-btn:hover {
    color: var(--primary);
    background: var(--primary-soft);
  }

  /* The list view: the board is the page, the rail sits beside it. `minmax(0, 1fr)`
   * so a wide board shrinks rather than pushing the rail off-screen, and the
   * two-column breakpoint matches the one `$lib/rail/Rail.svelte` unsticks at. */
  .hub {
    display: grid;
    gap: 1.25rem;
    align-items: start;
  }

  .hub-main {
    min-width: 0;
  }

  /* Candidate rows in the rail: when and where, what it is, why it surfaced, then
   * what you can do about it — the reading order the calendar's inboxes use. */
  .candidate-list {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .candidate-list li {
    padding: 0.55rem 0;
    border-top: 1px solid var(--card-border);
  }

  .candidate-list li.blocked .candidate-title {
    color: var(--text-secondary);
  }

  .candidate-when {
    display: flex;
    align-items: baseline;
    gap: 0.35rem;
    margin: 0;
    min-width: 0;
  }

  .candidate-when strong {
    font-size: var(--text-xs);
  }

  .candidate-when span {
    overflow: hidden;
    color: var(--text-tertiary);
    font-size: 0.625rem;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .candidate-title {
    display: block;
    margin-top: 0.15rem;
    color: var(--text-primary);
    font-size: var(--text-sm);
    font-weight: 600;
    line-height: 1.3;
  }

  .candidate-title:hover {
    color: var(--primary);
  }

  .candidate-list small {
    display: block;
    margin-top: 0.2rem;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
    line-height: 1.4;
  }

  .candidate-state {
    display: inline-block;
    margin-top: 0.25rem;
    padding: 0.12rem 0.3rem;
    border-radius: 999px;
    background: var(--surface);
    color: var(--text-secondary);
    font-family: var(--font-mono);
    font-size: 0.5625rem;
  }

  .candidate-state.matching_trip,
  .candidate-state.free_window {
    background: var(--success-soft);
    color: var(--success);
  }

  .candidate-state.needs_travel_day {
    background: var(--warning-soft);
    color: var(--warning-ink);
  }

  .candidate-state.conflicts {
    background: var(--danger-soft);
    color: var(--danger);
  }

  .candidate-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
    margin-top: 0.45rem;
  }

  .candidate-actions button {
    display: inline-flex;
    flex: 1;
    align-items: center;
    justify-content: center;
    gap: 0.25rem;
    padding: 0.3rem 0.5rem;
    border: 0;
    border-radius: var(--radius-sm);
    background: var(--primary-soft);
    color: var(--primary);
    font: inherit;
    font-size: var(--text-2xs);
    cursor: pointer;
  }

  .candidate-actions button:disabled {
    color: var(--text-tertiary);
    cursor: default;
  }

  /* Shared by both rail sections: the line that explains the list, the line that
   * reports what just happened, the line that says nothing is waiting. */
  .rail-hint {
    margin: 0 0 0.4rem;
    color: var(--text-secondary);
    font-size: var(--text-xs);
    line-height: 1.45;
  }

  .rail-notice {
    margin: 0 0 0.4rem;
    padding: 0.4rem 0.5rem;
    border-radius: var(--radius-sm);
    background: var(--primary-soft);
    color: var(--text-secondary);
    font-size: 0.72rem;
    line-height: 1.4;
  }

  .rail-empty {
    margin: 0;
    color: var(--text-tertiary);
    font-size: 0.78rem;
  }

  .rail-action,
  .rail-actions {
    margin-top: 0.5rem;
  }

  .rail-action {
    width: 100%;
  }

  .rail-actions {
    display: flex;
    gap: 0.3rem;
  }

  .rail-action,
  .rail-actions .btn {
    flex: 1;
    padding: 0.3rem 0.5rem;
    font-size: 0.72rem;
  }

  .rail-actions .btn:disabled,
  .rail-action:disabled {
    opacity: 0.48;
    cursor: not-allowed;
  }

  .map-top-bar {
    position: absolute;
    top: 1.25rem;
    right: 1.25rem;
    z-index: 2;
  }

  .map-link-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.3rem 0.6rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text-primary);
    font-size: var(--text-2xs);
    font-weight: 600;
    text-decoration: none;
    box-shadow: 0 1px 4px rgb(0 0 0 / 12%);
    transition: background-color var(--motion-fast) ease, border-color var(--motion-fast) ease;
  }

  .map-link-btn:hover {
    background: var(--card-bg);
    border-color: var(--primary);
    color: var(--primary);
  }

  .past-map-wrap {
    position: relative;
    grid-column: 1 / -1;
  }



  .import-origin p {
    margin: 0.15rem 0 0;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  .import-origin {
    display: grid;
    gap: 0.4rem;
    margin-bottom: 0.4rem;
    padding: 0.5rem 0.6rem;
    border-radius: var(--radius-sm);
    background: var(--surface);
  }

  .import-origin.missing {
    background: var(--warning-soft);
  }

  .import-origin.missing p {
    color: var(--warning-ink);
    font-weight: 600;
  }

  .import-origin.missing :global(input) {
    border-color: var(--warning);
    box-shadow: 0 0 0 2px var(--warning-soft);
  }

  .import-list {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .import-list li {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 0.5rem;
    align-items: center;
    padding: 0.55rem 0;
    border-top: 1px solid var(--card-border);
  }

  .import-list strong,
  .import-list span,
  .import-list small {
    display: block;
  }

  .import-list strong {
    font-size: var(--text-xs);
  }

  .import-list span,
  .import-list small {
    margin-top: 0.15rem;
    color: var(--text-tertiary);
    font-size: 0.625rem;
  }

  .import-list small {
    color: var(--warning-ink);
  }

  .import-list button,
  .imported {
    padding: 0.35rem 0.5rem;
    border: 0;
    border-radius: var(--radius-sm);
    background: var(--primary-soft);
    color: var(--primary);
    font: inherit;
    font-size: var(--text-2xs);
  }

  .import-list button {
    cursor: pointer;
  }

  .import-list button:disabled {
    color: var(--text-tertiary);
    cursor: default;
  }

  .import-list .imported {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    margin: 0;
    color: var(--success);
  }

  .empty-history {
    margin: 0;
    padding: 1rem;
    border: 1px dashed var(--card-border);
    border-radius: var(--radius-md);
    color: var(--text-tertiary);
    font-size: var(--text-xs);
  }


  @media (width >= 64rem) {
    .hub {
      grid-template-columns: minmax(0, 1fr) 19rem;
    }
  }
</style>
