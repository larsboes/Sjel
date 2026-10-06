<script lang="ts">
  /**
   * Names other capabilities store as text and nobody has decided yet (libs/links/ISA.md D7-D11).
   *
   * Asked of every capability whose `links_to` declares `ent`, so a new capability that stores
   * people by name appears here without a change to this file. Exact matches never reach this
   * list: their owner links them on its own. What is left is the operator's call:
   *
   * - a candidate entities suggested, or any person picked from the list,
   * - "Not a person", for a group or a typo,
   * - "Create person", which adds the entity and links the name to it (D9: never automatic).
   *
   * Collapsed behind its count. The review is occasional work, not something to read every visit.
   */
  import { untrack } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import { entities, people, type Entity, type OpenName } from "$lib/api";
  import { capabilities } from "$lib/capabilities.svelte";

  let { roster, onchange }: { roster: Entity[]; onchange: () => void } = $props();

  type Row = OpenName & { capability: string };

  let rows = $state<Row[]>([]);
  let failures = $state<string[]>([]);
  let open = $state(false);
  let busy = $state<string | null>(null);
  let picks = $state<Record<string, string>>({});

  const declaring = $derived(capabilities.items.filter((c) => c.links_to?.includes("ent")));

  async function load(): Promise<void> {
    const answers = await Promise.allSettled(declaring.map((c) => people.open(c.name)));
    const found: Row[] = [];
    const failed: string[] = [];
    answers.forEach((answer, i) => {
      const name = declaring[i].name;
      if (answer.status === "rejected") {
        failed.push(`${name} did not answer: ${answer.reason instanceof Error ? answer.reason.message : String(answer.reason)}`);
        return;
      }
      if (answer.value.error) failed.push(`${name}: ${answer.value.error}. Names are listed without suggestions.`);
      for (const n of answer.value.names) found.push({ ...n, capability: name });
    });
    rows = found.sort((a, b) => a.name.localeCompare(b.name));
    failures = failed;
  }

  // Re-asked when the declarations change, not on every registry poll.
  $effect(() => {
    void capabilities.linksKey;
    untrack(() => void load());
  });

  const key = (r: Row) => `${r.capability}:${r.name}`;
  // The picker yields a name; a name two people share links neither, so the button stays hidden.
  const byName = $derived.by(() => {
    const ids = new Map<string, string | null>();
    for (const p of roster) ids.set(p.name, ids.has(p.name) ? null : p.id);
    return ids;
  });

  async function decide(r: Row, entityId: string | null): Promise<void> {
    busy = key(r);
    try {
      await people.decide(r.capability, r.name, entityId);
      rows = rows.filter((x) => key(x) !== key(r));
      onchange();
    } catch (err) {
      failures = [...failures, `${r.capability} did not record "${r.name}": ${err instanceof Error ? err.message : String(err)}`];
    } finally {
      busy = null;
    }
  }

  async function create(r: Row): Promise<void> {
    busy = key(r);
    try {
      const made = await entities.create({ kind: "person", name: r.name });
      busy = null;
      await decide(r, made.id);
    } catch (err) {
      failures = [...failures, `entities did not create "${r.name}": ${err instanceof Error ? err.message : String(err)}`];
      busy = null;
    }
  }

  const STATUS: Record<string, string> = {
    first: "first name only",
    ambiguous: "several people fit",
    none: "nobody by that name",
  };
</script>

{#if rows.length > 0 || failures.length > 0}
  <section class="review" aria-label="Names to link">
    <button type="button" class="toggle" aria-expanded={open} onclick={() => (open = !open)}>
      <Icon name="users" size={13} />
      <span>{rows.length} {rows.length === 1 ? "name" : "names"} to link to people</span>
      <span class="hint">written in trips and places, not yet tied to anyone here</span>
      <span class="chev" class:open><Icon name="chevron" size={12} /></span>
    </button>

    {#if open}
      {#each failures as failure (failure)}
        <p class="failure"><Icon name="alert" size={12} /> {failure}</p>
      {/each}
      <!-- One shared list for every row's picker: typing filters it natively. -->
      <datalist id="name-review-roster">
        {#each roster as p (p.id)}<option value={p.name}></option>{/each}
      </datalist>
      <ol>
        {#each rows as r (key(r))}
          <li>
            <div class="what">
              <strong>{r.name}</strong>
              <span class="meta">
                {r.capability} · {r.rows} {r.rows === 1 ? "row" : "rows"}{r.status ? ` · ${STATUS[r.status] ?? r.status}` : ""}
              </span>
            </div>
            <div class="actions">
              {#each r.candidates as c (c.id)}
                <button class="btn btn-soft btn-sm" type="button" disabled={busy !== null} onclick={() => decide(r, c.id)}>
                  Is {c.name}
                </button>
              {/each}
              <input
                list="name-review-roster"
                aria-label={`Link ${r.name} to someone else`}
                placeholder="Someone else…"
                bind:value={picks[key(r)]}
              />
              {#if byName.get(picks[key(r)] ?? "")}
                <button class="btn btn-soft btn-sm" type="button" disabled={busy !== null} onclick={() => decide(r, byName.get(picks[key(r)]) ?? null)}>Link</button>
              {/if}
              <button class="btn btn-outline btn-sm" type="button" disabled={busy !== null} onclick={() => create(r)}>Create person</button>
              <button class="btn btn-outline btn-sm" type="button" disabled={busy !== null} onclick={() => decide(r, null)}>Not a person</button>
            </div>
          </li>
        {/each}
      </ol>
    {/if}
  </section>
{/if}

<style>
  .review {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    margin-bottom: var(--space-4);
    padding: var(--space-3) var(--space-4);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    background: var(--card-bg);
  }

  .toggle {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    padding: 0;
    border: none;
    background: transparent;
    font: inherit;
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text-primary);
    text-align: left;
    cursor: pointer;
  }

  .hint {
    font-weight: 400;
    font-size: var(--text-xs);
    color: var(--text-tertiary);
  }

  .chev {
    margin-left: auto;
    display: inline-flex;
    transition: transform var(--motion-fast) var(--ease-out);
  }

  .chev.open {
    transform: rotate(90deg);
  }

  ol {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
  }

  li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2) var(--space-4);
    padding: var(--space-2) 0;
    border-top: 1px solid var(--card-border);
  }

  .what {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    min-width: 10rem;
  }

  .meta {
    font-size: var(--text-xs);
    color: var(--text-tertiary);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  input {
    font: inherit;
    font-size: var(--text-xs);
    padding: 0.2rem 0.4rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    background: var(--card-bg);
    color: var(--text-primary);
  }

  .failure {
    margin: 0;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-xs);
    color: var(--warning-ink);
  }
</style>
