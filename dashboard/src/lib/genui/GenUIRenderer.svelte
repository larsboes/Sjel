<script lang="ts">
  import type { GenUISpec } from "./types";
  import MetricGrid from "./MetricGrid.svelte";
  import DecisionMatrix from "./DecisionMatrix.svelte";
  import ParameterSimulator from "./ParameterSimulator.svelte";
  import StatusTimeline from "./StatusTimeline.svelte";

  let { spec }: { spec: GenUISpec } = $props();

  let showRaw = $state(false);
</script>

<div class="genui-wrapper">
  {#if spec.type === "metric-grid"}
    <MetricGrid {spec} />
  {:else if spec.type === "decision-matrix"}
    <DecisionMatrix {spec} />
  {:else if spec.type === "simulator"}
    <ParameterSimulator {spec} />
  {:else if spec.type === "status-timeline"}
    <StatusTimeline {spec} />
  {:else if spec.type === "raw"}
    <div class="genui-error-box">
      <span class="error-title">Invalid GenUI Specification</span>
      {#if spec.error}
        <p class="error-msg">{spec.error}</p>
      {/if}
      <pre><code>{spec.raw}</code></pre>
    </div>
  {:else}
    <div class="genui-error-box">
      <span class="error-title">Unsupported GenUI Widget</span>
      <pre><code>{JSON.stringify(spec, null, 2)}</code></pre>
    </div>
  {/if}

  {#if spec.type !== "raw"}
    <div class="spec-toggle-row">
      <button
        type="button"
        class="spec-toggle-btn"
        onclick={() => (showRaw = !showRaw)}
      >
        {showRaw ? "Hide Spec ▴" : "View Declarative Spec ▾"}
      </button>
      {#if showRaw}
        <pre class="spec-raw-display"><code>{JSON.stringify(spec, null, 2)}</code></pre>
      {/if}
    </div>
  {/if}
</div>

<style>
  .genui-wrapper {
    margin: 1.2rem 0;
  }

  .genui-error-box {
    background: #fef2f2;
    border: 1px solid #fecaca;
    border-radius: var(--radius-sm, 6px);
    padding: 0.8rem;
    font-size: var(--text-xs, 0.75rem);
    color: #991b1b;
  }

  .error-title {
    font-weight: 700;
  }

  .error-msg {
    margin: 0.3rem 0;
  }

  .genui-error-box pre {
    background: #ffffff;
    border: 1px solid #fecaca;
    padding: 0.5rem;
    border-radius: 4px;
    overflow-x: auto;
  }

  .spec-toggle-row {
    margin-top: -0.6rem;
    margin-bottom: 0.8rem;
    text-align: right;
  }

  .spec-toggle-btn {
    background: transparent;
    border: none;
    font-size: var(--text-2xs);
    color: var(--text-secondary, #64748b);
    cursor: pointer;
    text-decoration: underline;
    text-decoration-style: dotted;
  }

  .spec-toggle-btn:hover {
    color: var(--primary, #2563eb);
  }

  .spec-raw-display {
    text-align: left;
    margin-top: 0.4rem;
    background: var(--surface, #f8fafc);
    border: 1px solid var(--rule, #e2e8f0);
    padding: 0.5rem 0.7rem;
    border-radius: 4px;
    font-size: var(--text-2xs);
    overflow-x: auto;
  }
</style>
