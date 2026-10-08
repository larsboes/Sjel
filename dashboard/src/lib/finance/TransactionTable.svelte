<script lang="ts">
  import type { FinanceTransaction } from "$lib/api";
  import { inspectorStore } from "$lib/inspector/inspector.svelte";
  import { transactionItem } from "$lib/inspector/connections";
  import DataTable, { type Column } from "$lib/ui/DataTable.svelte";
  import Chip from "$lib/ui/Chip.svelte";
  import { tip } from "$lib/tip";

  let { rows }: { rows: FinanceTransaction[] } = $props();

  const money = (cents: number, currency: string) =>
    new Intl.NumberFormat("de-DE", { style: "currency", currency }).format(cents / 100);

  const short = (account: string) => account.split(":").slice(1).join(" · ") || account;

  const signed = (row: FinanceTransaction) =>
    `${row.kind === "expense" ? "−" : row.kind === "income" ? "+" : ""}${money(row.amount_cents, row.currency)}`;

  // Through `transactionItem`, so the inspector gets the linkable `fin:tx:` id and every
  // capability that references this transaction can answer for it (libs/links/ISA.md D3).
  function inspect(row: FinanceTransaction) {
    inspectorStore.open({
      ...transactionItem(row),
      notes: row.purpose ? `Purpose: ${row.purpose.replaceAll("_", " ")}` : undefined,
    });
  }

  const columns: Column<FinanceTransaction>[] = [
    { id: "date", label: "Date", width: "6rem", cell: dateCell },
    { id: "description", label: "Description", cell: descriptionCell },
    { id: "account", label: "Account", width: "9rem", cell: accountCell },
    { id: "category", label: "Category", width: "9rem", cell: categoryCell },
    { id: "purpose", label: "Purpose", width: "8rem", cell: purposeCell },
    { id: "amount", label: "Your amount", width: "7.5rem", align: "end", cell: amountCell },
  ];
</script>

{#snippet dateCell(row: FinanceTransaction)}<span class="mono">{row.date}</span>{/snippet}
{#snippet descriptionCell(row: FinanceTransaction)}
  <span class="desc" use:tip={row.description}>{row.description}</span>
{/snippet}
{#snippet accountCell(row: FinanceTransaction)}<span class="soft">{short(row.account)}</span>{/snippet}
{#snippet categoryCell(row: FinanceTransaction)}<Chip label={short(row.category)} />{/snippet}
{#snippet purposeCell(row: FinanceTransaction)}
  {#if row.shared_cents > 0}
    <span class="soft purpose" use:tip={`Shared. You paid ${money(row.cash_amount_cents, row.currency)}.`}>
      {row.purpose ? `${row.purpose.replaceAll("_", " ")} · shared` : "Shared"}
    </span>
  {:else}
    <span class="soft purpose">{row.purpose?.replaceAll("_", " ") ?? ""}</span>
  {/if}
{/snippet}
{#snippet amountCell(row: FinanceTransaction)}<span class={row.kind}>{signed(row)}</span>{/snippet}

<div class="card">
  <!-- A transfer moves money between your own accounts, so it is dimmed, not hidden. -->
  <DataTable
    {rows}
    {columns}
    key={(r) => r.id}
    onOpen={inspect}
    inactive={(r) => r.kind === "transfer"}
    empty="No matching transactions."
  />
</div>

<style>
  .card {
    margin-top: var(--space-2);
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-lg);
    background: var(--card-bg);
    border: 1px solid var(--card-border);
  }

  .mono {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--text-secondary);
  }

  .desc {
    font-weight: 500;
  }

  .soft {
    color: var(--text-tertiary);
  }

  .purpose {
    text-transform: capitalize;
  }

  .expense {
    font-weight: 600;
  }

  .income {
    font-weight: 600;
    color: var(--primary);
  }
</style>
