<script lang="ts">
  import Icon from "$lib/Icon.svelte";
  import {
    finance,
    type CsvDateFormat,
    type CsvImportPreview,
    type CsvMapping,
    type CsvMappingProfile,
    type TransactionCandidate,
  } from "$lib/api";
  import { selectedCsvMapping } from "$lib/finance/csv-mapping";

  let { onchanged }: { onchanged?: () => void } = $props();
  let candidates = $state<TransactionCandidate[]>([]);
  let content = $state("");
  let filename = $state("");
  let busy = $state(false);
  let error = $state<string | null>(null);
  let notice = $state<string | null>(null);
  let accounts = $state<Record<string, string>>({});
  let sourceAccounts = $state<Record<string, string>>({});
  let mappingProfiles = $state<CsvMappingProfile[]>([]);
  let selectedProfile = $state("");
  let preview = $state<CsvImportPreview | null>(null);
  let previewKey = $state("");
  let mapping = $state<CsvMapping>({
    delimiter: ";",
    decimal_separator: ",",
    date_column: "Buchung",
    amount_column: "Betrag",
    description_column: "Verwendungszweck",
    categorization_columns: [],
    reference_column: null,
    currency_column: "Währung",
    default_currency: "EUR",
    source_account: "assets:bank:checking",
    default_outflow_account: "expenses:uncategorized",
    default_inflow_account: "income:uncategorized",
    categorization_rules: [],
    row_filter: null,
    amount_sign: "as_provided",
    amount_rounding: "reject",
    date_formats: ["day_month_year_dots"],
    row_policy: "strict",
  });

  async function load() {
    try {
      const [nextCandidates, nextProfiles] = await Promise.all([
        finance.candidates(),
        finance.csvMappings(),
      ]);
      candidates = nextCandidates;
      mappingProfiles = nextProfiles;
      accounts = Object.fromEntries(nextCandidates.map((candidate) => [
        candidate.id,
        accounts[candidate.id] ?? candidate.proposed_account,
      ]));
      sourceAccounts = Object.fromEntries(nextCandidates.map((candidate) => [
        candidate.id,
        sourceAccounts[candidate.id] ?? (
          candidate.source_account === "review:source-account-required" ? "" : candidate.source_account
        ),
      ]));
      error = null;
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    }
  }

  async function choose(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    filename = file.name;
    content = await file.text();
    preview = null;
    previewKey = "";
  }

  function selectMapping(event: Event) {
    selectedProfile = (event.currentTarget as HTMLSelectElement).value;
    const selected = selectedCsvMapping(mappingProfiles, selectedProfile);
    if (selected) {
      mapping = selected;
      preview = null;
      previewKey = "";
    }
  }

  function selectDateFormat(event: Event) {
    mapping.date_formats = [(event.currentTarget as HTMLSelectElement).value as CsvDateFormat];
  }

  function isMailCandidate(candidate: TransactionCandidate): boolean {
    return candidate.source_reference?.startsWith("gmail:") ?? false;
  }

  function gmailUrl(candidate: TransactionCandidate): string | null {
    const id = candidate.source_reference?.startsWith("gmail:")
      ? candidate.source_reference.slice("gmail:".length)
      : "";
    return /^[a-f0-9]{1,128}$/i.test(id)
      ? `https://mail.google.com/mail/u/0/#all/${encodeURIComponent(id)}`
      : null;
  }

  function normalizedMapping(): CsvMapping {
    return {
      ...mapping,
      reference_column: mapping.reference_column?.trim() || null,
      currency_column: mapping.currency_column?.trim() || null,
    };
  }

  function currentPreviewKey(): string {
    return JSON.stringify([content, normalizedMapping()]);
  }

  async function previewImport() {
    if (!content) return;
    busy = true;
    error = null;
    notice = null;
    try {
      preview = await finance.previewCsv(content, normalizedMapping());
      previewKey = currentPreviewKey();
    } catch (cause) {
      preview = null;
      previewKey = "";
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busy = false;
    }
  }

  async function stage() {
    if (!content || !preview || previewKey !== currentPreviewKey()) return;
    busy = true;
    error = null;
    notice = null;
    try {
      const result = await finance.importCsv(
        content,
        normalizedMapping(),
        preview.preview_id,
      );
      notice = `${result.created} staged, ${result.already_present} already known. ${result.preserved_repetitions} repeated reference-less rows were preserved; ${result.duplicate_rows} stable-reference duplicates and ${result.ignored_non_transaction_rows} non-transaction rows were not staged. Raw CSV rows were discarded.`;
      content = "";
      filename = "";
      preview = null;
      previewKey = "";
      await load();
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busy = false;
    }
  }

  async function review(candidate: TransactionCandidate, decision: "confirm" | "reject") {
    busy = true;
    notice = null;
    try {
      const result = await finance.reviewCandidate(
        candidate.id,
        decision,
        decision === "confirm" ? accounts[candidate.id] : undefined,
        decision === "confirm" && isMailCandidate(candidate)
          ? sourceAccounts[candidate.id]
          : undefined,
      );
      notice = result.state === "confirmed"
        ? result.journal_written
          ? "Confirmed, written to the journal, and projection rebuilt."
          : "Already present in the journal; review state repaired and projection rebuilt."
        : "Rejected. The journal was not changed.";
      await load();
      onchanged?.();
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busy = false;
    }
  }

  async function reconcile(candidate: TransactionCandidate) {
    const counterpartId = candidate.transfer_match_ids[0];
    if (!counterpartId) return;
    busy = true;
    error = null;
    notice = null;
    try {
      const result = await finance.reconcileTransfer(candidate.id, counterpartId);
      notice = result.journal_written
        ? "Transfer reconciled as one journal posting; the counterpart is retained as duplicate evidence."
        : "Transfer was already in the journal; the counterpart is now retained as duplicate evidence.";
      await load();
      onchanged?.();
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busy = false;
    }
  }

  $effect(() => { void load(); });
</script>

<section class="review">
  <div class="heading">
    <div>
      <h2>Import and review</h2>
      <p>Choose an export locally, map its columns, then review source and expense accounts. Nothing confirms itself.</p>
    </div>
    <label class="file">
      <Icon name="plus" size={14} /> {filename || "Choose CSV"}
      <input type="file" accept=".csv,text/csv" onchange={choose} />
    </label>
  </div>

  {#if content}
    <div class="mapping">
      {#if mappingProfiles.length > 0}
        <label class="profile">Mapping profile
          <select value={selectedProfile} onchange={selectMapping}>
            <option value="">Manual entry</option>
            {#each mappingProfiles as profile, index (index)}
              <option value={index}>{profile.label}</option>
            {/each}
          </select>
        </label>
      {/if}
      <label>Delimiter<input maxlength="1" bind:value={mapping.delimiter} /></label>
      <label>Decimal separator<input maxlength="1" bind:value={mapping.decimal_separator} /></label>
      <label>Date column<input bind:value={mapping.date_column} /></label>
      <label>Amount column<input bind:value={mapping.amount_column} /></label>
      <label>Description column<input bind:value={mapping.description_column} /></label>
      <label>Reference column<input bind:value={mapping.reference_column} placeholder="optional" /></label>
      <label>Currency column<input bind:value={mapping.currency_column} placeholder="optional" /></label>
      <label>Default currency<input maxlength="3" bind:value={mapping.default_currency} /></label>
      <label>Amount sign
        <select bind:value={mapping.amount_sign}>
          <option value="as_provided">As provided</option>
          <option value="invert">Invert</option>
        </select>
      </label>
      <label>Date format
        <select value={mapping.date_formats[0]} onchange={selectDateFormat}>
          <option value="iso_year_month_day">YYYY-MM-DD</option>
          <option value="day_month_year_dots">DD.MM.YYYY</option>
          <option value="day_month_year_slashes">DD/MM/YYYY</option>
        </select>
      </label>
      <label>Row policy
        <select bind:value={mapping.row_policy}>
          <option value="strict">Strict shape</option>
          <option value="required_fields">Required fields</option>
        </select>
      </label>
      <label class="account">Source account<input bind:value={mapping.source_account} /></label>
      <button disabled={busy} onclick={previewImport}>Preview candidates</button>
    </div>
    {#if preview && previewKey === currentPreviewKey()}
      <div class="preview" aria-live="polite">
        <span>{preview.candidate_count} candidates</span>
        <span>{preview.outflow_count} outflows</span>
        <span>{preview.inflow_count} inflows</span>
        <span>{preview.preserved_repetitions} repetitions preserved</span>
        <span>{preview.duplicate_rows} reference duplicates</span>
        <span>{preview.ignored_non_transaction_rows} ignored rows</span>
        <button disabled={busy} onclick={stage}>Stage reviewed preview</button>
      </div>
    {/if}
  {/if}

  {#if error}<p class="error">{error}</p>{/if}
  {#if notice}<p class="notice">{notice}</p>{/if}

  {#if candidates.length > 0}
    <div class="candidate-list">
      {#each candidates as candidate (candidate.id)}
        <article class:resolved={candidate.state !== "pending"}>
          <div class="candidate-main">
            <time>{candidate.booked_at}</time>
            <strong>{candidate.description}</strong>
            {#if gmailUrl(candidate)}<a class="source-mail" href={gmailUrl(candidate)!} target="_blank" rel="noreferrer">Open email</a>{/if}
            <span class:outflow={candidate.amount_cents < 0} class="amount">
              {new Intl.NumberFormat("de-DE", { style: "currency", currency: candidate.currency }).format(candidate.amount_cents / 100)}
            </span>
          </div>
          {#if candidate.state === "pending"}
            {#if candidate.transfer_match_ids.length === 1}
              <div class="decision transfer-match">
                <span>Matching transfer found · one posting</span>
                <button disabled={busy} onclick={() => reconcile(candidate)}><Icon name="swap" size={14} /> Reconcile</button>
                <button class="reject" disabled={busy} onclick={() => review(candidate, "reject")}>Reject</button>
              </div>
            {:else}
              <div class="decision">
                {#if candidate.transfer_match_ids.length > 1}<span class="ambiguous">{candidate.transfer_match_ids.length} possible transfer matches</span>{/if}
                <input aria-label="Expense account" bind:value={accounts[candidate.id]} />
                {#if isMailCandidate(candidate)}
                  <input aria-label="Source account" bind:value={sourceAccounts[candidate.id]} />
                {/if}
                <button disabled={busy || !accounts[candidate.id]?.trim() || (isMailCandidate(candidate) && !sourceAccounts[candidate.id]?.trim())} onclick={() => review(candidate, "confirm")}>
                  <Icon name="check" size={14} /> Confirm
                </button>
                <button class="reject" disabled={busy} onclick={() => review(candidate, "reject")}>Reject</button>
              </div>
            {/if}
          {:else}
            <span class="state">{candidate.state}</span>
          {/if}
        </article>
      {/each}
    </div>
  {:else}
    <p class="empty">No candidates staged.</p>
  {/if}
</section>

<style>
  .review { margin-top: 2rem; border-top: 1px solid var(--border, #333); padding-top: 1.25rem; }
  .heading, .candidate-main, .decision { display: flex; align-items: center; gap: .75rem; }
  .heading { justify-content: space-between; flex-wrap: wrap; }
  h2 { margin: 0; font-size: 1rem; }
  p { margin: .25rem 0 0; color: var(--muted, #888); font-size: .8rem; }
  .file, button { display: inline-flex; align-items: center; gap: .35rem; border: 1px solid var(--border, #333); border-radius: 6px; padding: .4rem .65rem; font: inherit; font-size: .78rem; background: transparent; color: inherit; cursor: pointer; }
  .file input { display: none; }
  .mapping { display: grid; grid-template-columns: repeat(auto-fit, minmax(9rem, 1fr)); gap: .65rem; margin: 1rem 0; padding: .8rem; background: var(--card-bg, rgba(127,127,127,.05)); border-radius: 8px; }
  .mapping label { display: flex; flex-direction: column; gap: .2rem; color: var(--muted, #888); font-size: .68rem; }
  input, select { min-width: 0; border: 1px solid var(--border, #333); border-radius: 5px; padding: .35rem .45rem; font: inherit; font-size: .78rem; background: transparent; color: inherit; }
  .mapping .profile, .mapping .account { grid-column: span 2; }
  .mapping button { align-self: end; justify-content: center; }
  .preview { display: flex; align-items: center; flex-wrap: wrap; gap: .65rem; margin: -.4rem 0 1rem; padding: .65rem .8rem; border: 1px solid var(--border, #333); border-radius: 7px; font-size: var(--text-xs); }
  .preview button { margin-left: auto; }
  .candidate-list { display: grid; gap: .4rem; margin-top: 1rem; min-width: 0; }
  article { display: flex; align-items: center; justify-content: space-between; gap: 1rem; width: 100%; min-width: 0; box-sizing: border-box; padding: .65rem .75rem; border: 1px solid var(--border, #333); border-radius: 7px; }
  article.resolved { opacity: .55; }
  .candidate-main { min-width: 0; flex: 1 1 auto; overflow: hidden; }
  time, .state { color: var(--muted, #888); font-size: .72rem; }
  strong { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: .82rem; }
  .source-mail { color: var(--muted, #888); font-size: .7rem; white-space: nowrap; }
  .amount { margin-left: auto; font-variant-numeric: tabular-nums; font-size: .82rem; color: var(--primary); }
  .amount.outflow { color: inherit; }
  .decision { flex: 0 1 32rem; min-width: 0; flex-wrap: wrap; justify-content: flex-end; }
  .decision input { width: 12rem; }
  .transfer-match span, .ambiguous { color: var(--warning, #a76b2c); font-size: .7rem; }
  button.reject { border-color: transparent; color: var(--muted, #888); }
  button:disabled { opacity: .45; cursor: default; }
  .error { color: var(--danger, #b44); }
  .notice { color: var(--primary); }
  .empty { margin-top: 1rem; }
  @media (max-width: 760px) { article, .candidate-main { align-items: flex-start; flex-direction: column; } .amount { margin-left: 0; } .decision, .decision input { width: 100%; } }
</style>
