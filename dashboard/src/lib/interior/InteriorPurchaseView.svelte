<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import {
    interior,
    type InteriorDeclaration,
    type InteriorDelivery,
    type InteriorPurchaseOrder,
  } from "$lib/api";

  const euro = (cents: number): string =>
    (cents / 100).toLocaleString("de-DE", { style: "currency", currency: "EUR" });

  const doorLine = (e: InteriorDelivery): string => {
    switch (e.door.kind) {
      case "fits":
        return `through the ${e.door.door_cm} cm door, ${e.door.air_cm} cm to spare`;
      case "does_not_fit":
        return `does NOT fit the ${e.door.door_cm} cm door — ${e.door.missing_cm} cm too wide`;
      case "carried_apart":
        return `${e.door.missing_cm} cm wider than the ${e.door.door_cm} cm door, carried in parts`;
      case "no_entrance":
        return "no opening is declared as the entrance";
    }
  };

  let declarations = $state<InteriorDeclaration[]>([]);
  let order = $state<InteriorPurchaseOrder | null>(null);
  let probeB = $state(120);
  let probeT = $state(60);
  let probeZerlegbar = $state(false);
  let probeAnswer = $state<string | null>(null);

  async function loadDeclarations(): Promise<void> {
    try {
      declarations = await interior.declarations();
    } catch {
      declarations = [];
    }
  }

  async function loadOrder(): Promise<void> {
    try {
      order = await interior.purchaseOrder();
    } catch {
      order = null;
    }
  }

  async function probeDoor(): Promise<void> {
    probeAnswer = null;
    try {
      const t = await interior.fits(probeB, probeT, probeZerlegbar);
      probeAnswer = doorLine({
        reference: "",
        width: probeB,
        depth: probeT,
        door: t,
        affordable: false,
        rotates: false,
      });
    } catch (caught) {
      probeAnswer = caught instanceof Error ? caught.message : String(caught);
    }
  }

  onMount(() => {
    void loadOrder();
    void loadDeclarations();
  });
</script>

<section class="card">
  <h2>Does it even fit through the door?</h2>
  <p class="note">The question before buying, answered from the flat's own entrance width.</p>
  <div class="probe">
    <label>width <input type="number" bind:value={probeB} /></label>
    <label>depth <input type="number" bind:value={probeT} /></label>
    <label>
      <input type="checkbox" bind:checked={probeZerlegbar} /> comes in parts
    </label>
    <button type="button" onclick={probeDoor}>Check</button>
  </div>
  {#if probeAnswer}<p class="reserve">{probeAnswer}</p>{/if}
</section>

{#if order}
  <section class="card">
    <h2>What to buy first</h2>
    <p class="note">
      Urgency, then whether a layout already builds on it, then price. No budget: nobody set a
      ceiling and this page does not invent one.
    </p>
    <table class="corridors">
      <tbody>
        {#each order.lines as p, i (p.id)}
          <tr>
            <th scope="row">{i + 1}. {p.label}</th>
            <td class="mono">{p.priority ?? "—"}</td>
            <td class="mono">{euro(p.price_cents ?? 0)}</td>
            <td class="mono">{euro(p.cumulative_cents)}</td>
            <td class="mono">
              {p.affordable_after_months !== null
                ? `${p.affordable_after_months.toFixed(1)} months`
                : order.balance
                  ? "not out of this balance"
                  : "no balance measured"}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
    {#if order.unpriced.length > 0}
      <p class="unchecked">
        <Icon name="alert" size={14} />
        {order.unpriced.length} needs carry no price and stand in no order:
        {order.unpriced.map((p) => p.label).join(", ")} — the move here is a price, not a
        purchase.
      </p>
    {/if}
    {#if order.unknown_priorities.length > 0}
      <p class="unchecked">
        <Icon name="alert" size={14} />
        Priority words this ranking does not know, sorted last:
        {order.unknown_priorities.join(", ")}
      </p>
    {/if}
    {#if order.balance && order.balance.median_cents <= 0}
      <p class="note">
        Median monthly balance {euro(order.balance.median_cents)} over {order.balance.months}
        months — nothing accrues out of that, so there is no month count.
      </p>
    {/if}
  </section>
{/if}

{#if declarations.length > 0}
  <section class="card">
    <h2>
      Still judged by name
      <span class="count">{declarations.filter((d) => !d.declared).length}</span>
    </h2>
    <p class="note">
      The engine has been cleverer than its data since 2026-08-31. Each line below is what the
      machine already assumes, written in the vocabulary the piece can carry itself — and what
      accepting it would change.
    </p>
    <ul class="declarations">
      {#each declarations.filter((d) => !d.declared && d.proposal) as d (d.id)}
        <li>
          <span class="mono">{d.id}</span>
          <span class="of">read as {d.guessed_as}, in {d.in_layouts.length} layouts</span>
          <pre class="mono">{d.proposal?.toml}</pre>
          {#if d.consequences}
            <span class="holds" class:tight={d.consequences.changed.length > 0}>
              {d.consequences.changed.length === 0
                ? "changes no verdict"
                : `changes ${d.consequences.changed.join(", ")}`}
            </span>
          {/if}
        </li>
      {/each}
    </ul>
  </section>
{/if}

<style>
  .card {
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    margin-bottom: 1rem;
    padding: 1.25rem;
  }
  .card h2 {
    font-size: var(--text-base);
    margin: 0 0 0.5rem;
  }
  .probe {
    align-items: center;
    display: flex;
    flex-wrap: wrap;
    gap: 0.75rem;
    margin: 0.6rem 0;
  }
  .probe label {
    align-items: center;
    color: var(--text-secondary);
    display: inline-flex;
    font-size: var(--text-sm);
    gap: 0.35rem;
  }
  .probe input[type="number"] {
    width: 4.5rem;
  }
  .reserve {
    color: var(--primary);
    font-size: var(--text-sm);
    font-weight: 500;
    margin: 0.5rem 0 0;
  }
  .corridors {
    border-collapse: collapse;
    font-size: var(--text-sm);
    margin-top: 0.75rem;
    width: 100%;
  }
  .corridors th,
  .corridors td {
    border-top: 1px solid var(--card-border);
    padding: 0.45rem 0.6rem;
    text-align: left;
  }
  .corridors td {
    color: var(--text-secondary);
  }
  .unchecked {
    color: var(--warning-ink);
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: var(--text-xs);
    margin: 0.5rem 0 0;
  }
  .declarations {
    list-style: none;
    margin: 0.6rem 0 0;
    padding: 0;
  }
  .declarations li {
    border-top: 1px solid var(--card-border);
    padding: 0.5rem 0;
  }
  .declarations pre {
    background: var(--surface);
    border-radius: var(--radius-sm);
    color: var(--text-secondary);
    font-size: var(--text-xs);
    margin: 0.35rem 0;
    padding: 0.4rem 0.6rem;
    white-space: pre-wrap;
  }
  .count {
    color: var(--text-tertiary);
    margin-left: 0.35rem;
  }
  .note {
    color: var(--text-tertiary);
    font-size: var(--text-xs);
    margin: 0.25rem 0;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .of {
    color: var(--text-tertiary);
    font-size: var(--text-xs);
    margin-left: 0.5rem;
  }
  .holds {
    display: inline-block;
    font-size: var(--text-xs);
    color: var(--success);
    margin-top: 0.25rem;
  }
  .holds.tight {
    color: var(--warning-ink);
  }
</style>
