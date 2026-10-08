<script lang="ts">
  import PageHeader from "$lib/PageHeader.svelte";
  import ResearchTabs from "$lib/research/ResearchTabs.svelte";
  import { percent, suites, verdict, type BenchmarkRun } from "$lib/research/content";
  import { benchmarks } from "virtual:sjel-research";
  import Collection from "$lib/ui/Collection.svelte";
  import DataTable, { type Column } from "$lib/ui/DataTable.svelte";
  import type { Field } from "$lib/ui/collection";

  const REPO = "https://github.com/larsboes/Sjel/tree/main/research/benchmarks";
  const all = suites(benchmarks);

  // A suite name becomes a URL prefix and a CSS anchor name, so it keeps only safe characters.
  const prefix = (suite: string) => `b-${suite.toLowerCase().replace(/[^a-z0-9-]/g, "-")}`;

  // Suites arrive best accuracy first, so the default view keeps that order.
  const fields: Field<BenchmarkRun>[] = [
    { id: "model", label: "Model", value: (r) => r.result.model, cell: modelCell },
    { id: "backend", label: "Backend", kind: "select", width: "7rem", value: (r) => r.result.backend },
    { id: "correct", label: "Correct", kind: "number", width: "5.5rem", value: (r) => r.result.correct, cell: correctCell },
    { id: "accuracy", label: "Accuracy", kind: "number", width: "6rem", value: (r) => r.result.accuracy, cell: accuracyCell },
    { id: "p50", label: "p50", kind: "number", width: "5.5rem", value: (r) => r.result.latency_ms.p50, cell: p50Cell },
    { id: "p95", label: "p95", kind: "number", width: "5.5rem", value: (r) => r.result.latency_ms.p95, cell: p95Cell },
    { id: "date", label: "Date", kind: "date", width: "6.5rem", value: (r) => r.result.date },
    { id: "host", label: "Host", kind: "select", width: "8rem", value: (r) => r.result.host },
  ];

  // The per-case matrix: one row per case, one column per run.
  const caseColumns = (runs: BenchmarkRun[]): Column<string>[] => [
    { id: "case", label: "Case", width: "12rem", cell: caseCell },
    ...runs.map((run) => ({ id: run.file, label: run.result.model, width: "7rem", align: "end" as const, cell: verdictCell })),
  ];
  const runOf = (file: string) => benchmarks.find((r) => r.file === file);
</script>

{#snippet modelCell(run: BenchmarkRun)}<span class="model">{run.result.model}</span>{/snippet}
{#snippet correctCell(run: BenchmarkRun)}{run.result.correct}/{run.result.n}{/snippet}
{#snippet accuracyCell(run: BenchmarkRun)}{percent(run.result.accuracy)}{/snippet}
{#snippet p50Cell(run: BenchmarkRun)}{Math.round(run.result.latency_ms.p50)} ms{/snippet}
{#snippet p95Cell(run: BenchmarkRun)}{Math.round(run.result.latency_ms.p95)} ms{/snippet}
{#snippet caseCell(id: string)}<span class="model">{id}</span>{/snippet}
{#snippet verdictCell(id: string, column: Column<string>)}
  {@const run = runOf(column.id)}
  {@const ok = run ? verdict(run, id) : null}
  <span class:miss={ok === false}>{ok === null ? "" : ok ? "✓" : "✗"}</span>
{/snippet}

<svelte:head><title>Benchmarks · Sjel research</title></svelte:head>

<PageHeader
  badge="Research"
  title="Benchmarks"
  desc="What Sjel measured on its own hardware. Each run is a result file in research/benchmarks, kept as the run wrote it."
/>
<ResearchTabs current="benchmarks" />

{#if all.length === 0}
  <p class="empty">No benchmark has run yet.</p>
{/if}

{#each all as suite (suite.name)}
  <section>
    <h2>
      {suite.name}
      <a class="method" href={`${REPO}/${suite.name}`} target="_blank" rel="noreferrer">Method and cases</a>
    </h2>

    <div class="card table-wrap">
      <Collection id={prefix(suite.name)} rows={suite.runs} {fields} key={(r) => r.file} title={(r) => r.result.model} />
    </div>

    <details>
      <summary>Per case ({suite.caseIds.length})</summary>
      <div class="card table-wrap">
        <DataTable rows={suite.caseIds} columns={caseColumns(suite.runs)} key={(id) => id} />
      </div>
    </details>
  </section>
{/each}

<style>
  section {
    margin-bottom: 2rem;
  }
  h2 {
    display: flex;
    align-items: baseline;
    gap: 0.75rem;
    margin: 1rem 0 0.75rem;
    color: var(--text-primary);
    font-size: var(--text-md);
  }
  .method,
  .empty {
    color: var(--text-secondary);
    font-size: var(--text-sm);
    font-weight: 400;
  }
  .method {
    color: var(--primary);
  }
  .table-wrap {
    overflow-x: auto;
    padding: var(--space-2);
  }
  .model {
    color: var(--text-primary);
    font-family: var(--font-mono);
  }
  .miss {
    color: var(--danger);
  }
  details {
    margin-top: 0.75rem;
  }
  summary {
    margin-bottom: 0.5rem;
    color: var(--text-secondary);
    font-size: var(--text-sm);
    cursor: pointer;
  }
</style>
