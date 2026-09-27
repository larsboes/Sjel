<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import {
    axonStatus,
    finance,
    trips,
    type FinanceDashboard,
    type SpendingPurpose,
    type TransactionCandidate,
    type TripPlan,
  } from "$lib/api";
  import {
    parsePersonalCents,
    presetShareCents,
    prioritizedExpenseQueue,
    reviewableExpenses,
    type ExpenseReview,
    type ReviewQueueMode,
  } from "$lib/finance/spending-review";

  let { data, start, end, account, category, onchanged }: {
    data: FinanceDashboard;
    start: string;
    end: string;
    account: string;
    category: string;
    onchanged?: () => void | Promise<void>;
  } = $props();
  let candidates = $state<TransactionCandidate[]>([]);
  let plans = $state<TripPlan[]>([]);
  let queueMode = $state<ReviewQueueMode>("unreviewed");
  let reviewTripId = $state("");
  let expenseId = $state("");
  let personalAmount = $state("");
  let purpose = $state<SpendingPurpose>("day_to_day");
  let tripId = $state("");
  let inflowId = $state("");
  let reimbursementTarget = $state("");
  let visibleLimit = $state(16);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let notice = $state<string | null>(null);

  const purposeLabels: Record<SpendingPurpose, string> = {
    day_to_day: "Day to day",
    trip: "Trip",
    work: "Work",
    housing: "Housing",
    other: "Other",
  };
  const selectedPlan = $derived(plans.find((plan) => plan.id === reviewTripId) ?? null);
  const eligibleExpenses = $derived(reviewableExpenses(
    candidates,
    data.transactions,
    { start, end, account, category },
  ));
  const awaitingCategoryCount = $derived(candidates.filter((candidate) => candidate.state === "confirmed"
    && candidate.amount_cents < 0
    && candidate.booked_at >= start
    && candidate.booked_at <= end
    && (account === "" || candidate.source_account === account)
    && candidate.proposed_account.split(":").includes("uncategorized")).length);
  const reviewQueue = $derived(prioritizedExpenseQueue(eligibleExpenses, queueMode, selectedPlan));
  const visibleQueue = $derived(reviewQueue.slice(0, visibleLimit));
  const selectedReview = $derived(eligibleExpenses.find((entry) => entry.candidate.id === expenseId) ?? null);
  const unreviewedCount = $derived(eligibleExpenses.filter((entry) => !entry.reviewed).length);
  const reviewedCount = $derived(eligibleExpenses.length - unreviewedCount);
  const purposeCoverage = $derived(eligibleExpenses.length === 0 ? null : reviewedCount / eligibleExpenses.length * 100);
  const inflows = $derived(candidates.filter((candidate) => candidate.state === "confirmed"
    && candidate.amount_cents > 0
    && candidate.booked_at >= start
    && candidate.booked_at <= end
    && (account === "" || candidate.source_account === account)
    && candidate.proposed_account !== "assets:receivable:shared")
    .sort((left, right) => right.booked_at.localeCompare(left.booked_at)));
  const openReceivables = $derived(data.shared_expenses
    .filter((expense) => expense.outstanding_cents > 0
      && (selectedPlan === null || expense.trip_id === selectedPlan.id))
    .sort((left, right) => right.outstanding_cents - left.outstanding_cents));
  const selectedInflow = $derived(inflows.find((candidate) => candidate.id === inflowId) ?? null);
  const selectedTarget = $derived(openReceivables.find((expense) => expense.candidate_id === reimbursementTarget) ?? null);
  const focusedTripRows = $derived(selectedPlan === null ? [] : eligibleExpenses.filter((entry) =>
    entry.transaction?.trip_id === selectedPlan.id,
  ));
  const focusedTripPersonalCents = $derived(focusedTripRows.reduce((sum, entry) =>
    sum + (entry.transaction?.amount_cents ?? 0), 0));
  const focusedTripGrossCents = $derived(focusedTripRows.reduce((sum, entry) =>
    sum + (entry.transaction?.cash_amount_cents ?? 0), 0));
  const focusedTripOutstandingCents = $derived(openReceivables.reduce((sum, expense) =>
    sum + expense.outstanding_cents, 0));

  const money = (cents: number, currency = data.summary.currency) =>
    new Intl.NumberFormat("de-DE", { style: "currency", currency }).format(cents / 100);
  const shortAccount = (value: string) => value.split(":").slice(1).join(" · ") || value;

  async function load() {
    try {
      candidates = await finance.candidates();
      try {
        plans = await trips.list();
      } catch {
        await axonStatus.start("trips").catch(() => undefined);
        plans = await trips.list().catch(() => []);
      }
      error = null;
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    }
  }

  function chooseQueueMode(event: Event) {
    queueMode = (event.currentTarget as HTMLSelectElement).value as ReviewQueueMode;
    visibleLimit = 16;
    clearExpense();
  }

  function chooseTrip(event: Event) {
    reviewTripId = (event.currentTarget as HTMLSelectElement).value;
    visibleLimit = 16;
    clearExpense();
  }

  function clearExpense() {
    expenseId = "";
    personalAmount = "";
    purpose = reviewTripId ? "trip" : "day_to_day";
    tripId = reviewTripId;
  }

  function selectExpense(entry: ExpenseReview) {
    expenseId = entry.candidate.id;
    personalAmount = ((entry.transaction?.amount_cents ?? entry.totalCents) / 100)
      .toFixed(2)
      .replace(".", ",");
    purpose = entry.transaction?.purpose ?? (reviewTripId ? "trip" : "day_to_day");
    tripId = entry.transaction?.trip_id ?? reviewTripId;
    error = null;
    notice = null;
  }

  function choosePurpose(event: Event) {
    purpose = (event.currentTarget as HTMLSelectElement).value as SpendingPurpose;
    if (purpose !== "trip") tripId = "";
    else if (!tripId) tripId = reviewTripId;
  }

  function setShare(percent: number) {
    if (!selectedReview) return;
    personalAmount = (presetShareCents(selectedReview.totalCents, percent) / 100)
      .toFixed(2)
      .replace(".", ",");
  }

  function personalCents(): number | null {
    return parsePersonalCents(personalAmount);
  }

  function shareIsValid() {
    const cents = personalCents();
    return selectedReview !== null && cents !== null && cents >= 0 && cents <= selectedReview.totalCents;
  }

  async function saveAllocation() {
    const cents = personalCents();
    if (!selectedReview || cents === null || !shareIsValid()) return;
    busy = true;
    error = null;
    notice = null;
    try {
      await finance.allocateExpense(
        selectedReview.candidate.id,
        cents,
        purpose,
        purpose === "trip" ? tripId : null,
      );
      notice = "Purpose and personal share saved; Finance was rebuilt from the reviewed journal.";
      await onchanged?.();
      await load();
      clearExpense();
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busy = false;
    }
  }

  async function saveReimbursement() {
    if (!selectedInflow || !selectedTarget) return;
    busy = true;
    error = null;
    notice = null;
    try {
      await finance.linkReimbursement(selectedInflow.id, selectedTarget.candidate_id);
      notice = "Repayment linked to the original shared expense; it no longer counts as income.";
      inflowId = "";
      reimbursementTarget = "";
      await onchanged?.();
      await load();
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busy = false;
    }
  }

  onMount(() => void load());
</script>

<section class="context-review">
  <div class="heading">
    <div>
      <h2>Purpose and shared-cost review</h2>
      <p>Largest unresolved expenses first. Record why you spent it, which trip it belongs to, and only the share that was yours.</p>
    </div>
    <div class="coverage">
      <strong>{purposeCoverage === null ? "—" : `${purposeCoverage.toFixed(1)}%`}</strong>
      <span>{unreviewedCount} need review · {awaitingCategoryCount} need category first</span>
    </div>
  </div>

  <div class="queue-controls">
    <label>Queue
      <select value={queueMode} onchange={chooseQueueMode}>
        <option value="unreviewed">Needs purpose</option>
        <option value="shared">Shared costs</option>
        <option value="all">All categorized expenses</option>
      </select>
    </label>
    <label>Trip focus
      <select value={reviewTripId} onchange={chooseTrip}>
        <option value="">Any purpose or trip</option>
        {#each plans as plan (plan.id)}
          <option value={plan.id}>{plan.title} · {plan.date_start}–{plan.date_end}</option>
        {/each}
      </select>
    </label>
    <div class="queue-scope">
      <span>{start}–{end}</span>
      <strong>{reviewQueue.length} expense{reviewQueue.length === 1 ? "" : "s"}</strong>
    </div>
  </div>

  {#if error}<p class="error"><Icon name="alert" size={14} /> {error}</p>{/if}
  {#if notice}<p class="notice">{notice}</p>{/if}

  {#if selectedPlan}
    <div class="trip-strip">
      <div><strong>{selectedPlan.title}</strong><span>{selectedPlan.date_start}–{selectedPlan.date_end}</span></div>
      <dl>
        <div><dt>Reviewed personal</dt><dd>{money(focusedTripPersonalCents)}</dd></div>
        <div><dt>Gross paid</dt><dd>{money(focusedTripGrossCents)}</dd></div>
        <div><dt>Still owed</dt><dd>{money(focusedTripOutstandingCents)}</dd></div>
        <div><dt>Assigned</dt><dd>{focusedTripRows.length}</dd></div>
      </dl>
    </div>
  {/if}

  <div class="review-workspace">
    <div class="expense-queue">
      {#each visibleQueue as entry (entry.candidate.id)}
        <button type="button" class:selected={entry.candidate.id === expenseId} onclick={() => selectExpense(entry)}>
          <time>{entry.candidate.booked_at}</time>
          <span class="expense-main">
            <strong>{entry.candidate.description}</strong>
            <small>{shortAccount(entry.candidate.proposed_account)}</small>
          </span>
          <span class="context-state">
            {#if entry.reviewed}
              <strong>{purposeLabels[entry.transaction?.purpose ?? "other"]}</strong>
              <small>{entry.transaction?.shared_cents ? `${money(entry.transaction.shared_cents, entry.candidate.currency)} shared` : "full amount yours"}</small>
            {:else}
              <strong>Needs purpose</strong>
              <small>full amount assumed until reviewed</small>
            {/if}
          </span>
          <strong class="amount">{money(entry.totalCents, entry.candidate.currency)}</strong>
        </button>
      {/each}
      {#if reviewQueue.length === 0}<p class="empty">Nothing is waiting in this queue.</p>{/if}
      {#if visibleLimit < reviewQueue.length}
        <button class="show-more" type="button" onclick={() => visibleLimit += 16}>Show 16 more</button>
      {/if}
    </div>

    <form class="allocation-editor" onsubmit={(event) => { event.preventDefault(); void saveAllocation(); }}>
      <div class="editor-heading">
        <div><h3>{selectedReview ? "Review selected expense" : "Select an expense"}</h3><p>Saving rewrites only this Sjel-owned journal entry.</p></div>
        {#if selectedReview}<strong>{money(selectedReview.totalCents, selectedReview.candidate.currency)}</strong>{/if}
      </div>
      {#if selectedReview}
        <div class="selected-expense">
          <time>{selectedReview.candidate.booked_at}</time>
          <strong>{selectedReview.candidate.description}</strong>
          <span>{shortAccount(selectedReview.candidate.proposed_account)}</span>
        </div>
        <label>Purpose
          <select value={purpose} onchange={choosePurpose}>
            {#each Object.entries(purposeLabels) as [value, label]}
              <option value={value}>{label}</option>
            {/each}
          </select>
        </label>
        {#if purpose === "trip"}
          <label>Trips plan
            <select bind:value={tripId}>
              <option value="">Select a plan</option>
              {#each plans as plan (plan.id)}
                <option value={plan.id}>{plan.title} · {plan.date_start}–{plan.date_end}</option>
              {/each}
            </select>
          </label>
        {/if}
        <label>Your final cost
          <input inputmode="decimal" bind:value={personalAmount} placeholder="0,00" />
        </label>
        <div class="share-presets" aria-label="Personal share presets">
          <button type="button" onclick={() => setShare(100)}>All mine</button>
          <button type="button" onclick={() => setShare(50)}>1 / 2</button>
          <button type="button" onclick={() => setShare(25)}>1 / 4</button>
          <button type="button" onclick={() => setShare(0)}>Fronted all</button>
        </div>
        {#if personalCents() !== null}
          <div class:error-calculation={!shareIsValid()} class="calculation">
            <span>Gross paid <strong>{money(selectedReview.totalCents, selectedReview.candidate.currency)}</strong></span>
            <span>Your cost <strong>{money(personalCents() ?? 0, selectedReview.candidate.currency)}</strong></span>
            <span>Receivable <strong>{money(Math.max(0, selectedReview.totalCents - (personalCents() ?? 0)), selectedReview.candidate.currency)}</strong></span>
          </div>
        {/if}
        <button type="submit" class="save" disabled={busy || !shareIsValid() || (purpose === "trip" && !tripId)}>
          Save purpose and share
        </button>
      {:else}
        <p class="empty editor-empty">Choose a row to review its purpose and personal share.</p>
      {/if}
    </form>
  </div>

  <section class="receivables">
    <div class="receivable-heading">
      <div><h3>Open shared costs</h3><p>Link money received back to the original expense rather than counting it as income.</p></div>
      <strong>{money(openReceivables.reduce((sum, expense) => sum + expense.outstanding_cents, 0))} open</strong>
    </div>
    <div class="receivable-grid">
      <div class="receivable-list">
        {#each openReceivables.slice(0, 8) as expense (expense.source_id)}
          <article>
            <time>{expense.date}</time>
            <span><strong>{expense.description}</strong><small>{expense.trip_id ? plans.find((plan) => plan.id === expense.trip_id)?.title ?? "Linked trip" : purposeLabels[expense.purpose ?? "other"]}</small></span>
            <strong>{money(expense.outstanding_cents, expense.currency)}</strong>
          </article>
        {/each}
        {#if openReceivables.length === 0}<p class="empty">No open receivables in this scope.</p>{/if}
      </div>
      <form class="repayment" onsubmit={(event) => { event.preventDefault(); void saveReimbursement(); }}>
        <label>Confirmed inflow
          <select bind:value={inflowId}>
            <option value="">Select money received back</option>
            {#each inflows as inflow (inflow.id)}
              <option value={inflow.id}>{inflow.booked_at} · {inflow.description} · {money(inflow.amount_cents, inflow.currency)}</option>
            {/each}
          </select>
        </label>
        <label>Original shared expense
          <select bind:value={reimbursementTarget}>
            <option value="">Select an open receivable</option>
            {#each openReceivables as expense (expense.source_id)}
              <option value={expense.candidate_id}>{expense.date} · {expense.description} · {money(expense.outstanding_cents, expense.currency)} open</option>
            {/each}
          </select>
        </label>
        {#if selectedInflow && selectedTarget && selectedInflow.currency !== selectedTarget.currency}
          <p class="error">The repayment and original expense use different currencies.</p>
        {:else if selectedInflow && selectedTarget && selectedInflow.amount_cents > selectedTarget.outstanding_cents}
          <p class="error">This inflow is larger than the selected outstanding receivable.</p>
        {/if}
        <button type="submit" disabled={busy || !selectedInflow || !selectedTarget || selectedInflow.currency !== selectedTarget.currency || selectedInflow.amount_cents > selectedTarget.outstanding_cents}>
          Link repayment
        </button>
      </form>
    </div>
  </section>
</section>

<style>
  .context-review { min-width: 0; border: 1px solid var(--border, #333); border-radius: 8px; padding: .9rem; }
  .heading, .editor-heading, .receivable-heading { display: flex; align-items: start; justify-content: space-between; gap: 1rem; }
  h2, h3 { margin: 0; }
  h2 { font-size: .95rem; }
  h3 { font-size: .78rem; }
  p { margin: .2rem 0 0; }
  .heading p, .editor-heading p, .receivable-heading p, .empty { color: var(--muted, #888); font-size: .68rem; }
  .coverage { display: grid; justify-items: end; white-space: nowrap; }
  .coverage strong { font-size: 1rem; font-variant-numeric: tabular-nums; }
  .coverage span, .queue-scope span { color: var(--muted, #888); font-size: .64rem; }
  .queue-controls { display: grid; grid-template-columns: minmax(10rem, .55fr) minmax(16rem, 1fr) auto; align-items: end; gap: .6rem; margin-top: .8rem; padding-top: .75rem; border-top: 1px solid var(--border, #333); }
  label { display: grid; gap: .25rem; min-width: 0; color: var(--muted, #888); font-size: .66rem; }
  input, select, button { min-width: 0; border: 1px solid var(--border, #333); border-radius: 6px; padding: .4rem .52rem; font: inherit; font-size: .72rem; background: transparent; color: inherit; }
  button { cursor: pointer; }
  button:disabled { opacity: .45; cursor: default; }
  .queue-scope { display: grid; justify-items: end; gap: .15rem; padding-bottom: .35rem; }
  .queue-scope strong { font-size: .72rem; }
  .trip-strip { display: grid; grid-template-columns: minmax(12rem, .65fr) minmax(30rem, 1.35fr); align-items: center; gap: 1rem; margin-top: .7rem; padding: .6rem .7rem; border: 1px solid var(--border, #333); border-radius: 7px; }
  .trip-strip > div { display: grid; gap: .15rem; }
  .trip-strip > div span, .selected-expense span { color: var(--muted, #888); font-size: .64rem; }
  .trip-strip dl { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); margin: 0; }
  .trip-strip dl div { display: grid; gap: .15rem; padding-left: .65rem; border-left: 1px solid var(--border, #333); }
  .trip-strip dt { color: var(--muted, #888); font-size: .6rem; }
  .trip-strip dd { margin: 0; font-size: var(--text-xs); font-weight: 650; font-variant-numeric: tabular-nums; }
  .review-workspace { display: grid; grid-template-columns: minmax(26rem, 1.25fr) minmax(19rem, .75fr); gap: .75rem; margin-top: .75rem; }
  .expense-queue, .allocation-editor, .receivables { min-width: 0; border: 1px solid var(--border, #333); border-radius: 7px; }
  .expense-queue { max-height: 27rem; overflow: auto; }
  .expense-queue > button:not(.show-more) { width: 100%; display: grid; grid-template-columns: 5.4rem minmax(11rem, 1fr) 9rem 6.5rem; align-items: center; gap: .65rem; padding: .55rem .65rem; border: 0; border-bottom: 1px solid var(--border, #333); border-radius: 0; text-align: left; }
  .expense-queue > button.selected { background: color-mix(in srgb, var(--primary) 9%, transparent); }
  .expense-queue time, .selected-expense time, .receivable-list time { color: var(--muted, #888); font-size: .64rem; }
  .expense-main, .context-state { min-width: 0; display: grid; gap: .13rem; }
  .expense-main strong, .selected-expense strong, .receivable-list span strong { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .expense-main small, .context-state small, .receivable-list small { color: var(--muted, #888); font-size: .6rem; }
  .context-state { text-align: right; }
  .context-state strong { font-size: .66rem; font-weight: 600; }
  .amount { text-align: right; font-variant-numeric: tabular-nums; }
  .show-more { margin: .55rem; }
  .allocation-editor { display: grid; align-content: start; gap: .65rem; padding: .75rem; }
  .editor-heading > strong { font-size: .85rem; font-variant-numeric: tabular-nums; }
  .selected-expense { display: grid; grid-template-columns: auto minmax(8rem, 1fr); gap: .15rem .55rem; padding: .5rem .55rem; background: color-mix(in srgb, currentColor 4%, transparent); font-size: .68rem; }
  .selected-expense span { grid-column: 2; }
  .share-presets { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: .35rem; }
  .share-presets button { padding-inline: .25rem; }
  .calculation { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: .4rem; padding: .5rem; border: 1px solid var(--border, #333); border-radius: 6px; }
  .calculation span { display: grid; gap: .12rem; color: var(--muted, #888); font-size: .6rem; }
  .calculation strong { color: inherit; font-size: .7rem; font-variant-numeric: tabular-nums; }
  .error-calculation { border-color: var(--danger, #b44); }
  .save { justify-self: start; }
  .editor-empty { min-height: 8rem; display: grid; place-items: center; text-align: center; }
  .receivables { margin-top: .75rem; padding: .75rem; }
  .receivable-heading > strong { font-size: .78rem; font-variant-numeric: tabular-nums; }
  .receivable-grid { display: grid; grid-template-columns: minmax(24rem, 1.2fr) minmax(18rem, .8fr); gap: .8rem; margin-top: .65rem; }
  .receivable-list article { display: grid; grid-template-columns: 5.2rem minmax(10rem, 1fr) 6.5rem; align-items: center; gap: .55rem; padding: .4rem 0; border-bottom: 1px solid var(--border, #333); font-size: .66rem; }
  .receivable-list span { min-width: 0; display: grid; gap: .12rem; }
  .receivable-list > article > strong { text-align: right; font-variant-numeric: tabular-nums; }
  .repayment { display: grid; align-content: start; gap: .55rem; }
  .repayment button { justify-self: start; }
  .error { color: var(--danger, #b44); font-size: .7rem; }
  .notice { color: var(--primary); font-size: .7rem; }
  .empty { padding: .7rem; }
  @media (max-width: 900px) {
    .review-workspace, .receivable-grid, .trip-strip { grid-template-columns: 1fr; }
    .trip-strip dl div:first-child { border-left: 0; padding-left: 0; }
  }
  @media (max-width: 650px) {
    .heading { flex-direction: column; }
    .coverage { justify-items: start; }
    .queue-controls { grid-template-columns: 1fr; }
    .queue-scope { justify-items: start; }
    .trip-strip dl { grid-template-columns: 1fr 1fr; gap: .55rem 0; }
    .trip-strip dl div:nth-child(3) { border-left: 0; padding-left: 0; }
    .expense-queue > button:not(.show-more) { grid-template-columns: 4.7rem minmax(9rem, 1fr) auto; }
    .context-state { grid-column: 2; text-align: left; }
    .amount { grid-column: 3; grid-row: 1; }
    .share-presets { grid-template-columns: 1fr 1fr; }
    .calculation { grid-template-columns: 1fr; }
  }
</style>
