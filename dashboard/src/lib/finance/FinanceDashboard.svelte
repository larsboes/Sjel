<script lang="ts">
  import { tip } from "$lib/tip";
  import { onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import ImportReview from "$lib/finance/ImportReview.svelte";
  import HoldingsPanel from "$lib/finance/HoldingsPanel.svelte";
  import BalanceEditor from "$lib/finance/BalanceEditor.svelte";
  import CategorizationReview from "$lib/finance/CategorizationReview.svelte";
  import CategoryTrend from "$lib/finance/CategoryTrend.svelte";
  import FinanceQuality from "$lib/finance/FinanceQuality.svelte";
  import InvestmentPreview from "$lib/finance/InvestmentPreview.svelte";
  import SpendingDrivers from "$lib/finance/SpendingDrivers.svelte";
  import SpendingPurposeOverview from "$lib/finance/SpendingPurposeOverview.svelte";
  import PlanningPanel from "$lib/finance/PlanningPanel.svelte";
  import TransactionTable from "$lib/finance/TransactionTable.svelte";
  import SpendingContextReview from "$lib/finance/SpendingContextReview.svelte";
  import { exactMoney } from "$lib/finance/money";
  import { finance, type FinanceDashboard } from "$lib/api";

  type Mode = "overview" | "planning" | "transactions";
  type ReviewTarget = "overview" | "transactions" | "subscriptions";
  let {
    mode,
    onnavigate,
  }: {
    mode: Mode;
    onnavigate: (target: ReviewTarget) => void;
  } = $props();
  const today = new Date().toISOString().slice(0, 10);
  const lookback = new Date(`${today}T00:00:00Z`);
  lookback.setUTCMonth(lookback.getUTCMonth() - 11, 1);
  let start = $state(lookback.toISOString().slice(0, 10));
  let end = $state(today);
  let account = $state("");
  let category = $state("");
  let data = $state<FinanceDashboard | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let activeLink = $state<FinanceDashboard["sankey"][number] | null>(null);
  let flowView = $state<"personal" | "cash">("personal");
  let flowMonth = $state("");

  async function load() {
    busy = true;
    try {
      data = await finance.dashboard({ start, end, account, category, currency: "EUR" });
      if (!flowMonth || !data.trend.some((point) => point.month === flowMonth)) {
        flowMonth = data.trend.at(-1)?.month ?? "";
      }
      error = null;
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busy = false;
    }
  }

  async function rebuild() {
    busy = true;
    try {
      await finance.rebuildLedger();
      await load();
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
      busy = false;
    }
  }

  function selectLink(link: FinanceDashboard["sankey"][number]) {
    account = link.account;
    category = link.category;
    void load();
  }

  function clearLinkFilter() {
    account = "";
    category = "";
    void load();
  }

  function money(cents: number, currency = "EUR") {
    return new Intl.NumberFormat("de-DE", { style: "currency", currency }).format(cents / 100);
  }

  function short(account: string) {
    return account.split(":").slice(1).join(" · ") || account;
  }

  const trendMax = $derived(Math.max(1, ...(data?.trend.flatMap((point) => flowView === "personal"
    ? [point.income_cents, point.personal_spending_cents]
    : [point.external_cash_inflow_cents, point.gross_cash_outflow_cents]) ?? [1])));
  const monthlyLinks = $derived((data?.sankey ?? []).filter((link) => link.month === flowMonth).sort((left, right) => right.amount_cents - left.amount_cents));
  const displayLinks = $derived(monthlyLinks.slice(0, 5));
  const otherFlowCents = $derived(monthlyLinks.slice(5).reduce((total, link) => total + link.amount_cents, 0));
  const incomeNodes = $derived([...new Set(displayLinks.filter((link) => link.source.startsWith("income:")).map((link) => link.source))]);
  const accountNodes = $derived([...new Set(displayLinks.map((link) => link.account))]);
  const expenseNodes = $derived([...new Set(displayLinks.filter((link) => link.target.startsWith("expenses:")).map((link) => link.target))]);
  const linkMax = $derived(Math.max(1, ...displayLinks.map((link) => link.amount_cents)));
  const portfolioValue = $derived(data?.portfolio_values.find((value) => value.currency === data?.summary.currency) ?? null);
  const portfolioLowerBound = $derived(data?.investment?.coverage === "partial" || (portfolioValue?.unpriced_holdings ?? 0) > 0);
  const outstandingShared = $derived(data?.shared_expenses.reduce((total, expense) => total + expense.outstanding_cents, 0) ?? 0);
  const nodeY = (nodes: string[], name: string) => 35 + (Math.max(0, nodes.indexOf(name)) + 0.5) * (290 / Math.max(1, nodes.length));
  const linkPath = (source: string, target: string) => {
    const income = source.startsWith("income:");
    const x1 = income ? 150 : 500;
    const x2 = income ? 500 : 850;
    const y1 = income ? nodeY(incomeNodes, source) : nodeY(accountNodes, source);
    const y2 = income ? nodeY(accountNodes, target) : nodeY(expenseNodes, target);
    return `M ${x1} ${y1} C ${(x1 + x2) / 2} ${y1}, ${(x1 + x2) / 2} ${y2}, ${x2} ${y2}`;
  };

  onMount(() => void load());
</script>

<form class="filters" onsubmit={(event) => { event.preventDefault(); void load(); }}>
  <label>From<input type="date" bind:value={start} /></label>
  <label>To<input type="date" bind:value={end} /></label>
  <label>Account<select bind:value={account}>
    <option value="">All accounts</option>
    {#each data?.accounts ?? [] as value (value)}<option value={value}>{short(value)}</option>{/each}
  </select></label>
  <button type="submit" disabled={busy}>Apply</button>
  {#if category}<button type="button" class="quiet" onclick={clearLinkFilter}>Clear Sankey filter</button>{/if}
  <button type="button" class="rebuild" disabled={busy} onclick={rebuild}>
    <Icon name="refresh" size={13} /> Rebuild journal index
  </button>
</form>

{#if error}
  <p class="error"><Icon name="alert" size={14} /> {error}</p>
{:else if !data}
  <p class="muted">Loading finance projection…</p>
{:else if mode === "overview"}
  <FinanceQuality {data} />

  <section class="cards primary-cards" aria-label="Finance summary">
    <article>
      <span>Tracked net worth</span>
      <strong>{data.tracked_net_worth === null ? "—" : exactMoney(data.tracked_net_worth.value.mantissa, data.tracked_net_worth.value.scale, data.tracked_net_worth.currency)}</strong>
      <small>{data.tracked_net_worth === null ? "Add a manual balance snapshot" : `${data.tracked_net_worth.complete ? "Complete" : "Incomplete"} · balances as of ${data.balance_snapshot?.as_of ?? "unknown"}${data.tracked_net_worth.portfolio_included ? " · reviewed portfolio included" : ""}`}</small>
    </article>
    <article><span>Personal spending</span><strong>{money(data.summary.personal_spending_cents)}</strong><small>Your reviewed share after shared-cost splits.</small></article>
    <article><span>Personal result</span><strong class:negative={data.summary.personal_result_cents < 0}>{money(data.summary.personal_result_cents)}</strong><small>Income minus your personal spending.</small></article>
    <article><span>External cash movement</span><strong class:negative={data.summary.external_cash_movement_cents < 0}>{money(data.summary.external_cash_movement_cents)}</strong><small>External inflow plus reimbursements minus gross payments.</small></article>
    <article>
      <span>Current commitments</span>
      <strong>{money(data.current_commitment_monthly_cents)}</strong>
      <small>Monthly obligations active on {data.commitment_as_of}.</small>
    </article>
  </section>

  <section class="secondary-facts" aria-label="Supporting finance facts">
    <span>Income <strong>{money(data.summary.income_cents)}</strong></span>
    <span>Gross payments <strong>{money(data.summary.gross_cash_outflow_cents)}</strong></span>
    <span>Reimbursements <strong>{money(data.summary.reimbursement_received_cents)}</strong></span>
    <span>Shared receivables <strong>{money(outstandingShared)}</strong></span>
    <span>Savings rate <strong>{data.summary.savings_rate_percent === null ? "—" : `${data.summary.savings_rate_percent.toFixed(1)}%`}</strong></span>
    <span>Reviewed portfolio <strong>{portfolioValue === null ? "—" : `${portfolioLowerBound ? "≥ " : ""}${exactMoney(portfolioValue.value.mantissa, portfolioValue.value.scale, portfolioValue.currency)}`}</strong></span>
  </section>

  <BalanceEditor snapshot={data.balance_snapshot} onsaved={load} />

  {#if data.commitments.length}
    <section class="panel commitments">
      <div class="panel-heading"><div><h2>Recurring commitments</h2><p>Forward-looking obligations, with effective dates.</p></div></div>
      <div class="commitment-list">
        {#each data.commitments as commitment (commitment.id)}
          <article class:inactive={commitment.valid_from > data.commitment_as_of || (commitment.valid_until !== null && commitment.valid_until < data.commitment_as_of)}>
            <div><strong>{commitment.label}</strong><small>{short(commitment.account)}</small></div>
            <span>{commitment.valid_from}{commitment.valid_until ? ` – ${commitment.valid_until}` : " onward"}</span>
            <strong>{money(commitment.monthly_cents, commitment.currency)} / month</strong>
          </article>
        {/each}
      </div>
    </section>
  {/if}

  <CategoryTrend {data} />
  <SpendingDrivers {data} throughDate={end} />
  <SpendingPurposeOverview {data} />

  <section class="panel trend">
    <div class="panel-heading">
      <div>
        <h2>{flowView === "personal" ? "Personal result by month" : "External cash movement by month"}</h2>
        <p>{flowView === "personal" ? "Income compared with your reviewed personal share of spending." : "External inflows and reimbursements compared with gross payments; internal transfers stay excluded."}</p>
      </div>
      <div class="segmented" aria-label="Monthly metric view">
        <button type="button" class:active={flowView === "personal"} onclick={() => flowView = "personal"}>Personal</button>
        <button type="button" class:active={flowView === "cash"} onclick={() => flowView = "cash"}>Cash</button>
      </div>
    </div>
      {#if data.trend.length}
        <div class="trend-rows">
          {#each data.trend as point (point.month)}
            <div class="trend-row">
              <time>{point.month}</time>
              <div class="bars">
                <span class="income" style:width={`${Math.max(1, (flowView === "personal" ? point.income_cents : point.external_cash_inflow_cents) / trendMax * 100)}%`} use:tip={`${flowView === "personal" ? "Income" : "External inflow"} ${money(flowView === "personal" ? point.income_cents : point.external_cash_inflow_cents)}`}></span>
                <span class="expense" style:width={`${Math.max(1, (flowView === "personal" ? point.personal_spending_cents : point.gross_cash_outflow_cents) / trendMax * 100)}%`} use:tip={`${flowView === "personal" ? "Personal spending" : "Gross payments"} ${money(flowView === "personal" ? point.personal_spending_cents : point.gross_cash_outflow_cents)}`}></span>
              </div>
              <strong class:negative={(flowView === "personal" ? point.personal_result_cents : point.external_cash_movement_cents) < 0}>{money(flowView === "personal" ? point.personal_result_cents : point.external_cash_movement_cents)}</strong>
            </div>
          {/each}
        </div>
        <p class="legend"><i class="income"></i> {flowView === "personal" ? "income" : "external inflow"} <i class="expense"></i> {flowView === "personal" ? "personal spending" : "gross payments"}; the value at right is the selected result.</p>
      {:else}<p class="muted">No transactions in this period.</p>{/if}
  </section>

  <details class="panel flow">
    <summary>
      <span><strong>Money-flow explorer</strong><small>Open a one-month, top-flow drill-down</small></span>
      <span>{flowMonth || "No month"}</span>
    </summary>
    <div class="flow-controls">
      <label>Month<select bind:value={flowMonth}>{#each data.trend as point (point.month)}<option value={point.month}>{point.month}</option>{/each}</select></label>
      {#if activeLink}<strong>{money(activeLink.amount_cents)}</strong>{/if}
    </div>
    {#if displayLinks.length}
      <svg viewBox="0 0 1000 360" role="img" aria-label="Interactive one-month money-flow Sankey">
        {#each displayLinks as link, index (`${link.month}-${link.source}-${link.target}`)}
          <a href={`#flow-${index}`} onclick={(event) => { event.preventDefault(); selectLink(link); }} onmouseenter={() => activeLink = link} onmouseleave={() => activeLink = null}>
            <path d={linkPath(link.source, link.target)} stroke-width={Math.max(3, link.amount_cents / linkMax * 25)} class:active={activeLink === link}>
              <title>{short(link.source)} to {short(link.target)}: {money(link.amount_cents)}</title>
            </path>
          </a>
        {/each}
        {#each incomeNodes as node (node)}<text x="140" y={nodeY(incomeNodes, node)} text-anchor="end">{short(node)}</text>{/each}
        {#each accountNodes as node (node)}<text x="500" y={nodeY(accountNodes, node)} text-anchor="middle">{short(node)}</text>{/each}
        {#each expenseNodes as node (node)}<text x="860" y={nodeY(expenseNodes, node)}>{short(node)}</text>{/each}
      </svg>
      <div class="flow-note">
        <p>Top five journal flows in {flowMonth}. Internal transfers are excluded; this is an analytical path view, not account reconciliation.</p>
        {#if otherFlowCents > 0}<strong>Other flows {money(otherFlowCents)}</strong>{/if}
      </div>
    {:else}<p class="muted">No income or expense flow in this month.</p>{/if}
  </details>

  <HoldingsPanel snapshot={data.investment} />

  <section class="panel recent">
    <h2>Recent transactions</h2>
    <TransactionTable rows={data.transactions.slice(0, 8)} />
  </section>
{:else if mode === "planning"}
  <PlanningPanel {data} {onnavigate} />
{:else}
  <CategorizationReview {data} {start} {end} {account} onchanged={load} />
  <SpendingContextReview {data} {start} {end} {account} {category} onchanged={load} />
  <section class="panel transactions">
    <div class="panel-heading"><div><h2>Transactions</h2><p>{data.transactions.length} normalized journal rows. Transfers are excluded from totals by default.</p></div></div>
    <TransactionTable rows={data.transactions} />
  </section>
  <ImportReview onchanged={load} />
  <InvestmentPreview onchanged={load} />
{/if}

<style>
  .filters { display: flex; flex-wrap: wrap; align-items: end; gap: .65rem; margin-bottom: 1.2rem; }
  label { display: flex; flex-direction: column; gap: .2rem; font-size: .68rem; color: var(--muted, #888); }
  input, select, button { border: 1px solid var(--border, #333); border-radius: 6px; padding: .38rem .55rem; font: inherit; font-size: .78rem; background: transparent; color: inherit; }
  button { cursor: pointer; }
  button:disabled { opacity: .45; }
  button.rebuild { display: inline-flex; align-items: center; gap: .3rem; margin-left: auto; }
  button.quiet { border-color: transparent; color: var(--muted, #888); }
  .cards { display: grid; grid-template-columns: repeat(auto-fit, minmax(11.5rem, 1fr)); gap: .65rem; margin-bottom: .75rem; }
  .cards article, .panel { border: 1px solid var(--border, #333); border-radius: 8px; background: var(--card-bg, transparent); }
  .cards article { display: flex; flex-direction: column; gap: .3rem; padding: .8rem; }
  .cards span, .panel p, .legend { color: var(--muted, #888); font-size: .7rem; }
  .cards strong { font-size: 1.25rem; font-variant-numeric: tabular-nums; }
  .cards small { color: var(--muted, #888); font-size: .62rem; line-height: 1.35; }
  .negative { color: var(--danger, #b44); }
  .secondary-facts { display: flex; flex-wrap: wrap; gap: .45rem 1.1rem; padding: .25rem 0 .85rem; color: var(--muted, #888); font-size: .66rem; }
  .secondary-facts span { display: inline-flex; align-items: baseline; gap: .3rem; }
  .secondary-facts strong { color: inherit; font-variant-numeric: tabular-nums; }
  .panel { padding: .9rem; min-width: 0; }
  .panel h2 { margin: 0; font-size: .85rem; }
  .panel p { margin: .2rem 0 0; }
  .panel-heading { display: flex; align-items: start; justify-content: space-between; gap: 1rem; margin-bottom: .75rem; }
  .trend { margin-bottom: .75rem; }
  .segmented { display: inline-flex; padding: 2px; border: 1px solid var(--border, #333); border-radius: 6px; }
  .segmented button { border: 0; padding: .28rem .55rem; font-size: .68rem; color: var(--muted, #888); }
  .segmented button.active { background: color-mix(in srgb, var(--primary) 12%, transparent); color: inherit; }
  .trend-rows { display: grid; gap: .55rem; margin-top: 1rem; }
  .trend-row { display: grid; grid-template-columns: 4.5rem 1fr 6.5rem; align-items: center; gap: .6rem; font-size: .7rem; }
  .trend-row time { color: var(--muted, #888); }
  .trend-row > strong { text-align: right; font-variant-numeric: tabular-nums; }
  .bars { display: grid; gap: 3px; }
  .bars span { display: block; height: 5px; border-radius: 3px; }
  .income { background: var(--primary); }
  .expense { background: #9b7653; }
  .legend { display: flex; align-items: center; gap: .35rem; }
  .legend i { width: .55rem; height: .35rem; display: inline-block; }
  svg { width: 100%; min-height: 17rem; overflow: visible; }
  svg path { fill: none; stroke: var(--primary); opacity: .28; transition: opacity var(--motion-fast); cursor: pointer; }
  svg path:hover, svg path.active { opacity: .85; }
  svg text { fill: currentColor; font-size: 14px; dominant-baseline: middle; pointer-events: none; }
  .flow { margin-bottom: .75rem; }
  .flow summary { display: flex; align-items: center; justify-content: space-between; gap: 1rem; cursor: pointer; list-style: none; }
  .flow summary::-webkit-details-marker { display: none; }
  .flow summary > span:first-child { display: grid; gap: .12rem; }
  .flow summary small, .flow summary > span:last-child { color: var(--muted, #888); font-size: .66rem; }
  .flow-controls, .flow-note { display: flex; align-items: end; justify-content: space-between; gap: 1rem; margin-top: .8rem; }
  .flow-note { align-items: baseline; border-top: 1px solid var(--border, #333); padding-top: .6rem; }
  .flow-note > strong { white-space: nowrap; font-size: .7rem; font-variant-numeric: tabular-nums; }
  .recent { margin-top: .75rem; }
  .transactions { margin-top: .75rem; }
  .commitments { margin-bottom: .75rem; }
  .commitment-list { display: grid; gap: .5rem; }
  .commitment-list article { display: grid; grid-template-columns: minmax(12rem, 1fr) auto auto; align-items: center; gap: 1rem; padding: .55rem 0; border-top: 1px solid var(--border, #333); font-size: .72rem; }
  .commitment-list article > div { display: flex; flex-direction: column; gap: .15rem; }
  .commitment-list small, .commitment-list span { color: var(--muted, #888); }
  .commitment-list article > strong { font-variant-numeric: tabular-nums; text-align: right; }
  .commitment-list article.inactive { opacity: .62; }
  .error { display: flex; align-items: center; gap: .35rem; }
  .muted { color: var(--muted, #888); }
  @media (max-width: 850px) { button.rebuild { margin-left: 0; } .commitment-list article { grid-template-columns: 1fr; gap: .4rem; } .commitment-list article > strong { text-align: left; } }
  @media (max-width: 600px) { .panel-heading, .flow-controls, .flow-note { align-items: start; flex-direction: column; } .trend-row { grid-template-columns: 3.8rem 1fr; } .trend-row > strong { grid-column: 2; text-align: left; } }
</style>
