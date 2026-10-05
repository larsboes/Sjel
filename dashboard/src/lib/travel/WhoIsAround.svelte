<script lang="ts">
  import { tip } from "$lib/tip";
  /**
   * Who you know near each leg, where you could stay, and the plan's meetups.
   *
   * People come from capabilities/entities (PRD Q117): where each person is on the leg's
   * day, from their home base or an away period. The join runs here, on this machine
   * (who-is-around.ts).
   */
  import { link } from "$lib/nav";
  import Icon from "$lib/Icon.svelte";
  import { entities, type Entity, type LocatedPerson, type PlanItem, type TripStage } from "$lib/api";
  import { AROUND_RADIUS_KM, aroundFromLocated, meetupsOf, staysOf } from "$lib/travel/who-is-around";
  import { inspectorStore } from "$lib/inspector/inspector.svelte";

  let {
    stages,
    items,
    coordinates,
    locatedByDay,
    people = [],
    notice = null,
    onChanged,
  }: {
    stages: TripStage[];
    items: PlanItem[];
    /** Each leg's destination coordinate, by stage id; null when none resolved. */
    coordinates: Record<string, [number, number] | null>;
    /** entities' /api/located answer, by day. */
    locatedByDay: Record<string, LocatedPerson[]>;
    /** Every person, for the form's name list. */
    people?: Entity[];
    notice?: string | null;
    /** Called after a fact is saved, so the page reloads. */
    onChanged?: () => void;
  } = $props();

  const meetups = $derived(meetupsOf(items));
  const stays = $derived(staysOf(items));
  const legs = $derived(
    stages.map((stage) => {
      const located = stage.date ? (locatedByDay[stage.date] ?? []) : [];
      return {
        stage,
        resolved: coordinates[stage.id] != null,
        ...aroundFromLocated(coordinates[stage.id] ?? null, located),
      };
    }),
  );

  const personFor = (name: string): Entity | undefined =>
    people.find((p) => p.name.toLowerCase() === name.toLowerCase());

  function inspectPersonByName(name: string) {
    const p = personFor(name);
    if (p) {
      const home = p.facts?.find((f) => f.predicate === "home_base")?.place;
      const away = p.facts?.find((f) => f.predicate === "away")?.place;
      const loc = away ? `${away} (away)` : home;
      inspectorStore.inspectPerson({
        id: p.id,
        name: p.name,
        location: loc,
        relationship: (p.values?.relationship?.value as string) ?? undefined,
        notes: p.note_ref ?? undefined,
      });
    } else {
      inspectorStore.inspectPerson({
        id: name,
        name,
      });
    }
  }

  // "Where is someone": an away period (with dates) or a home base (without), written to
  // entities. A name not yet known creates the person.
  let formPerson = $state("");
  let formCity = $state("");
  let formFrom = $state("");
  let formTo = $state("");
  let formBusy = $state(false);
  let formMessage = $state<string | null>(null);

  async function statePlace(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    const name = formPerson.trim();
    const city = formCity.trim();
    if (!name || !city) return;
    formBusy = true;
    formMessage = null;
    try {
      const known = people.find((p) => p.name.toLowerCase() === name.toLowerCase());
      const person = known ?? (await entities.create({ kind: "person", name }));
      const away = Boolean(formFrom && formTo);
      const saved = await entities.addFact(person.id, {
        predicate: away ? "away" : "home_base",
        place: city,
        valid_from: formFrom || undefined,
        valid_to: away ? formTo : undefined,
      });
      formMessage =
        `Saved: ${person.name} ${away ? `in ${city} ${formFrom} – ${formTo}` : `lives in ${city}`}.` +
        (saved.geocode.status === "found" ? "" : ` No coordinate found, so no leg will match it.`);
      formCity = "";
      formFrom = "";
      formTo = "";
      onChanged?.();
    } catch (caught) {
      formMessage = caught instanceof Error ? caught.message : String(caught);
    } finally {
      formBusy = false;
    }
  }
</script>

<p class="rail-hint">
  People within {AROUND_RADIUS_KM} km of each leg's destination on its day, where you could
  stay, and the meetups on this plan.
</p>

{#if notice}
  <p class="rail-notice" aria-live="polite">{notice}</p>
{/if}

{#if meetups.length > 0}
  <ol class="around-list">
    {#each meetups as meetup (meetup.itemId)}
      <li>
        <p class="who">
          <strong>
            {#each meetup.people as p, i (p)}
              {#if i > 0}, {/if}
              <button
                type="button"
                class="person-chip-btn"
                onclick={() => inspectPersonByName(p)}
                use:tip={`Inspect ${p}`}
              >
                {p}
              </button>
            {/each}
          </strong>
          <span>{meetup.title}</span>
        </p>
        <p class="meta">
          {meetup.status} · {meetup.day ?? "no day yet"}{meetup.place ? ` · ${meetup.place}` : ""}
        </p>
      </li>
    {/each}
  </ol>
{/if}

<ol class="around-list">
  {#each legs as leg (leg.stage.id)}
    <li>
      <p class="who">
        <strong>{leg.stage.destination.name}</strong>
        <span>{leg.stage.date ?? "no date"}</span>
      </p>
      {#if !leg.stage.date}
        <p class="meta">No date yet, so nobody can be placed on this leg.</p>
      {:else if !leg.resolved}
        <p class="meta">No coordinate for this destination, so nobody is matched.</p>
      {:else if leg.around.length === 0}
        <p class="meta">Nobody you know is nearby that day.</p>
      {:else}
        <p class="when">
          {#each leg.around as p, i (p.person)}
            {#if i > 0}, {/if}
            <button
              type="button"
              class="person-chip-btn"
              onclick={() => inspectPersonByName(p.person)}
              use:tip={`Inspect ${p.person}`}
            >
              {p.person}
            </button>
            <span class="meta">({p.distanceKm.toFixed(0)} km)</span>
            {#if leg.stage.date}
              <a
                class="meetup-action-link"
                href={link(
                  `/calendar?date=${encodeURIComponent(leg.stage.date)}&title=${encodeURIComponent(`Meetup with ${p.person}`)}&location=${encodeURIComponent(leg.stage.destination.name)}`,
                )}
                use:tip={"Schedule calendar meetup"}
              >
                + Meetup
              </a>
            {/if}
          {/each}
        </p>
      {/if}
      {#if leg.hosts.length > 0}
        <p class="stay-line">
          <Icon name="home" size={12} /> Could stay with:
          {#each leg.hosts as h, i (h.person)}
            {#if i > 0}, {/if}
            <button
              type="button"
              class="person-chip-btn"
              onclick={() => inspectPersonByName(h.person)}
              use:tip={`Inspect host ${h.person}`}
            >
              {h.person}
            </button>
            {#if h.note}<span class="meta">({h.note})</span>{/if}
          {/each}
        </p>
      {/if}
    </li>
  {/each}
</ol>

{#if stays.length > 0}
  <p class="rail-hint">Stays on this plan</p>
  <ol class="around-list">
    {#each stays as stay (stay.title + stay.checkIn)}
      <li>
        <p class="who"><strong>{stay.title}</strong></p>
        <p class="meta">{stay.checkIn ?? "?"} – {stay.checkOut ?? "?"}</p>
      </li>
    {/each}
  </ol>
{/if}

<!-- An occasional write beside a list that is read on every visit. Folded since 2026-10-05:
     open, the four fields were the tallest thing in the section. -->
<details class="state-add">
<summary>Where is someone?</summary>
<form class="state-form" onsubmit={statePlace}>
  <p class="rail-hint">With dates it is an away period; without, their home base.</p>
  <input aria-label="Person" placeholder="Person" list="people-names" bind:value={formPerson} required />
  <datalist id="people-names">
    {#each people as person (person.id)}
      <option value={person.name}></option>
    {/each}
  </datalist>
  <input aria-label="City" placeholder="City" bind:value={formCity} required />
  <div class="dates">
    <input aria-label="From" type="date" bind:value={formFrom} />
    <input aria-label="To" type="date" bind:value={formTo} />
  </div>
  <button class="btn btn-outline" type="submit" disabled={formBusy}>
    {formBusy ? "Saving…" : "Save"}
  </button>
  {#if formMessage}<p class="meta" aria-live="polite">{formMessage}</p>{/if}
</form>
</details>

<a class="btn btn-outline rail-action" href={link("/people")}>
  <Icon name="users" size={13} /> All people
</a>

<style>
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
    font-size: var(--text-xs);
    line-height: 1.4;
  }

  .around-list {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    margin: 0 0 0.6rem;
    padding: 0;
    list-style: none;
  }

  .around-list li {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    padding: 0.5rem 0.6rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
  }

  .who {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0.4rem;
    margin: 0;
    font-size: var(--text-sm);
  }

  .who span {
    color: var(--text-secondary);
  }

  .when {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-primary);
  }

  .meta {
    margin: 0;
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
  }

  .stay-line {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-secondary);
  }

  .state-add {
    margin-top: 0.6rem;
  }

  .state-add summary {
    color: var(--text-secondary);
    font-size: var(--text-xs);
    cursor: pointer;
  }

  .state-add summary:hover {
    color: var(--primary);
  }

  .state-form {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    margin-top: 0.6rem;
  }

  .state-form input {
    padding: 0.3rem 0.45rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    background: var(--card-bg);
    color: var(--text-primary);
    font: inherit;
    font-size: var(--text-xs);
  }

  .state-form .dates {
    display: flex;
    gap: 0.35rem;
  }

  .state-form .dates input {
    flex: 1;
    min-width: 0;
  }

  .rail-action {
    display: inline-flex;
    width: 100%;
    align-items: center;
    justify-content: center;
    gap: 0.35rem;
    margin-top: 0.5rem;
    padding: 0.3rem 0.5rem;
    font-size: var(--text-xs);
    text-decoration: none;
  }

  .person-chip-btn {
    display: inline;
    padding: 0;
    margin: 0;
    border: none;
    background: none;
    color: var(--primary);
    font: inherit;
    font-weight: 500;
    cursor: pointer;
    text-decoration: none;
  }

  .person-chip-btn:hover {
    text-decoration: underline;
  }

  .meetup-action-link {
    display: inline-flex;
    align-items: center;
    gap: 0.2rem;
    margin-left: 0.35rem;
    padding: 0.05rem 0.35rem;
    border-radius: var(--radius-sm);
    background: var(--primary-soft);
    color: var(--primary);
    font-size: var(--text-2xs);
    font-weight: 600;
    text-decoration: none;
    transition: opacity var(--motion-fast) ease;
  }

  .meetup-action-link:hover {
    opacity: 0.85;
    text-decoration: none;
  }
</style>
