<script lang="ts">
  import type { FinanceTransaction } from "$lib/api";
  import { inspectorStore } from "$lib/inspector/inspector.svelte";
  import { transactionItem } from "$lib/inspector/connections";
  import Collection from "$lib/ui/Collection.svelte";
  import type { Field } from "$lib/ui/collection";
  import { tip } from "$lib/tip";

  /** `id` turns on search, filter, sort, group and the board, kept in the URL under it. */
  let { rows, id }: { rows: FinanceTransaction[]; id?: string } = $props();

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

  const purpose = (p: string) => p.replaceAll("_", " ");
  const KIND = { expense: "Expense", income: "Income", transfer: "Transfer" } as Record<string, string>;

  const fields: Field<FinanceTransaction>[] = [
    { id: "date", label: "Date", kind: "date", width: "6rem", value: (r) => r.date, cell: dateCell },
    { id: "description", label: "Description", value: (r) => r.description, cell: descriptionCell },
    { id: "account", label: "Account", kind: "select", width: "9rem", value: (r) => r.account, display: short, cell: accountCell },
    { id: "category", label: "Category", kind: "select", width: "9rem", value: (r) => r.category, display: short },
    { id: "purpose", label: "Purpose", kind: "select", width: "8rem", value: (r) => r.purpose, display: purpose, cell: purposeCell },
    // Not a column: the amount's sign already says it. Here so a view can filter or split by it.
    {
      id: "kind", label: "Kind", kind: "select", column: false, value: (r) => r.kind, display: (k) => KIND[k] ?? k,
      tone: (k) => (k === "income" ? "success" : k === "transfer" ? "muted" : "neutral"),
    },
    {
      id: "amount", label: "Your amount", kind: "number", width: "7.5rem",
      value: (r) => (r.kind === "expense" ? -r.amount_cents : r.amount_cents), cell: amountCell,
    },
  ];
</script>

{#snippet dateCell(row: FinanceTransaction)}<span class="mono">{row.date}</span>{/snippet}
{#snippet descriptionCell(row: FinanceTransaction)}
  <span class="desc" use:tip={row.description}>{row.description}</span>
{/snippet}
{#snippet accountCell(row: FinanceTransaction)}<span class="soft">{short(row.account)}</span>{/snippet}
{#snippet purposeCell(row: FinanceTransaction)}
  {#if row.shared_cents > 0}
    <span class="soft purpose" use:tip={`Shared. You paid ${money(row.cash_amount_cents, row.currency)}.`}>
      {row.purpose ? `${purpose(row.purpose)} · shared` : "Shared"}
    </span>
  {:else}
    <span class="soft purpose">{row.purpose ? purpose(row.purpose) : ""}</span>
  {/if}
{/snippet}
{#snippet amountCell(row: FinanceTransaction)}<span class={row.kind}>{signed(row)}</span>{/snippet}

<div class="card">
  <!-- A transfer moves money between your own accounts, so it is dimmed, not hidden. -->
  <Collection
    {id}
    {rows}
    {fields}
    key={(r) => r.id}
    title={(r) => `${r.description} · ${signed(r)}`}
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
