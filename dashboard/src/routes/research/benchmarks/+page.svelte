<script lang="ts">
  import PageHeader from "$lib/PageHeader.svelte";
  import ResearchTabs from "$lib/research/ResearchTabs.svelte";
  import { percent, suites, verdict } from "$lib/research/content";
  import { benchmarks } from "virtual:sjel-research";

  const REPO = "https://github.com/larsboes/Sjel/tree/main/research/benchmarks";
  const all = suites(benchmarks);
</script>

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
      <table>
        <thead>
          <tr>
            <th scope="col">Model</th>
            <th scope="col">Backend</th>
            <th scope="col" class="num">Correct</th>
            <th scope="col" class="num">Accuracy</th>
            <th scope="col" class="num">p50</th>
            <th scope="col" class="num">p95</th>
            <th scope="col">Date</th>
            <th scope="col">Host</th>
          </tr>
        </thead>
        <tbody>
          {#each suite.runs as run (run.file)}
            <tr>
              <td class="model">{run.result.model}</td>
              <td>{run.result.backend}</td>
              <td class="num">{run.result.correct}/{run.result.n}</td>
              <td class="num">{percent(run.result.accuracy)}</td>
              <td class="num">{Math.round(run.result.latency_ms.p50)} ms</td>
              <td class="num">{Math.round(run.result.latency_ms.p95)} ms</td>
              <td>{run.result.date}</td>
              <td class="dim">{run.result.host}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>

    <details>
      <summary>Per case ({suite.caseIds.length})</summary>
      <div class="card table-wrap">
        <table class="cases">
          <thead>
            <tr>
              <th scope="col">Case</th>
              {#each suite.runs as run (run.file)}<th scope="col" class="num">{run.result.model}</th>{/each}
            </tr>
          </thead>
          <tbody>
            {#each suite.caseIds as id (id)}
              <tr>
                <td class="model">{id}</td>
                {#each suite.runs as run (run.file)}
                  {@const ok = verdict(run, id)}
                  <td class="num" class:miss={ok === false}>{ok === null ? "" : ok ? "✓" : "✗"}</td>
                {/each}
              </tr>
            {/each}
          </tbody>
        </table>
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
    font-size: 1.05rem;
  }
  .method,
  .empty {
    color: var(--text-secondary);
    font-size: 0.85rem;
    font-weight: 400;
  }
  .method {
    color: var(--primary);
  }
  .table-wrap {
    overflow-x: auto;
    padding: 0;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.86rem;
  }
  th,
  td {
    padding: 0.5rem 0.75rem;
    border-bottom: 1px solid var(--card-border);
    text-align: left;
    white-space: nowrap;
  }
  th {
    color: var(--text-primary);
    font-weight: 600;
  }
  td {
    color: var(--text-secondary);
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .model {
    color: var(--text-primary);
    font-family: var(--font-mono, monospace);
  }
  .dim {
    white-space: normal;
  }
  .miss {
    color: var(--danger, #c0392b);
  }
  details {
    margin-top: 0.75rem;
  }
  summary {
    margin-bottom: 0.5rem;
    color: var(--text-secondary);
    font-size: 0.86rem;
    cursor: pointer;
  }
</style>
