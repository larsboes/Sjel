<script lang="ts">
  import type { ParameterSimulatorSpec } from "./types";

  let { spec }: { spec: ParameterSimulatorSpec } = $props();

  // svelte-ignore state_referenced_locally
  let values = $state<Record<string, number | string>>(
    Object.fromEntries(spec.inputs.map((inp) => [inp.id, inp.default])),
  );

  // Safe formula evaluator without eval()
  function evaluateFormula(formula: string, env: Record<string, number | string>): number {
    try {
      // Tokenize formula into identifiers, numbers, and basic operators (+, -, *, /)
      // Example formula: "load * temp * 0.015 + 0.2"
      let expr = formula;
      for (const [k, v] of Object.entries(env)) {
        const numVal = typeof v === "number" ? v : Number(v) || 0;
        expr = expr.replace(new RegExp(`\\b${k}\\b`, "g"), String(numVal));
      }
      // Simple recursive or regex math evaluator for arithmetic (+, -, *, /)
      // Only allow digits, decimals, whitespace, and + - * / ( )
      if (!/^[0-9.\s+\-*/()]+$/.test(expr)) return 0;
      // Use Function constructor with empty scope as sandboxed math parser
      const calc = new Function(`return (${expr})`);
      const res = Number(calc());
      return isNaN(res) ? 0 : res;
    } catch {
      return 0;
    }
  }

  const computedOutputs = $derived.by(() => {
    return spec.outputs.map((out) => {
      const val = evaluateFormula(out.formula, values);
      const decimals = out.decimals ?? 2;
      let formatted = val.toFixed(decimals);
      if (out.format === "currency") formatted = `€${formatted}`;
      else if (out.format === "percent") formatted = `${formatted}%`;
      return {
        ...out,
        raw: val,
        formatted,
      };
    });
  });
</script>

<div class="genui-card simulator-container">
  <div class="sim-header">
    <div class="header-left">
      <span class="genui-badge">GenUI · Simulator</span>
      <h4 class="sim-title">{spec.title ?? "Parameter Simulator"}</h4>
    </div>
    {#if spec.description}
      <p class="sim-desc">{spec.description}</p>
    {/if}
  </div>

  <div class="sim-body">
    <!-- Inputs Panel -->
    <div class="inputs-panel">
      <span class="panel-heading">Controls & Parameters</span>
      {#each spec.inputs as inp (inp.id)}
        <div class="input-row">
          <div class="input-label-row">
            <span class="input-label">{inp.label}</span>
            <span class="input-current">{values[inp.id]}{inp.unit ? ` ${inp.unit}` : ""}</span>
          </div>

          {#if inp.type === "slider"}
            <input
              type="range"
              min={inp.min ?? 0}
              max={inp.max ?? 100}
              step={inp.step ?? 1}
              bind:value={values[inp.id]}
              class="sim-range"
              aria-label={inp.label}
            />
          {:else if inp.type === "segmented" && inp.options}
            <div class="segmented-control">
              {#each inp.options as opt (opt.value)}
                <button
                  type="button"
                  class="segment-btn"
                  class:active={values[inp.id] === opt.value}
                  onclick={() => (values[inp.id] = opt.value)}
                >
                  {opt.label}
                </button>
              {/each}
            </div>
          {/if}
        </div>
      {/each}
    </div>

    <!-- Live Outputs Panel -->
    <div class="outputs-panel">
      <span class="panel-heading">Live Computed Telemetry</span>
      <div class="outputs-grid">
        {#each computedOutputs as out (out.id)}
          <div class="output-card">
            <span class="out-label">{out.label}</span>
            <div class="out-val-row">
              <span class="out-val">{out.formatted}</span>
              {#if out.unit && out.format !== "currency" && out.format !== "percent"}
                <span class="out-unit">{out.unit}</span>
              {/if}
            </div>
          </div>
        {/each}
      </div>
    </div>
  </div>
</div>

<style>
  .genui-card {
    background: var(--surface, #ffffff);
    border: 1px solid var(--rule, #e2e8f0);
    border-radius: var(--radius-sm, 6px);
    padding: 0.9rem;
    margin: 1rem 0;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.04);
  }

  .sim-header {
    margin-bottom: 0.8rem;
    padding-bottom: 0.4rem;
    border-bottom: 1px solid var(--rule, #e2e8f0);
  }

  .header-left {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .genui-badge {
    text-transform: uppercase;
    font-size: var(--text-2xs);
    font-weight: 700;
    background: var(--primary-soft, #eff6ff);
    color: var(--primary, #2563eb);
    padding: 0.1rem 0.35rem;
    border-radius: 3px;
    letter-spacing: 0.04em;
  }

  .sim-title {
    margin: 0;
    font-size: var(--text-sm, 0.9rem);
    font-weight: 600;
    color: var(--text-primary, #0f172a);
  }

  .sim-desc {
    margin: 0.3rem 0 0;
    font-size: var(--text-xs, 0.75rem);
    color: var(--text-secondary, #64748b);
  }

  .sim-body {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(14rem, 1fr));
    gap: 1rem;
  }

  .panel-heading {
    display: block;
    font-size: var(--text-xs);
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--text-secondary, #64748b);
    margin-bottom: 0.6rem;
  }

  .inputs-panel {
    display: flex;
    flex-direction: column;
    gap: 0.7rem;
    padding-right: 0.5rem;
  }

  .input-row {
    background: var(--surface, #f8fafc);
    border: 1px solid var(--rule, #e2e8f0);
    border-radius: var(--radius-sm, 4px);
    padding: 0.5rem 0.65rem;
  }

  .input-label-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: var(--text-xs);
    margin-bottom: 0.3rem;
  }

  .input-label {
    font-weight: 500;
    color: var(--text-primary, #0f172a);
  }

  .input-current {
    font-weight: 700;
    color: var(--primary, #2563eb);
  }

  .sim-range {
    width: 100%;
    accent-color: var(--primary, #2563eb);
    cursor: pointer;
  }

  .segmented-control {
    display: flex;
    gap: 0.2rem;
    background: var(--surface, #ffffff);
    padding: 0.15rem;
    border: 1px solid var(--rule, #cbd5e1);
    border-radius: 4px;
  }

  .segment-btn {
    flex: 1;
    background: transparent;
    border: none;
    font-size: var(--text-2xs);
    padding: 0.2rem 0.4rem;
    border-radius: 3px;
    color: var(--text-secondary, #64748b);
    cursor: pointer;
    transition: color var(--motion-fast) ease, background var(--motion-fast) ease;
  }

  .segment-btn:hover {
    color: var(--text-primary, #0f172a);
  }

  .segment-btn.active {
    background: var(--primary, #2563eb);
    color: #ffffff;
    font-weight: 600;
  }

  .outputs-panel {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
  }

  .outputs-grid {
    display: grid;
    gap: 0.5rem;
  }

  .output-card {
    background: var(--surface, #f8fafc);
    border: 1px solid var(--rule, #e2e8f0);
    border-left: 3px solid var(--primary, #2563eb);
    border-radius: var(--radius-sm, 4px);
    padding: 0.55rem 0.75rem;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
  }

  .out-label {
    font-size: var(--text-2xs);
    color: var(--text-secondary, #64748b);
    font-weight: 500;
  }

  .out-val-row {
    display: flex;
    align-items: baseline;
    gap: 0.3rem;
    margin-top: 0.1rem;
  }

  .out-val {
    font-size: var(--text-md);
    font-weight: 700;
    color: var(--text-primary, #0f172a);
  }

  .out-unit {
    font-size: var(--text-xs);
    color: var(--text-secondary, #64748b);
  }
</style>
