<script lang="ts">
  import { tip } from "$lib/tip";
  /**
   * People: the entity core's person kind (capabilities/entities, PRD Q117).
   *
   * Sjel is the system of record; a person's note in Obsidian holds the prose and is
   * linked, not copied. Fields render from the registry, so a field declared here needs no
   * code change to appear. Every value and fact is C2 and stays on this machine.
   */
  import { onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import PageHeader from "$lib/PageHeader.svelte";
  import { link } from "$lib/nav";
  import {
    entities,
    calendar,
    trips,
    type Entity,
    type EntityField,
    type EntitySource,
    type LocatedPerson,
    type CalendarEntry,
    type TripPlan,
  } from "$lib/api";
  import MapSurface from "$lib/map/MapSurface.svelte";
  import {
    PEOPLE_LAYERS,
    PEOPLE_POINTS,
    groupByPlace,
    peopleFitKey,
    peopleSources,
  } from "$lib/people/people-layers";
  import { assistantStore } from "$lib/assistant/assistant.svelte";
  import { page } from "$app/state";

  let people = $state<Entity[]>([]);
  let fields = $state<EntityField[]>([]);
  let query = $state("");
  let selectedId = $state<string | null>(page.url.searchParams.get("id"));
  let allEntries = $state<CalendarEntry[]>([]);
  let allTrips = $state<TripPlan[]>([]);
  let error = $state<string | null>(null);
  let notice = $state<string | null>(null);
  let busy = $state(false);

  $effect(() => {
    const urlId = page.url.searchParams.get("id");
    if (urlId && urlId !== selectedId) {
      selectedId = urlId;
    }
  });

  const today = new Date().toISOString().slice(0, 10);
  const selected = $derived(people.find((p) => p.id === selectedId) ?? null);
  // ─── Map: where people are on a chosen day (entities /api/located) ───
  let mapDay = $state(new Date().toISOString().slice(0, 10));
  let located = $state<LocatedPerson[]>([]);
  let onlyHosts = $state(false);
  let placeKey = $state<string | null>(null);
  const groups = $derived(groupByPlace(located, onlyHosts));
  const placeGroup = $derived(groups.find((g) => g.key === placeKey) ?? null);
  // The selected person's place is highlighted when no place is picked.
  const highlightKey = $derived(
    placeKey ?? groups.find((g) => g.people.some((p) => p.entity_id === selectedId))?.key ?? null,
  );
  const mapSources = $derived(peopleSources(groups, highlightKey));
  const unplaced = $derived(people.length - located.filter((p) => p.latitude != null).length);

  async function loadLocated(): Promise<void> {
    try {
      located = (await entities.located(mapDay)).located;
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    }
  }
  $effect(() => {
    if (/^\d{4}-\d{2}-\d{2}$/.test(mapDay)) void loadLocated();
  });

  const filtered = $derived(
    people.filter(
      (p) =>
        (!query.trim() || p.name.toLowerCase().includes(query.trim().toLowerCase())) &&
        (!placeGroup || placeGroup.people.some((here) => here.entity_id === p.id)),
    ),
  );

  /** Where a person is today: an away period covering it, else the home base that holds. */
  function whereToday(person: Entity): string | null {
    const covers = (f: Entity["facts"][number]) =>
      (!f.valid_from || f.valid_from <= today) && (!f.valid_to || f.valid_to >= today);
    const latest = (predicate: string) =>
      person.facts
        .filter((f) => f.predicate === predicate && covers(f))
        .sort((a, b) => (b.valid_from ?? "").localeCompare(a.valid_from ?? ""))[0];
    const fact = latest("away") ?? latest("home_base");
    return fact ? (fact.predicate === "away" ? `in ${fact.place}` : fact.place) : null;
  }

  async function loadConnectedContext(): Promise<void> {
    try {
      const now = new Date();
      const fromStr = new Date(now.getFullYear(), now.getMonth() - 1, 1).toISOString().slice(0, 10);
      const toStr = new Date(now.getFullYear(), now.getMonth() + 3, 1).toISOString().slice(0, 10);
      const [calRes, tripRes] = await Promise.allSettled([
        calendar.entries.list(fromStr, toStr),
        trips.list(),
      ]);
      if (calRes.status === "fulfilled") allEntries = calRes.value;
      if (tripRes.status === "fulfilled") allTrips = tripRes.value;
    } catch {
      // Graceful fallback
    }
  }

  const personCalendarEntries = $derived.by(() => {
    if (!selected) return [];
    const nameLower = selected.name.toLowerCase();
    const firstName = nameLower.split(/\s+/)[0];
    return allEntries.filter((e) => {
      const titleLower = e.title.toLowerCase();
      const notesLower = (e.notes ?? "").toLowerCase();
      return (
        titleLower.includes(nameLower) ||
        notesLower.includes(nameLower) ||
        (firstName && firstName.length > 2 && (titleLower.includes(` ${firstName} `) || titleLower.endsWith(` ${firstName}`) || titleLower.startsWith(`${firstName} `)))
      );
    });
  });

  const personTrips = $derived.by(() => {
    if (!selected) return [];
    const nameLower = selected.name.toLowerCase();
    const personPlaces = selected.facts.map((f) => f.place.toLowerCase());
    return allTrips.filter((t) => {
      const titleLower = t.title.toLowerCase();
      const destinations = t.destinations?.map((d) => d.name.toLowerCase()) ?? [];
      const matchesPlace = personPlaces.some((p) => destinations.some((d) => d.includes(p) || p.includes(d)));
      const matchesTitle = titleLower.includes(nameLower);
      return matchesTitle || matchesPlace;
    });
  });

  const selectedBirthdayInfo = $derived.by(() => {
    if (!selected) return null;
    const bval = selected.values?.birthday?.value;
    if (typeof bval !== "string" || !/^\d{4}-\d{2}-\d{2}$/.test(bval)) return null;
    const monthDay = bval.slice(5);
    const now = new Date();
    const currentYear = now.getFullYear();
    const thisYearDate = new Date(`${currentYear}-${monthDay}T00:00:00`);
    let targetDate = thisYearDate;
    if (thisYearDate.getTime() < now.getTime() - 86400000) {
      targetDate = new Date(`${currentYear + 1}-${monthDay}T00:00:00`);
    }
    const diffDays = Math.round((targetDate.getTime() - now.getTime()) / 86400000);
    const birthYear = parseInt(bval.slice(0, 4), 10);
    const age = !isNaN(birthYear) ? targetDate.getFullYear() - birthYear : undefined;
    return {
      date: bval,
      monthDay,
      diffDays,
      age,
      isToday: diffDays === 0,
      isTomorrow: diffDays === 1,
    };
  });

  async function load(): Promise<void> {
    try {
      [people, fields] = await Promise.all([entities.list("person"), entities.fields("person")]);
      error = null;
      void loadConnectedContext();
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    }
  }

  onMount(() => void load());

  function replace(updated: Entity): void {
    people = people.map((p) => (p.id === updated.id ? updated : p));
  }

  async function run(action: () => Promise<void>, done?: string): Promise<void> {
    busy = true;
    error = null;
    notice = null;
    try {
      await action();
      if (done) notice = done;
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    } finally {
      busy = false;
    }
  }

  // ─── Add a person ───
  let newName = $state("");
  function addPerson(event: SubmitEvent): void {
    event.preventDefault();
    const name = newName.trim();
    if (!name) return;
    void run(async () => {
      const created = await entities.create({ kind: "person", name });
      people = [...people, created].sort((a, b) => a.name.localeCompare(b.name));
      selectedId = created.id;
      newName = "";
    });
  }

  // ─── Field values: a draft per selected person, saved in one PATCH ───
  let draft = $state<Record<string, string>>({});
  $effect(() => {
    const person = selected;
    const next: Record<string, string> = {};
    for (const field of fields) {
      const value = person?.values[field.key]?.value;
      next[field.key] = Array.isArray(value) ? value.join(", ") : value == null ? "" : String(value);
    }
    draft = next;
  });

  /** A draft string as the value its field type takes; null clears. */
  function parsed(field: EntityField, raw: string): unknown {
    const text = raw.trim();
    if (!text) return null;
    if (field.field_type === "emails" || field.field_type === "phones" || field.field_type === "tags") {
      return text.split(",").map((s) => s.trim()).filter(Boolean);
    }
    if (field.field_type === "bool") return text === "true";
    if (field.field_type === "number") return Number(text);
    return text;
  }

  function saveFields(event: SubmitEvent): void {
    event.preventDefault();
    const person = selected;
    if (!person) return;
    const values: Record<string, unknown> = {};
    for (const field of fields) {
      const before = person.values[field.key]?.value;
      const beforeText = Array.isArray(before) ? before.join(", ") : before == null ? "" : String(before);
      if ((draft[field.key] ?? "") !== beforeText) values[field.key] = parsed(field, draft[field.key] ?? "");
    }
    if (Object.keys(values).length === 0) return;
    void run(async () => {
      replace(await entities.patch(person.id, { values, expected_revision: person.revision }));
    }, "Saved.");
  }

  // ─── Dated facts ───
  let factPredicate = $state<"home_base" | "away">("away");
  let factPlace = $state("");
  let factFrom = $state("");
  let factTo = $state("");
  let factNote = $state("");
  function addFact(event: SubmitEvent): void {
    event.preventDefault();
    const person = selected;
    if (!person || !factPlace.trim()) return;
    void run(async () => {
      const added = await entities.addFact(person.id, {
        predicate: factPredicate,
        place: factPlace.trim(),
        valid_from: factFrom || undefined,
        valid_to: factTo || undefined,
        note: factNote.trim() || undefined,
      });
      replace(await entities.get(person.id));
      void loadLocated();
      factPlace = "";
      factFrom = "";
      factTo = "";
      factNote = "";
      if (added.geocode.status !== "found") {
        notice = `Saved, but no coordinate for "${added.fact.place}" (${added.geocode.status}). It will not match any trip leg.`;
      }
    });
  }

  // ─── Name and delete ───
  let nameDraft = $state("");
  let confirmDelete = $state(false);
  $effect(() => {
    nameDraft = selected?.name ?? "";
    confirmDelete = false;
    sources = null;
  });

  function rename(): void {
    const person = selected;
    const name = nameDraft.trim();
    if (!person || !name || name === person.name) return;
    void run(async () => {
      replace(await entities.patch(person.id, { name, expected_revision: person.revision }));
      people = [...people].sort((a, b) => a.name.localeCompare(b.name));
    });
  }

  function removePerson(): void {
    const person = selected;
    if (!person) return;
    if (!confirmDelete) {
      confirmDelete = true;
      return;
    }
    void run(async () => {
      await entities.remove(person.id);
      people = people.filter((p) => p.id !== person.id);
      selectedId = null;
      void loadLocated();
    }, `Deleted ${person.name}. A sync will not bring them back.`);
  }

  // ─── Compare with sources: what Google and the note say now ───
  let sources = $state<EntitySource[] | null>(null);
  let sourcesLoading = $state(false);

  async function loadSources(): Promise<void> {
    const person = selected;
    if (!person) return;
    sourcesLoading = true;
    try {
      sources = await entities.sources(person.id);
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    } finally {
      sourcesLoading = false;
    }
  }

  const label = (key: string) => fields.find((f) => f.key === key)?.label ?? key;
  const show = (v: unknown) => (Array.isArray(v) ? v.join(", ") : v == null ? "–" : String(v));

  /** Source values that differ from what Sjel holds, one row per system and key. */
  const differences = $derived.by(() => {
    const person = selected;
    if (!person || !sources) return [];
    const rows: { system: EntitySource["system"]; key: string; value: unknown }[] = [];
    for (const source of sources) {
      for (const [key, value] of Object.entries(source.values ?? {})) {
        if (JSON.stringify(person.values[key]?.value) !== JSON.stringify(value)) {
          rows.push({ system: source.system, key, value });
        }
      }
    }
    return rows;
  });

  /** Takes a source's value, owned by that source again, so its sync keeps it current. */
  function useSource(system: EntitySource["system"], key: string, value: unknown): void {
    const person = selected;
    if (!person) return;
    void run(async () => {
      replace(await entities.patch(person.id, { values: { [key]: value }, source: system }));
    }, `${label(key)} taken from ${system === "google" ? "Google" : "the note"}.`);
  }

  const sourceName: Record<string, string> = { google: "Google", obsidian: "note", operator: "you" };

  function removeFact(factId: string): void {
    const person = selected;
    if (!person) return;
    void run(async () => {
      await entities.removeFact(person.id, factId);
      replace(await entities.get(person.id));
      void loadLocated();
    });
  }

  // ─── Declare a field ───
  let fieldLabel = $state("");
  let fieldType = $state<EntityField["field_type"]>("text");
  let fieldOptions = $state("");
  const keyOf = (label: string) =>
    label.trim().toLowerCase().normalize("NFKD").replace(/[^a-z0-9]+/g, "_").replace(/^_+|_+$/g, "").replace(/^(\d)/, "f_$1").slice(0, 40);
  function declareField(event: SubmitEvent): void {
    event.preventDefault();
    const label = fieldLabel.trim();
    if (!label) return;
    void run(async () => {
      const field = await entities.declareField({
        kind: "person",
        key: keyOf(label),
        label,
        field_type: fieldType,
        options: fieldType === "enum" ? fieldOptions.split(",").map((o) => o.trim()).filter(Boolean) : [],
        data_class: "C2",
      });
      fields = [...fields, field];
      fieldLabel = "";
      fieldOptions = "";
    }, `Field "${label}" added.`);
  }

  function period(fact: Entity["facts"][number]): string {
    if (fact.predicate === "home_base") return fact.valid_from ? `since ${fact.valid_from}` : "home base";
    return `${fact.valid_from} – ${fact.valid_to}`;
  }
</script>

<PageHeader badge="People" title="People you know" desc="Where they live, where they are, and where you could stay. Stored in Sjel; notes stay in Obsidian." />

<p class="toolbar">
  <button
    class="btn btn-outline"
    type="button"
    onclick={() => {
      assistantStore.openDrawer();
      void assistantStore.send("Find duplicates", page.url.pathname).then(load);
    }}
  >
    <Icon name="users" size={13} /> Find duplicates
  </button>
</p>

{#if error}<p class="error"><Icon name="alert" size={15} /> {error}</p>{/if}
{#if notice}<p class="notice" aria-live="polite">{notice}</p>{/if}

<section class="map-card" aria-label="Map of people">
  <div class="map-controls">
    <label>Where on <input type="date" aria-label="Day" bind:value={mapDay} /></label>
    <label class="check"><input type="checkbox" bind:checked={onlyHosts} /> Only people I could stay with</label>
    <span class="meta">
      {groups.reduce((n, g) => n + g.people.length, 0)} on the map · {unplaced} without a place
    </span>
    {#if placeGroup}
      <button class="chip" type="button" onclick={() => (placeKey = null)}>
        {placeGroup.place} · {placeGroup.people.length} <Icon name="close" size={11} />
      </button>
    {/if}
    <a
      class="chip master-map-link"
      href={placeGroup ? link(`/map?city=${encodeURIComponent(placeGroup.place)}`) : link("/map")}
      use:tip={"Open Master Life Map"}
    >
      <Icon name="globe" size={12} />
      <span>{placeGroup ? `Explore ${placeGroup.place} on Master Map` : "Master Life Map"}</span>
    </a>
  </div>
  <div class="map">
    <MapSurface
      sources={mapSources}
      layers={PEOPLE_LAYERS}
      fitKey={peopleFitKey(groups)}
      interactive={[PEOPLE_POINTS]}
      onFeatureClick={(_, feature) => {
        const key = feature.properties?.key;
        if (typeof key === "string") placeKey = placeKey === key ? null : key;
      }}
      deferredLabel={`${groups.length} ${groups.length === 1 ? "place" : "places"} with people`}
    />
  </div>
</section>

<div class="people">
  <section class="list" aria-label="People">
    <input class="search" aria-label="Search people" placeholder="Search" bind:value={query} />
    <ol>
      {#each filtered as person (person.id)}
        <li>
          <button type="button" class:active={person.id === selectedId} onclick={() => (selectedId = person.id)}>
            <strong>{person.name}</strong>
            <span>{whereToday(person) ?? ""}</span>
          </button>
        </li>
      {/each}
    </ol>
    <form class="row" onsubmit={addPerson}>
      <input aria-label="New person's name" placeholder="New person" bind:value={newName} />
      <button class="btn btn-outline" type="submit" disabled={busy}><Icon name="plus" size={13} /> Add</button>
    </form>
  </section>

  <section class="detail" aria-label="Person">
    {#if !selected}
      <p class="empty">Pick a person, or add one.</p>
    {:else}
      <header>
        <div class="name-row">
          <input
            class="name-input"
            aria-label="Name"
            bind:value={nameDraft}
            onblur={rename}
            onkeydown={(e) => e.key === "Enter" && rename()}
          />
          <button class="btn btn-outline" type="button" disabled={busy || sourcesLoading} onclick={() => void loadSources()}>
            {sourcesLoading ? "Asking sources…" : "Compare with sources"}
          </button>
          <button class="btn btn-outline danger" type="button" disabled={busy} onclick={removePerson}>
            {confirmDelete ? "Really delete?" : "Delete"}
          </button>
        </div>
        <p class="meta">
          {whereToday(selected) ? `Today: ${whereToday(selected)}` : "No home base yet"}
          {#if selected.note_ref} · note: {selected.note_ref}{/if}
        </p>
      </header>

      <!-- Connected Life Context Card -->
      <div class="life-context-card">
        <div class="life-context-header">
          <span class="context-label">
            <Icon name="sparkles" size={13} />
            <strong>Connected Life Context</strong>
          </span>
          {#if selectedBirthdayInfo}
            <span class="birthday-badge" class:birthday-soon={selectedBirthdayInfo.diffDays <= 7}>
              🎂 {selectedBirthdayInfo.isToday ? "Birthday today!" : selectedBirthdayInfo.isTomorrow ? "Birthday tomorrow!" : `Birthday in ${selectedBirthdayInfo.diffDays} days`}
              {#if selectedBirthdayInfo.age}(turns {selectedBirthdayInfo.age}){/if}
            </span>
          {/if}
        </div>

        {#if whereToday(selected)}
          <div class="context-row">
            <span class="context-icon"><Icon name="map-pin" size={13} /></span>
            <span class="context-text">Located {whereToday(selected)}</span>
            <div class="context-row-actions">
              <a class="btn btn-soft btn-xs" href={link(`/map?person=${encodeURIComponent(selected.name)}`)}>
                <Icon name="globe" size={11} /> View on Map
              </a>
              <a class="btn btn-soft btn-xs" href={link('/travel/connections')}>
                <Icon name="train" size={11} /> Check Trains
              </a>
            </div>
          </div>
        {/if}

        <div class="context-section">
          <div class="context-section-title">
            <Icon name="calendar" size={12} />
            <span>Shared Schedule ({personCalendarEntries.length})</span>
            <a class="context-action-link" href={link('/calendar')}>+ Schedule Event</a>
          </div>
          {#if personCalendarEntries.length === 0}
            <p class="context-empty">No calendar entries referencing {selected.name}.</p>
          {:else}
            <ul class="context-list">
              {#each personCalendarEntries.slice(0, 4) as entry (entry.id)}
                <li>
                  <a class="context-list-item" href={link('/calendar')}>
                    <span class="item-time mono">{entry.starts_at.slice(0, 10)}</span>
                    <span class="item-name">{entry.title}</span>
                    {#if entry.location}<span class="item-meta">· {entry.location}</span>{/if}
                  </a>
                </li>
              {/each}
            </ul>
          {/if}
        </div>

        {#if personTrips.length > 0}
          <div class="context-section">
            <div class="context-section-title">
              <Icon name="train" size={12} />
              <span>Related Trips ({personTrips.length})</span>
            </div>
            <ul class="context-list">
              {#each personTrips.slice(0, 3) as trip (trip.id)}
                <li>
                  <a class="context-list-item" href={link('/travel')}>
                    <span class="item-time mono">{trip.date_start}</span>
                    <span class="item-name">{trip.title}</span>
                    {#if trip.destinations && trip.destinations.length > 0}
                      <span class="item-meta">→ {trip.destinations.map(d => d.name).join(', ')}</span>
                    {/if}
                  </a>
                </li>
              {/each}
            </ul>
          </div>
        {/if}
      </div>

      {#if sources}
        <div class="sources">
          {#each sources.filter((s) => s.error) as failed (failed.system)}
            <p class="meta">{failed.system}: {failed.error}</p>
          {/each}
          {#if differences.length === 0}
            <p class="meta">Google and the note agree with what Sjel has.</p>
          {:else}
            <p class="meta">Where a source says something else:</p>
            <ol>
              {#each differences as row (row.system + row.key)}
                <li>
                  <span class="field">{label(row.key)}</span>
                  <span>Sjel: {show(selected.values[row.key]?.value)}</span>
                  <span>{row.system === "google" ? "Google" : "Note"}: {show(row.value)}</span>
                  <button class="btn btn-soft btn-sm" type="button" disabled={busy} onclick={() => useSource(row.system, row.key, row.value)}>
                    Use this
                  </button>
                </li>
              {/each}
            </ol>
          {/if}
        </div>
      {/if}

      <h3>Where</h3>
      {#if selected.facts.length === 0}
        <p class="empty">No home base or away periods yet.</p>
      {:else}
        <ol class="facts">
          {#each selected.facts as fact (fact.id)}
            <li>
              <span class="tag">{fact.predicate === "away" ? "Away" : "Home"}</span>
              <strong>{fact.place}</strong>
              <span class="meta">{period(fact)}{fact.note ? ` · ${fact.note}` : ""}{fact.latitude == null ? " · no coordinate" : ""}</span>
              <button class="link" type="button" aria-label={`Delete ${fact.place}`} onclick={() => removeFact(fact.id)}>
                <Icon name="close" size={12} />
              </button>
            </li>
          {/each}
        </ol>
      {/if}
      <form class="fact-form" onsubmit={addFact}>
        <select aria-label="Kind of fact" bind:value={factPredicate}>
          <option value="away">Away</option>
          <option value="home_base">Home base</option>
        </select>
        <input aria-label="Place" placeholder="City" bind:value={factPlace} required />
        <input aria-label="From" type="date" bind:value={factFrom} required={factPredicate === "away"} />
        <input aria-label="To" type="date" bind:value={factTo} required={factPredicate === "away"} disabled={factPredicate === "home_base"} />
        <input aria-label="Note" placeholder="Note" bind:value={factNote} />
        <button class="btn btn-outline" type="submit" disabled={busy}>Add</button>
      </form>

      <h3>Details</h3>
      <form class="fields" onsubmit={saveFields}>
        {#each fields as field (field.key)}
          <label>
            <span>
              {field.label}
              {#if selected.values[field.key]}<small class="src">from {sourceName[selected.values[field.key].source] ?? selected.values[field.key].source}</small>{/if}
            </span>
            {#if field.field_type === "enum"}
              <select aria-label={field.label} bind:value={draft[field.key]}>
                <option value="">–</option>
                {#each field.options as option (option)}<option value={option}>{option}</option>{/each}
              </select>
            {:else if field.field_type === "bool"}
              <select aria-label={field.label} bind:value={draft[field.key]}>
                <option value="">–</option><option value="true">yes</option><option value="false">no</option>
              </select>
            {:else}
              <input
                aria-label={field.label}
                type={field.field_type === "date" ? "date" : field.field_type === "number" ? "number" : "text"}
                placeholder={["emails", "phones", "tags"].includes(field.field_type) ? "comma-separated" : ""}
                bind:value={draft[field.key]}
              />
            {/if}
          </label>
        {/each}
        <button class="btn btn-primary" type="submit" disabled={busy}>Save details</button>
      </form>

      <details class="declare">
        <summary>Add a field for everyone</summary>
        <form class="fact-form" onsubmit={declareField}>
          <input aria-label="Field label" placeholder="Label, e.g. Climbing grade" bind:value={fieldLabel} required />
          <select aria-label="Field type" bind:value={fieldType}>
            {#each ["text", "tags", "enum", "bool", "date", "number", "url", "emails", "phones"] as type (type)}
              <option value={type}>{type}</option>
            {/each}
          </select>
          {#if fieldType === "enum"}
            <input aria-label="Options" placeholder="Options, comma-separated" bind:value={fieldOptions} required />
          {/if}
          <button class="btn btn-outline" type="submit" disabled={busy}>Add field</button>
        </form>
      </details>
    {/if}
  </section>
</div>

<style>
  .people {
    display: grid;
    grid-template-columns: minmax(14rem, 20rem) 1fr;
    gap: 1rem;
    align-items: start;
  }

  @media (max-width: 760px) {
    .people {
      grid-template-columns: 1fr;
    }
  }

  .list,
  .detail {
    padding: 0.9rem 1rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius);
    background: var(--card-bg);
  }

  .list ol,
  .facts {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    margin: 0.6rem 0;
    padding: 0;
    list-style: none;
  }

  .list ol {
    max-height: 60vh;
    overflow-y: auto;
  }

  .list li button {
    display: flex;
    width: 100%;
    justify-content: space-between;
    gap: 0.5rem;
    padding: 0.35rem 0.5rem;
    border: 0;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--text-primary);
    font: inherit;
    font-size: var(--text-sm);
    text-align: left;
    cursor: pointer;
  }

  .list li button span {
    color: var(--text-tertiary);
    font-size: var(--text-xs);
  }

  .list li button.active,
  .list li button:hover {
    background: var(--primary-soft);
  }

  input,
  select {
    padding: 0.35rem 0.5rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    background: var(--card-bg);
    color: var(--text-primary);
    font: inherit;
    font-size: var(--text-sm);
  }

  .search {
    width: 100%;
  }

  .row {
    display: flex;
    gap: 0.4rem;
  }

  .row input {
    flex: 1;
    min-width: 0;
  }


  h3 {
    margin: 1rem 0 0.3rem;
    font-size: var(--text-sm);
    color: var(--text-secondary);
  }

  .meta,
  .empty {
    margin: 0.2rem 0;
    color: var(--text-tertiary);
    font-size: var(--text-xs);
  }

  .facts li {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0.45rem;
    font-size: var(--text-sm);
  }

  .tag {
    padding: 0 0.35rem;
    border-radius: var(--radius-sm);
    background: var(--primary-soft);
    color: var(--primary);
    font-size: var(--text-2xs);
  }

  .link {
    margin-left: auto;
    border: 0;
    background: none;
    color: var(--text-tertiary);
    cursor: pointer;
  }

  .fact-form {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    margin-top: 0.4rem;
  }

  .fields {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(14rem, 1fr));
    gap: 0.6rem;
  }

  .fields label {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    font-size: var(--text-xs);
    color: var(--text-secondary);
  }

  .fields button {
    align-self: end;
  }

  .declare {
    margin-top: 1rem;
    font-size: var(--text-sm);
  }

  .map-card {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    margin-bottom: 1rem;
  }

  .map {
    height: 22rem;
  }

  @media (max-width: 760px) {
    .map {
      height: 16rem;
    }
  }

  .map-controls {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.6rem 1rem;
    font-size: var(--text-sm);
  }

  .map-controls label {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    padding: 0.15rem 0.5rem;
    border: 1px solid var(--primary);
    border-radius: var(--radius-full);
    background: var(--primary-soft);
    color: var(--primary);
    font: inherit;
    font-size: var(--text-xs);
    cursor: pointer;
    text-decoration: none;
  }

  .master-map-link {
    margin-left: auto;
    text-decoration: none;
    transition: background-color var(--motion-fast) ease, color var(--motion-fast) ease;
  }

  .master-map-link:hover {
    background: var(--primary);
    color: var(--text-inverse);
  }

  .name-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem;
  }

  .name-input {
    flex: 1;
    min-width: 12rem;
    font-size: var(--text-lg);
    font-weight: 600;
  }

  .danger {
    color: var(--danger);
  }

  .sources {
    margin-top: 0.6rem;
    padding: 0.6rem 0.75rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    background: var(--surface);
  }

  .sources ol {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    margin: 0.3rem 0 0;
    padding: 0;
    list-style: none;
  }

  .sources li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.3rem 0.75rem;
    font-size: var(--text-xs);
  }

  .sources .field {
    font-weight: 600;
  }

  .src {
    margin-left: 0.3rem;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
    font-weight: 400;
  }

  .toolbar {
    margin: 0 0 0.75rem;
  }

  .error,
  .notice {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: var(--text-sm);
  }

  .error {
    color: var(--danger);
  }

  .notice {
    color: var(--text-secondary);
  }

  .life-context-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-md);
    background-color: var(--surface);
    border: 1px solid var(--card-border);
    margin-bottom: var(--space-4);
  }

  .life-context-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .context-label {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    font-size: var(--text-xs);
    color: var(--primary);
  }

  .birthday-badge {
    font-size: var(--text-2xs);
    font-weight: 600;
    color: var(--text-secondary);
    background-color: var(--card-bg);
    padding: 0.15rem 0.5rem;
    border-radius: var(--radius-sm);
    border: 1px solid var(--card-border);
  }

  .birthday-soon {
    color: var(--warning-ink);
    background-color: var(--primary-soft);
    border-color: var(--primary);
  }

  .context-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-xs);
    color: var(--text-secondary);
  }

  .context-icon {
    color: var(--primary);
    display: grid;
    place-items: center;
  }

  .context-text {
    font-weight: 500;
  }

  .context-row-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-left: auto;
  }

  .btn-xs {
    padding: 0.15rem 0.45rem;
    font-size: var(--text-2xs);
  }

  .context-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding-top: var(--space-2);
    border-top: 1px solid var(--card-border);
  }

  .context-section-title {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    font-size: var(--text-2xs);
    font-weight: 600;
    color: var(--text-tertiary);
  }

  .context-action-link {
    margin-left: auto;
    font-size: var(--text-2xs);
    color: var(--primary);
    text-decoration: none;
  }

  .context-action-link:hover {
    text-decoration: underline;
  }

  .context-empty {
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
    margin: 0;
  }

  .context-list {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
  }

  .context-list-item {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 0.2rem 0.4rem;
    border-radius: var(--radius-sm);
    text-decoration: none;
    color: inherit;
    font-size: var(--text-xs);
    transition: background-color var(--motion-fast) ease;
  }

  .context-list-item:hover {
    background-color: var(--card-bg);
  }

  .item-time {
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
  }

  .item-name {
    font-weight: 500;
    color: var(--text-primary);
  }

  .item-meta {
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
