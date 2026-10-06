<script lang="ts">
  import type { FinanceTransaction } from "$lib/api";
  import { inspectorStore } from "$lib/inspector/inspector.svelte";
  import { transactionItem } from "$lib/inspector/connections";

  let { rows }: { rows: FinanceTransaction[] } = $props();

  const money = (cents: number, currency: string) =>
    new Intl.NumberFormat("de-DE", { style: "currency", currency }).format(cents / 100);

  const short = (account: string) => account.split(":").slice(1).join(" · ") || account;

  // Through `transactionItem`, so the inspector gets the linkable `fin:tx:` id and every
  // capability that references this transaction can answer for it (libs/links/ISA.md D3).
  function inspect(row: FinanceTransaction) {
    inspectorStore.open({
      ...transactionItem(row),
      notes: row.purpose ? `Purpose: ${row.purpose.replaceAll("_", " ")}` : undefined,
    });
  }
</script>

<div class="table-wrap">
  <table>
    <thead>
      <tr>
        <th scope="col">Date</th>
        <th scope="col">Description</th>
        <th scope="col">Account</th>
        <th scope="col">Category</th>
        <th scope="col">Purpose</th>
        <th class="num" scope="col">Your amount</th>
      </tr>
    </thead>
    <tbody>
      {#each rows as row (row.id)}
        <tr
          class="interactive-row"
          tabindex="0"
          role="button"
          aria-label={`Transaction: ${row.description}, ${money(row.amount_cents, row.currency)}`}
          onclick={() => {
            inspect(row);
          }}
          onkeydown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              inspect(row);
            }
          }}
        >
          <td class="mono">{row.date}</td>
          <td class="desc-cell">
            <span class="desc-text">{row.description}</span>
          </td>
          <td><span class="account-tag">{short(row.account)}</span></td>
          <td><span class="category-pill">{short(row.category)}</span></td>
          <td class="context">
            {row.purpose?.replaceAll("_", " ") ?? "—"}
            {#if row.shared_cents > 0}
              <span class="shared-sub">· {money(row.cash_amount_cents, row.currency)} paid</span>
            {/if}
          </td>
          <td class="num {row.kind}">
            {row.kind === "expense" ? "−" : row.kind === "income" ? "+" : ""}{money(row.amount_cents, row.currency)}
          </td>
        </tr>
      {/each}
      {#if rows.length === 0}
        <tr><td colspan="6" class="muted">No matching transactions.</td></tr>
      {/if}
    </tbody>
  </table>
</div>

<style>
  .table-wrap {
    overflow-x: auto;
    margin-top: 0.65rem;
    border-radius: var(--radius-lg);
    background: var(--card-bg);
    border: 1px solid var(--card-border);
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--text-xs);
  }

  th,
  td {
    text-align: left;
    padding: 0.55rem 0.65rem;
    border-bottom: 1px solid var(--card-border);
    white-space: nowrap;
  }

  th {
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    background: var(--surface);
  }

  .interactive-row {
    cursor: pointer;
    transition: background-color var(--motion-fast) var(--ease-out);
  }

  .interactive-row:hover {
    background: var(--surface);
  }

  .interactive-row:focus-visible {
    outline: 2px solid var(--primary);
    outline-offset: -2px;
  }

  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-2xs);
    color: var(--text-secondary);
  }

  .desc-cell {
    white-space: normal;
    min-width: 11rem;
  }

  .desc-text {
    font-weight: 500;
    color: var(--text-primary);
  }

  .account-tag {
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  .category-pill {
    display: inline-flex;
    align-items: center;
    padding: 0.1rem 0.45rem;
    border-radius: var(--radius-full);
    background: var(--surface);
    color: var(--text-secondary);
    font-size: var(--text-2xs);
    font-weight: 500;
    border: 1px solid var(--card-border);
  }

  td.context {
    color: var(--text-tertiary);
    text-transform: capitalize;
  }

  .shared-sub {
    font-size: var(--text-2xs);
    color: var(--text-tertiary);
  }

  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
    font-weight: 600;
  }

  td.income {
    color: var(--primary);
  }

  td.transfer,
  .muted {
    color: var(--text-tertiary);
  }
</style>
