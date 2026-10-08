<script lang="ts">
  import { onMount } from "svelte";
  import { page } from "$app/state";
  import Icon from "$lib/Icon.svelte";
  import PageHeader from "$lib/PageHeader.svelte";
  import FinanceDashboard from "$lib/finance/FinanceDashboard.svelte";
  import InvestmentsPanel from "$lib/finance/InvestmentsPanel.svelte";
  import DecisionsInbox from "$lib/finance/DecisionsInbox.svelte";
  import DataTable, { type Column } from "$lib/ui/DataTable.svelte";
  import Chip, { type Tone } from "$lib/ui/Chip.svelte";
  import SidePeek from "$lib/ui/SidePeek.svelte";
  import * as invest from "$lib/finance/invest-api";
  import {
    axonStatus,
    finance,
    type Burn,
    type FinanceDashboard as FinanceDashboardData,
    type PricePoint,
    type Subscription,
    type SubscriptionState,
    type WritebackResult,
  } from "$lib/api";

  // The tab order, and the union derived from it: two lists would drift, and the
  // one that drifted would be the one a deep link is checked against.
  const VIEWS = ["overview", "planning", "transactions", "investments", "subscriptions"] as const;
  type View = (typeof VIEWS)[number];

  // Home's decision row links to /finance?view=investments, so the query has to
  // land on the tab it names -- a link that opens a different tab is a link that
  // did not work. Seeded ONCE rather than $derived: the tabs below write `view`,
  // and a derived value would snap back to the URL on the next click.
  const requestedView = page.url.searchParams.get("view");
  let view = $state<View>(VIEWS.find((candidate) => candidate === requestedView) ?? "overview");

  // The date picker is the point of this page rather than a convenience on it. A
  // subscription's price is an append-only series, so "what am I paying" and "what
  // will I be paying in October" are different questions with different answers,
  // and every other subscription tool can only answer the first.
  let at = $state(new Date().toISOString().slice(0, 10));

  let subs = $state<Subscription[]>([]);
  let burn = $state<Burn | null>(null);
  let subscriptionInsights = $state<FinanceDashboardData["planning"]["subscriptions"] | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let notice = $state<string | null>(null);
  let loaded = $state(false);

  async function load(date: string) {
    try {
      const [list, totals, dashboard] = await Promise.all([
        finance.subscriptions(),
        finance.burn(date),
        finance.dashboard({ currency: "EUR" }),
      ]);
      subs = list;
      burn = totals;
      subscriptionInsights = dashboard.planning.subscriptions;
      error = null;
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      loaded = true;
    }
  }

  // A native date input can emit a partial or malformed value mid-edit, and it does
  // not have to be plausible: typing eight digits into the year segment produced
  // `202610-01-08`, which every comparison here happily accepted. Dates are compared
  // lexicographically, so a malformed one does not throw — it sorts somewhere
  // arbitrary and returns a confident wrong number, which is the failure mode this
  // whole capability exists to avoid.
  const WELL_FORMED = /^\d{4}-\d{2}-\d{2}$/;
  const validAt = $derived(WELL_FORMED.test(at) ? at : null);

  // Investments: loaded on demand, because a cold page must not pay for a
  // portfolio nobody asked to see.
  let portfolio = $state<invest.Portfolio | null>(null);
  let priceStatus = $state<invest.PriceStatus | null>(null);
  let openDecisions = $state<invest.Decision[]>([]);
  let investError = $state<string | null>(null);
  let investLoaded = $state(false);

  async function loadInvestments() {
    try {
      const [book, prices, open] = await Promise.all([
        invest.portfolio("EUR"),
        invest.priceStatus(),
        invest.decisions("open"),
      ]);
      portfolio = book;
      priceStatus = prices;
      openDecisions = open;
      investError = null;
    } catch (cause) {
      investError = cause instanceof Error ? cause.message : String(cause);
    } finally {
      investLoaded = true;
    }
  }

  // finance declares autostart = "false" in its manifest, and this page was the
  // one finance surface that never started it — on a cold machine it showed a raw
  // request error instead of a capability that had simply not been started.
  onMount(() => {
    void axonStatus.start("finance").catch(() => undefined);
    void load(at);
  });

  $effect(() => {
    if (view === "investments" && !investLoaded) void loadInvestments();
  });

  $effect(() => {
    if (loaded && validAt) void load(validAt);
  });

  const ORDER: Record<SubscriptionState, number> = {
    active: 0,
    trial: 1,
    covered: 2,
    paused: 3,
    considering: 4,
    cancelled: 5,
  };

  interface Row {
    sub: Subscription;
    state: SubscriptionState;
    /** The price in force at `at`, which is also what the editor prefills from. */
    price: PricePoint | null;
    monthlyCents: number;
    currency: string;
    scheduled: { valid_from: string; amount_cents: number; currency: string } | null;
  }

  /** The same arithmetic the server does, so a row and the total agree. The server
   *  owns the authoritative figure; this exists because rendering per-row costs
   *  would otherwise need one request each. */
  function at_or_before<T extends { d: string }>(items: T[], date: string): T | null {
    let best: T | null = null;
    // Series are ordered oldest to newest. On a same-day correction the later
    // append supersedes the earlier speculative point.
    for (const item of items) if (item.d <= date && (!best || item.d >= best.d)) best = item;
    return best;
  }

  const MONTHLY: Record<string, (cents: number) => number> = {
    weekly: (c) => Math.round((c * 52) / 12),
    monthly: (c) => c,
    quarterly: (c) => Math.round(c / 3),
    yearly: (c) => Math.round(c / 12),
    one_off: () => 0,
  };

  const rows = $derived.by<Row[]>(() =>
    subs
      .map((sub) => {
        const price = at_or_before(
          sub.prices.map((p) => ({ d: p.valid_from, p })),
          at,
        )?.p;
        const state =
          at_or_before(
            sub.states.map((s) => ({ d: s.effective, s })),
            at,
          )?.s.state ?? "considering";
        const billing = state === "active" || state === "trial";
        let scheduled: PricePoint | null = null;
        for (const future of sub.prices) {
          if (future.valid_from <= at) continue;
          if (!scheduled || future.valid_from <= scheduled.valid_from) scheduled = future;
        }
        if (scheduled && price
          && scheduled.amount_cents === price.amount_cents
          && scheduled.currency === price.currency
          && scheduled.cycle === price.cycle
          && scheduled.plan === price.plan) scheduled = null;
        return {
          sub,
          state,
          price: price ?? null,
          monthlyCents: billing && price ? (MONTHLY[price.cycle] ?? ((c: number) => c))(price.amount_cents) : 0,
          currency: price?.currency ?? "EUR",
          scheduled,
        };
      })
      // Billing first, then most expensive. A cut candidate is a big number next to
      // a low value rating, and this puts both in the first rows.
      .sort(
        (a, b) => ORDER[a.state] - ORDER[b.state] || b.monthlyCents - a.monthlyCents,
      ),
  );

  let scenarioIds = $state<string[]>([]);
  let scenarioInitialized = $state(false);

  $effect(() => {
    if (!loaded || scenarioInitialized || rows.length === 0) return;
    scenarioIds = rows
      .filter((row) => row.state === "active" || row.state === "trial")
      .map((row) => row.sub.id);
    scenarioInitialized = true;
  });

  function toggleScenario(id: string) {
    scenarioIds = scenarioIds.includes(id)
      ? scenarioIds.filter((candidate) => candidate !== id)
      : [...scenarioIds, id];
  }

  function scenarioMonthly(row: Row): number {
    if (!row.price) return 0;
    return (MONTHLY[row.price.cycle] ?? ((c: number) => c))(row.price.amount_cents);
  }

  const scenarioTotals = $derived.by(() => {
    const totals = new Map<string, number>(
      burn?.currencies.map((amount) => [amount.currency, 0]) ?? [],
    );
    for (const row of rows) {
      if (!scenarioIds.includes(row.sub.id)) continue;
      totals.set(row.currency, (totals.get(row.currency) ?? 0) + scenarioMonthly(row));
    }
    return [...totals.entries()].map(([currency, monthlyCents]) => ({ currency, monthlyCents }));
  });

  function currentBurn(currency: string): number {
    return burn?.currencies.find((amount) => amount.currency === currency)?.monthly_cents ?? 0;
  }

  interface Upcoming {
    id: string;
    name: string;
    valid_from: string;
    amount_cents: number;
    currency: string;
  }

  // Narrowed here rather than asserted in the markup: the template is not parsed as
  // TypeScript, so `row.scheduled!` is a syntax error rather than a type hint.
  const upcoming = $derived.by<Upcoming[]>(() =>
    rows.flatMap((row) =>
      row.scheduled
        ? [
            {
              id: row.sub.id,
              name: row.sub.name,
              valid_from: row.scheduled.valid_from,
              amount_cents: row.scheduled.amount_cents,
              currency: row.scheduled.currency,
            },
          ]
        : [],
    ),
  );

  function money(cents: number, currency = "EUR"): string {
    const sign = cents < 0 ? "-" : "";
    const abs = Math.abs(cents);
    return `${sign}${Math.floor(abs / 100)}.${String(abs % 100).padStart(2, "0")} ${currency}`;
  }

  const PILLAR_MAPPING: Record<string, string> = {
    housing: "Living Space",
    home: "Living Space",
    utilities: "Living Space",
    interior: "Living Space",
    mobility: "Mobility & Travel",
    travel: "Mobility & Travel",
    transit: "Mobility & Travel",
    infrastructure: "Tools & Infra",
    cloud: "Tools & Infra",
    software: "Tools & Infra",
    dev: "Tools & Infra",
    knowledge: "Mind & Knowledge",
    learning: "Mind & Knowledge",
    feed: "Mind & Knowledge",
    news: "Mind & Knowledge",
    health: "Vitality & Health",
    wellness: "Vitality & Health",
  };

  function pillarOf(category?: string | null): string {
    if (!category) return "Other";
    return PILLAR_MAPPING[category.toLowerCase()] ?? "Other";
  }

  let selectedPillar = $state<string>("all");

  const visibleRows = $derived(
    selectedPillar === "all"
      ? rows
      : rows.filter((r) => pillarOf(r.sub.category) === selectedPillar),
  );

  const pillarTotals = $derived.by(() => {
    const totals: Record<string, number> = {};
    for (const r of rows) {
      const p = pillarOf(r.sub.category);
      totals[p] = (totals[p] ?? 0) + (r.monthlyCents > 0 ? r.monthlyCents : 0);
    }
    return totals;
  });

  const totalMonthlyCents = $derived(
    Object.values(pillarTotals).reduce((sum, v) => sum + v, 0),
  );

  // Which row is expanded. One at a time: two open editors invite editing the wrong
  // subscription, and the rows are one line each so there is nothing to compare.
  let open = $state<string | null>(null);
  const openRow = $derived(rows.find((r) => r.sub.id === open) ?? null);

  // success = settled, warning = needs a decision, muted = not costing anything (Chip.svelte).
  const STATE_TONE: Record<SubscriptionState, Tone> = {
    active: "success",
    trial: "accent",
    covered: "neutral",
    considering: "warning",
    paused: "muted",
    cancelled: "muted",
  };

  const subColumns: Column<Row>[] = [
    { id: "name", label: "Subscription", cell: subNameCell },
    { id: "state", label: "State", width: "7rem", cell: subStateCell },
    { id: "monthly", label: "Monthly", width: "7rem", align: "end", cell: subMonthlyCell },
    { id: "value", label: "Value", width: "4rem", align: "end", cell: subValueCell },
    { id: "category", label: "Category", width: "8rem", cell: subCategoryCell },
  ];

  // Draft state for the two forms. Reset whenever a different row opens, so a date
  // typed for one subscription cannot be submitted against another.
  let priceDraft = $state({
    valid_from: '',
    amount: '',
    cycle: 'monthly' as PricePoint['cycle'],
    plan: '',
    reason: '',
  });
  let stateDraft = $state({ effective: '', state: 'paused' as SubscriptionState, note: '' });

  function toggle(row: Row) {
    if (open === row.sub.id) {
      open = null;
      return;
    }
    open = row.sub.id;
    priceDraft = {
      valid_from: at,
      amount: '',
      cycle: row.price?.cycle ?? 'monthly',
      plan: row.price?.plan ?? '',
      reason: '',
    };
    stateDraft = {
      effective: at,
      state: row.state === 'paused' ? 'active' : 'paused',
      note: '',
    };
  }

  /** Tiers already recorded on this subscription, so a second switch is a click. */
  function knownPlans(sub: Subscription): string[] {
    return [...new Set(sub.prices.map((p) => p.plan).filter((p): p is string => !!p))];
  }

  /** "20", "20,00" and "€ 20.00" all mean the same thing to a person. */
  function toCents(raw: string): number | null {
    if (raw.includes('-')) return null;
    const cleaned = raw.trim().replace(',', '.').replace(/[^\d.]/g, '');
    if (!cleaned) return null;
    const value = Number(cleaned);
    const cents = Math.round(value * 100);
    return Number.isSafeInteger(cents) ? cents : null;
  }

  const priceReady = $derived(
    WELL_FORMED.test(priceDraft.valid_from) &&
      toCents(priceDraft.amount) !== null &&
      priceDraft.reason.trim().length > 0,
  );
  const stateReady = $derived(WELL_FORMED.test(stateDraft.effective));

  async function submitPrice(row: Row) {
    const amount_cents = toCents(priceDraft.amount);
    if (amount_cents === null) return;
    const saved = await run('price', () =>
      finance.appendPrice(row.sub.id, {
        valid_from: priceDraft.valid_from,
        amount_cents,
        currency: row.currency,
        cycle: priceDraft.cycle,
        plan: priceDraft.plan.trim() || null,
        reason: priceDraft.reason.trim(),
      }),
    );
    if (saved) open = null;
  }

  async function submitState(row: Row) {
    const saved = await run('state', () =>
      finance.appendState(row.sub.id, {
        effective: stateDraft.effective,
        state: stateDraft.state,
        note: stateDraft.note.trim(),
      }),
    );
    if (saved) open = null;
  }

  async function run(label: string, action: () => Promise<unknown>): Promise<boolean> {
    busy = true;
    notice = null;
    try {
      const result = await action();
      notice = describe(label, result);
      await load(at);
      return true;
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
      return false;
    } finally {
      busy = false;
    }
  }

  /** A writeback that hit a conflict is not a failure to report as an error, and
   *  not a success to report as done. It names the notes and waits for a human.
   *
   *  Regions and projections are counted apart because they are different places:
   *  a region goes inside a note you wrote, a projection is a whole generated file
   *  for a subscription that has no note. Summing them would report "7 written"
   *  over a vault where nothing you own was touched. */
  function describe(label: string, result: unknown): string {
    if (label === "writeback") {
      const w = result as WritebackResult;
      const p = w.projected;
      const projected =
        p && p.created + p.updated > 0
          ? ` ${p.created + p.updated} subscription(s) with no note projected to Resources/Sjel/.`
          : "";
      if (w.conflicts.length > 0) {
        return `${w.written} written, ${w.unchanged} unchanged.${projected} Left alone because you edited inside the block: ${w.conflicts.join(", ")}`;
      }
      if (p && p.refused.length > 0) {
        return `${w.written} written, ${w.unchanged} already correct.${projected} Not written, a note of yours holds the path: ${p.refused.join(", ")}`;
      }
      return `${w.written} written, ${w.unchanged} already correct.${projected}`;
    }
    if (label === 'price' || label === 'state') {
      const write = result as { created: boolean };
      if (!write.created) return 'That exact history point was already present.';
      return label === 'price'
        ? 'Price point appended. Nothing was overwritten.'
        : 'State change appended. Nothing was overwritten.';
    }
    const i = result as { created: number; already_present: number };
    return `${i.created} imported, ${i.already_present} already known.`;
  }
</script>

{#snippet subNameCell(row: Row)}
  {row.sub.name}{#if row.price?.plan} <Chip label={row.price.plan} tone="muted" />{/if}
{/snippet}
{#snippet subStateCell(row: Row)}<Chip label={row.state} tone={STATE_TONE[row.state]} />{/snippet}
{#snippet subMonthlyCell(row: Row)}{row.monthlyCents > 0 ? money(row.monthlyCents, row.currency) : ""}{/snippet}
{#snippet subValueCell(row: Row)}{row.sub.value_rating ?? ""}{/snippet}
{#snippet subCategoryCell(row: Row)}<span class="muted">{row.sub.category ?? ""}</span>{/snippet}

<PageHeader
  badge="Finance"
  title="Money, as a system"
  desc="Cash flow, reviewed transactions and recurring commitments from one journal-backed projection."
/>

<nav aria-label="Finance views">
  {#each VIEWS as item (item)}
    <button class:active={view === item} onclick={() => view = item}>{item}</button>
  {/each}
</nav>

{#if view === "investments"}
  {#if investError}
    <p class="err"><Icon name="alert" size={14} /> {investError}</p>
  {:else if !investLoaded}
    <p class="muted">Loading…</p>
  {:else if portfolio}
    <DecisionsInbox decisions={openDecisions} onchanged={loadInvestments} />
    <InvestmentsPanel {portfolio} prices={priceStatus} />
  {/if}
{:else if view !== "subscriptions"}
  <FinanceDashboard mode={view} onnavigate={(target) => view = target} />
{:else if error}
  <p class="err"><Icon name="alert" size={14} /> {error}</p>
{:else if !loaded}
  <p class="muted">Loading…</p>
{:else}
  <section class="bar">
    <label class="date">
      <span>As of</span>
      <input type="date" bind:value={at} />
    </label>

    {#if !validAt}
      <p class="muted totals">Waiting for a complete date…</p>
    {:else if burn}
      <div class="totals">
        {#each burn.currencies as total (total.currency)}
          <div class="total">
            <span class="figure">{money(total.monthly_cents, total.currency)}</span>
            <span class="unit">per month · {money(total.annual_cents, total.currency)} per year</span>
          </div>
        {/each}
        <p class="muted count">
          {burn.billing_count} personally billing · {burn.covered_count} externally covered · {burn.total_count} tracked{burn.unknown_price_count ? ` · ${burn.unknown_price_count} missing a price` : ""}
        </p>
      </div>
    {/if}

    <div class="actions">
      <button disabled={busy} onclick={() => run("import", finance.importVault)}>
        <Icon name="refresh" size={14} /> Import vault
      </button>
      <button disabled={busy} onclick={() => run("writeback", finance.writeback)}>
        <Icon name="arrow-right" size={14} /> Write back
      </button>
    </div>
  </section>

  {#if notice}<p class="notice">{notice}</p>{/if}

  <section class="scenario">
    <div class="scenario-heading">
      <div>
        <h2>Plan combination</h2>
        <p>Toggle the services you would keep. Prices come from the history in force on {at}; this does not change their recorded state.</p>
      </div>
      <div class="scenario-totals">
        {#each scenarioTotals as total (total.currency)}
          <strong>{money(total.monthlyCents, total.currency)} / month</strong>
          <span class:save={total.monthlyCents < currentBurn(total.currency)}>
            {total.monthlyCents <= currentBurn(total.currency) ? "save" : "add"} {money(Math.abs(currentBurn(total.currency) - total.monthlyCents), total.currency)} / month
          </span>
        {/each}
      </div>
    </div>
    <div class="scenario-options">
      {#each rows.filter((row) => row.price !== null) as row (row.sub.id)}
        <label class:selected={scenarioIds.includes(row.sub.id)}>
          <input type="checkbox" checked={scenarioIds.includes(row.sub.id)} onchange={() => toggleScenario(row.sub.id)} />
          <span><strong>{row.sub.name}</strong>{#if row.price?.plan}<small>{row.price.plan}</small>{/if}</span>
          <span class="state {row.state}">{row.state}</span>
          <strong>{money(scenarioMonthly(row), row.currency)}</strong>
        </label>
      {/each}
    </div>
    {#if subscriptionInsights?.anomalies.length}
      <div class="scenario-anomalies">
        <strong>Review flags</strong>
        <ul>
          {#each subscriptionInsights.anomalies as anomaly (`${anomaly.subscription_id}-${anomaly.kind}`)}
            <li><strong>{anomaly.subscription_name}</strong> — {anomaly.detail}</li>
          {/each}
        </ul>
      </div>
    {/if}
  </section>

  {#if upcoming.length > 0}
    <section class="upcoming">
      <h2>Price changes ahead</h2>
      <ul>
        {#each upcoming as item (item.id)}
          <li>
            <strong>{item.name}</strong> goes to
            {money(item.amount_cents, item.currency)} on {item.valid_from}
          </li>
        {/each}
      </ul>
    </section>
  {/if}

  <div class="pillar-filter-bar" role="group" aria-label="Life pillars">
    <button
      type="button"
      class="pillar-pill"
      class:active={selectedPillar === "all"}
      onclick={() => (selectedPillar = "all")}
    >
      All Pillars <span class="pillar-sum">{money(totalMonthlyCents)}/mo</span>
    </button>
    {#each Object.entries(pillarTotals) as [pillar, totalCents] (pillar)}
      <button
        type="button"
        class="pillar-pill"
        class:active={selectedPillar === pillar}
        onclick={() => (selectedPillar = pillar)}
      >
        {pillar} <span class="pillar-sum">{money(totalCents)}/mo</span>
      </button>
    {/each}
  </div>

  <div class="sub-card">
    <DataTable
      rows={visibleRows}
      columns={subColumns}
      key={(r) => r.sub.id}
      onOpen={toggle}
      selected={open}
      inactive={(r) => r.monthlyCents === 0}
      empty={rows.length === 0 ? "Nothing imported yet. Import vault reads the notes." : "No subscriptions in this pillar."}
    />
  </div>

  {#if openRow}
    {@const row = openRow}
    <SidePeek title={row.sub.name} eyebrow={row.price?.plan ? `Subscription · ${row.price.plan}` : "Subscription"} onClose={() => (open = null)}>
    <div class="panes">
        <form class="pane" onsubmit={(e) => { e.preventDefault(); void submitPrice(row); }}>
          <h3>Change price or plan</h3>
          <div class="fields">
            <label>From<input type="date" bind:value={priceDraft.valid_from} /></label>
            <label>Amount<input
              type="text"
              inputmode="decimal"
              placeholder="100"
              bind:value={priceDraft.amount}
            /></label>
            <label>Cycle<select bind:value={priceDraft.cycle}>
              <option value="weekly">weekly</option>
              <option value="monthly">monthly</option>
              <option value="quarterly">quarterly</option>
              <option value="yearly">yearly</option>
              <option value="one_off">one-off</option>
            </select></label>
            <label>Plan<input
              type="text"
              list="plans-{row.sub.id}"
              placeholder="Max"
              bind:value={priceDraft.plan}
            /></label>
          </div>
          <datalist id="plans-{row.sub.id}">
            {#each knownPlans(row.sub) as plan (plan)}<option value={plan}></option>{/each}
          </datalist>
          <label class="wide">Why<input
            type="text"
            required
            placeholder="upgrade to Max, income scales"
            bind:value={priceDraft.reason}
          /></label>
          <button type="submit" disabled={busy || !priceReady}>Append price point</button>
        </form>

        <form class="pane" onsubmit={(e) => { e.preventDefault(); void submitState(row); }}>
          <h3>Change state</h3>
          <div class="fields">
            <label>From<input type="date" bind:value={stateDraft.effective} /></label>
            <label>State<select bind:value={stateDraft.state}>
              <option value="active">active</option>
              <option value="covered">covered externally</option>
              <option value="paused">paused</option>
              <option value="trial">trial</option>
              <option value="considering">considering</option>
              <option value="cancelled">cancelled</option>
            </select></label>
          </div>
          <label class="wide">Note<input
            type="text"
            placeholder="reassess after the raise lands"
            bind:value={stateDraft.note}
          /></label>
          <button type="submit" disabled={busy || !stateReady}>Append state change</button>
        </form>

        <div class="pane history">
          <h3>History</h3>
          <ul>
            {#each row.sub.prices as p (p.valid_from + p.reason)}
              <li>
                <code>{p.valid_from}</code>
                {money(p.amount_cents, p.currency)}{#if p.plan}, {p.plan}{/if}
                {#if p.reason}<span class="muted">— {p.reason}</span>{/if}
              </li>
            {/each}
            {#each row.sub.states as st (st.effective + st.state)}
              <li>
                <code>{st.effective}</code> {st.state}
                {#if st.note}<span class="muted">— {st.note}</span>{/if}
              </li>
            {/each}
          </ul>
          <p class="muted small">Appended, never edited. Nothing above is overwritten.</p>
        </div>
      </div>
    </SidePeek>
  {/if}
{/if}

<style>
  .pillar-filter-bar {
    display: flex;
    gap: 0.4rem;
    flex-wrap: wrap;
    align-items: center;
    margin: 1rem 0;
  }

  .pillar-pill {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.3rem 0.65rem;
    border-radius: var(--radius-full);
    font-size: var(--text-xs);
    font-weight: 500;
    background: var(--card-bg);
    border: 1px solid var(--rule);
    color: var(--text-secondary);
    cursor: pointer;
    transition: background-color var(--motion-fast) ease, border-color var(--motion-fast) ease, color var(--motion-fast) ease;
  }

  .pillar-pill:hover {
    color: var(--text-primary);
    border-color: var(--primary);
  }

  .pillar-pill.active {
    background: var(--primary-soft);
    color: var(--primary);
    border-color: var(--primary);
    font-weight: 600;
  }

  .pillar-sum {
    padding: 0.05rem 0.35rem;
    border-radius: var(--radius-full);
    background: var(--rule-soft, rgba(125, 125, 125, 0.15));
    font-size: var(--text-2xs);
  }

  nav {
    display: flex;
    gap: 0.2rem;
    margin: -0.25rem 0 1.25rem;
    border-bottom: 1px solid var(--card-border);
    max-width: 100%;
    overflow-x: auto;
  }

  nav button {
    border: 0;
    border-bottom: 2px solid transparent;
    padding: 0.55rem 0.75rem;
    background: transparent;
    color: var(--text-tertiary);
    font: inherit;
    font-size: 0.78rem;
    text-transform: capitalize;
    white-space: nowrap;
    cursor: pointer;
  }

  nav button.active {
    color: inherit;
    border-bottom-color: var(--primary);
  }

  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: 1.5rem;
    margin-bottom: 1.25rem;
  }

  .date {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    font-size: var(--text-xs);
    color: var(--text-tertiary);
  }

  .date input {
    font: inherit;
    font-size: 0.875rem;
    padding: 0.35rem 0.5rem;
    border: 1px solid var(--card-border);
    border-radius: 6px;
    background: transparent;
    color: inherit;
  }

  .totals {
    display: flex;
    align-items: baseline;
    gap: 1.25rem;
    flex-wrap: wrap;
  }

  .total {
    display: flex;
    align-items: baseline;
    gap: 0.35rem;
  }

  .figure {
    font-size: var(--text-xl);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  .unit {
    font-size: var(--text-xs);
    color: var(--text-tertiary);
  }

  .count {
    margin: 0;
    font-size: var(--text-xs);
  }

  .actions {
    display: flex;
    gap: 0.5rem;
    margin-left: auto;
  }

  .actions button {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    font: inherit;
    font-size: var(--text-sm);
    padding: 0.4rem 0.7rem;
    border: 1px solid var(--card-border);
    border-radius: 6px;
    background: transparent;
    color: inherit;
    cursor: pointer;
  }

  .actions button:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .scenario {
    margin-bottom: 1rem;
    padding: 0.85rem;
    border: 1px solid var(--card-border);
    border-top: 2px solid var(--primary);
  }

  .scenario-heading {
    display: flex;
    justify-content: space-between;
    align-items: start;
    gap: 1rem;
    margin-bottom: 0.75rem;
  }

  .scenario h2 {
    margin: 0;
    font-size: 0.85rem;
  }

  .scenario p {
    margin: 0.18rem 0 0;
    color: var(--text-tertiary);
    font-size: 0.68rem;
  }

  .scenario-totals {
    display: grid;
    justify-items: end;
    gap: 0.15rem;
    font-size: 0.72rem;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .scenario-totals span {
    color: var(--danger, #b44);
    font-size: 0.65rem;
  }

  .scenario-totals span.save {
    color: var(--primary);
  }

  .scenario-options {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(18rem, 1fr));
    border-top: 1px solid var(--card-border);
  }

  .scenario-options label {
    display: grid;
    grid-template-columns: auto minmax(8rem, 1fr) auto auto;
    align-items: center;
    gap: 0.55rem;
    padding: 0.5rem;
    border-bottom: 1px solid var(--card-border);
    font-size: 0.7rem;
    cursor: pointer;
  }

  .scenario-options label.selected {
    background: color-mix(in srgb, var(--primary) 6%, transparent);
  }

  .scenario-options label > span:nth-child(2) {
    display: grid;
    gap: 0.08rem;
  }

  .scenario-options small {
    color: var(--text-tertiary);
    font-size: 0.6rem;
  }

  .scenario-options label > strong:last-child {
    font-variant-numeric: tabular-nums;
  }

  .scenario-anomalies {
    display: grid;
    grid-template-columns: 7rem 1fr;
    gap: 0.75rem;
    margin-top: 0.75rem;
    font-size: 0.68rem;
  }

  .scenario-anomalies ul {
    margin: 0;
    padding-left: 1rem;
    color: var(--text-tertiary);
  }

  .upcoming {
    margin-bottom: 1.25rem;
    padding: 0.75rem 1rem;
    border: 1px solid var(--card-border);
    border-radius: 8px;
  }

  .upcoming h2 {
    margin: 0 0 0.5rem;
    font-size: var(--text-2xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--primary);
  }

  .upcoming ul {
    margin: 0;
    padding-left: 1.1rem;
    font-size: 0.875rem;
  }







  .state {
    font-size: var(--text-2xs);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 0.1rem 0.4rem;
    border-radius: 4px;
    border: 1px solid var(--card-border);
  }

  .state.active {
    border-color: var(--primary);
    color: var(--primary);
  }

  .state.paused,
  .state.cancelled {
    opacity: 0.6;
  }

  .muted {
    color: var(--text-tertiary);
  }






  .sub-card {
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-lg);
    background: var(--card-bg);
    border: 1px solid var(--card-border);
  }

  .panes {
    display: grid;
    gap: var(--space-5);
  }

  .pane {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .pane h3 {
    margin: 0;
    font-size: var(--text-2xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--primary);
  }

  .fields {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
  }

  .pane label {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
    flex: 1;
  }

  .pane input,
  .pane select {
    font: inherit;
    font-size: var(--text-sm);
    padding: 0.3rem 0.45rem;
    border: 1px solid var(--card-border);
    border-radius: 5px;
    background: transparent;
    color: inherit;
    min-width: 0;
  }

  .pane button {
    align-self: flex-start;
    font: inherit;
    font-size: var(--text-sm);
    padding: 0.35rem 0.7rem;
    border: 1px solid var(--card-border);
    border-radius: 6px;
    background: transparent;
    color: inherit;
    cursor: pointer;
  }

  .pane button:disabled {
    opacity: 0.45;
    cursor: default;
  }

  .history ul {
    margin: 0;
    padding-left: 1rem;
    font-size: var(--text-sm);
    line-height: 1.6;
  }

  .history code {
    font-size: var(--text-xs);
    opacity: 0.75;
  }

  .small {
    font-size: var(--text-2xs);
    margin: 0.2rem 0 0;
  }

  .notice {
    font-size: var(--text-sm);
    margin: 0 0 1rem;
  }

  .err {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.875rem;
  }

  @media (max-width: 650px) {
    .scenario-heading {
      flex-direction: column;
    }

    .scenario-totals {
      justify-items: start;
    }

    .scenario-options {
      grid-template-columns: 1fr;
    }

    .scenario-anomalies {
      grid-template-columns: 1fr;
    }
  }
</style>
