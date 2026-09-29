<script lang="ts">
  import { onMount } from 'svelte';
  import Overlay from './Overlay.svelte';
  import { decisionEngine } from './models/decision-engine.svelte';

  let open = $state(false);

  onMount(() => {
    void decisionEngine.refresh();
  });

  function openSheet(): void {
    open = true;
    void decisionEngine.refresh();
  }

  function closeSheet(): void {
    if (!decisionEngine.isPulling) {
      open = false;
    }
  }
</script>

<!-- Footer trigger pill -->
<button
  type="button"
  class="decision-pill"
  class:active={decisionEngine.isMechanism2Active}
  onclick={openSheet}
  title="Configure Sjel Decision Engine & System-1 Models"
>
  {#if decisionEngine.isMechanism2Active}
    <span class="dot active">●</span>
    <span>Decision: CLM 8B</span>
  {:else}
    <span class="dot">○</span>
    <span>Decision: Heuristic</span>
  {/if}
</button>

{#if open}
  <Overlay
    eyebrow="System 1 Local Intelligence"
    title="Decision Engine"
    onClose={closeSheet}
    busy={decisionEngine.isPulling}
    width="540px"
  >
    <div class="content">
      <!-- Node Hardware Card -->
      <section class="card hardware">
        <div class="card-header">
          <span class="label">THIS NODE</span>
          <span class="status-badge" class:online={decisionEngine.ollamaRunning}>
            {decisionEngine.ollamaRunning ? '● Ollama Connected' : '○ Ollama Offline'}
          </span>
        </div>
        <p class="node-title">{decisionEngine.hardware.chip}</p>
        <p class="node-sub">
          {decisionEngine.hardware.totalMemoryGb} GB Unified Memory · Loopback (127.0.0.1:11434)
        </p>
      </section>

      <!-- Active Mechanism Selector -->
      <section class="mechanisms">
        <p class="label">ACTIVE MECHANISM</p>

        <!-- Mechanism 1 -->
        <label
          class="mechanism-card"
          class:selected={decisionEngine.mechanism === 'mechanism1'}
        >
          <input
            type="radio"
            name="mechanism"
            value="mechanism1"
            checked={decisionEngine.mechanism === 'mechanism1'}
            onchange={() => decisionEngine.setMechanism('mechanism1')}
          />
          <div class="mechanism-body">
            <div class="mechanism-title-row">
              <span class="mechanism-name">Mechanism 1 · Built-in Heuristics & Cues</span>
              <span class="badge zero-ram">0 GB RAM</span>
            </div>
            <p class="mechanism-desc">
              Deterministic whole-word keyword routing and 5-factor trip plan scoring.
              Always available, instant, zero extra footprint.
            </p>
          </div>
        </label>

        <!-- Mechanism 2 -->
        <label
          class="mechanism-card"
          class:selected={decisionEngine.mechanism === 'mechanism2'}
        >
          <input
            type="radio"
            name="mechanism"
            value="mechanism2"
            checked={decisionEngine.mechanism === 'mechanism2'}
            onchange={() => decisionEngine.setMechanism('mechanism2')}
          />
          <div class="mechanism-body">
            <div class="mechanism-title-row">
              <span class="mechanism-name">Mechanism 2 · Contrastive Decision (CLM)</span>
              <span class="badge recommended">Recommended for this Mac</span>
            </div>
            <p class="mechanism-desc">
              Scores split-ticket travel options, itinerary trade-offs, and opportunity matches
              using a local Qwen3-8B ({decisionEngine.recommendedTag}, ~{decisionEngine.recommendedVramGb} GB).
            </p>
          </div>
        </label>
      </section>

      <!-- Model Installation & Readiness -->
      {#if !decisionEngine.hasDecisionModel}
        <section class="card install-box">
          <div class="install-info">
            <p class="install-title">Recommended Model Not Yet Installed</p>
            <p class="install-sub">
              Download <strong>{decisionEngine.recommendedTag}</strong> into your local Ollama instance.
              Uses ~{decisionEngine.recommendedVramGb} GB, leaving {Math.round(decisionEngine.hardware.totalMemoryGb - decisionEngine.recommendedVramGb)} GB headroom.
            </p>
          </div>

          {#if decisionEngine.isPulling}
            <div class="progress-container">
              <div class="progress-bar">
                <div class="progress-fill" style="width: {decisionEngine.pullProgress}%"></div>
              </div>
              <p class="progress-text">{decisionEngine.pullStatusText}</p>
            </div>
          {:else}
            <button
              type="button"
              class="primary-btn"
              disabled={!decisionEngine.ollamaRunning}
              onclick={() => decisionEngine.installRecommendedModel()}
            >
              {decisionEngine.ollamaRunning ? `Download & Enable ${decisionEngine.recommendedTag}` : 'Start Ollama to Download'}
            </button>
          {/if}

          {#if decisionEngine.error}
            <p class="error-text">{decisionEngine.error}</p>
          {/if}
        </section>
      {:else}
        <section class="card ready-box">
          <span class="ready-icon">✓</span>
          <div>
            <p class="ready-title">{decisionEngine.recommendedTag} is ready</p>
            <p class="ready-sub">
              {decisionEngine.isMechanism2Active
                ? 'Active on loopback. Travel candidate scoring is enhanced.'
                : 'Installed in Ollama. Switch to Mechanism 2 above to activate.'}
            </p>
          </div>
        </section>
      {/if}

      <p class="quiet-note">
        Simplicity at its core: if Ollama is quit or sleeping, Sjel quietly falls back
        to Mechanism 1 without disruption.
      </p>
    </div>
  </Overlay>
{/if}

<style>
  .decision-pill {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.2rem 0.6rem;
    border-radius: 999px;
    border: 1px solid var(--card-border, #e5e5ea);
    background: var(--surface, #ffffff);
    color: var(--text-primary, #1c1c1e);
    font-size: var(--text-xs);
    font-weight: 500;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .decision-pill:hover {
    border-color: var(--accent, #007aff);
  }

  .decision-pill.active {
    border-color: rgba(52, 199, 89, 0.5);
    background: rgba(52, 199, 89, 0.08);
  }

  .dot {
    font-size: var(--text-2xs);
    color: var(--text-secondary, #8e8e93);
  }

  .dot.active {
    color: #34c759;
  }

  .content {
    display: grid;
    gap: 1rem;
    padding-top: 0.5rem;
  }

  .label {
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.05em;
    color: var(--text-secondary, #8e8e93);
    text-transform: uppercase;
    margin: 0 0 0.4rem;
  }

  .card {
    border: 1px solid var(--card-border, #e5e5ea);
    border-radius: 10px;
    padding: 0.85rem 1rem;
    background: var(--surface-card, rgba(0, 0, 0, 0.02));
  }

  .hardware .card-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 0.2rem;
  }

  .status-badge {
    font-size: var(--text-2xs);
    color: var(--text-secondary, #8e8e93);
  }

  .status-badge.online {
    color: #34c759;
    font-weight: 500;
  }

  .node-title {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 600;
  }

  .node-sub {
    margin: 0.15rem 0 0;
    font-size: var(--text-sm);
    color: var(--text-secondary, #8e8e93);
  }

  .mechanisms {
    display: grid;
    gap: 0.5rem;
  }

  .mechanism-card {
    display: flex;
    align-items: flex-start;
    gap: 0.75rem;
    padding: 0.85rem;
    border: 1px solid var(--card-border, #e5e5ea);
    border-radius: 10px;
    cursor: pointer;
    transition: all 0.15s ease;
    background: var(--surface, #ffffff);
  }

  .mechanism-card:hover {
    border-color: var(--card-border-hover, #c7c7cc);
  }

  .mechanism-card.selected {
    border-color: var(--accent, #007aff);
    background: rgba(0, 122, 255, 0.03);
  }

  .mechanism-card input {
    margin-top: 0.2rem;
    cursor: pointer;
  }

  .mechanism-body {
    flex: 1;
    display: grid;
    gap: 0.2rem;
  }

  .mechanism-title-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
  }

  .mechanism-name {
    font-weight: 600;
    font-size: var(--text-base);
  }

  .badge {
    font-size: var(--text-2xs);
    padding: 0.15rem 0.45rem;
    border-radius: 4px;
    font-weight: 500;
  }

  .badge.zero-ram {
    background: rgba(142, 142, 147, 0.15);
    color: var(--text-secondary, #8e8e93);
  }

  .badge.recommended {
    background: rgba(0, 122, 255, 0.12);
    color: var(--accent, #007aff);
  }

  .mechanism-desc {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-secondary, #8e8e93);
    line-height: 1.35;
  }

  .install-box {
    display: grid;
    gap: 0.75rem;
    background: rgba(0, 122, 255, 0.04);
    border-color: rgba(0, 122, 255, 0.2);
  }

  .install-title {
    margin: 0;
    font-weight: 600;
    font-size: var(--text-base);
  }

  .install-sub {
    margin: 0.2rem 0 0;
    font-size: var(--text-xs);
    color: var(--text-secondary, #8e8e93);
    line-height: 1.3;
  }

  .primary-btn {
    width: 100%;
    padding: 0.55rem;
    border-radius: 8px;
    border: none;
    background: var(--accent, #007aff);
    color: #ffffff;
    font-weight: 500;
    font-size: var(--text-sm);
    cursor: pointer;
    transition: opacity 0.15s ease;
  }

  .primary-btn:hover:not(:disabled) {
    opacity: 0.9;
  }

  .primary-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .progress-container {
    display: grid;
    gap: 0.35rem;
  }

  .progress-bar {
    width: 100%;
    height: 8px;
    background: rgba(0, 0, 0, 0.08);
    border-radius: 4px;
    overflow: hidden;
  }

  .progress-fill {
    height: 100%;
    background: var(--accent, #007aff);
    transition: width 0.2s ease;
  }

  .progress-text {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-secondary, #8e8e93);
    font-family: var(--font-mono, monospace);
  }

  .ready-box {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    background: rgba(52, 199, 89, 0.06);
    border-color: rgba(52, 199, 89, 0.25);
  }

  .ready-icon {
    font-size: var(--text-lg);
    color: #34c759;
    font-weight: 700;
  }

  .ready-title {
    margin: 0;
    font-weight: 600;
    font-size: var(--text-sm);
  }

  .ready-sub {
    margin: 0.15rem 0 0;
    font-size: var(--text-xs);
    color: var(--text-secondary, #8e8e93);
  }

  .error-text {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--danger, #ff3b30);
  }

  .quiet-note {
    margin: 0.25rem 0 0;
    font-size: var(--text-xs);
    color: var(--text-secondary, #8e8e93);
    text-align: center;
    line-height: 1.4;
  }
</style>
