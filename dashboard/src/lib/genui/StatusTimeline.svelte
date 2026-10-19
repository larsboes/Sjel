<script lang="ts">
  import type { StatusTimelineSpec } from "./types";

  let { spec }: { spec: StatusTimelineSpec } = $props();
</script>

<div class="genui-card timeline-container">
  <div class="timeline-header">
    <div class="header-left">
      <span class="genui-badge">GenUI · Timeline</span>
      <h4 class="timeline-title">{spec.title ?? "Project Milestones"}</h4>
    </div>
  </div>

  <div class="timeline-steps">
    {#each spec.steps as step, idx (idx)}
      <div class="step-item step-{step.status}">
        <div class="step-marker-col">
          <div class="step-dot">
            {#if step.status === "done"}✓{:else if step.status === "current"}●{:else if step.status === "blocked"}✕{:else}○{/if}
          </div>
          {#if idx < spec.steps.length - 1}
            <div class="step-line"></div>
          {/if}
        </div>
        <div class="step-content">
          <div class="step-label-row">
            <span class="step-label">{step.label}</span>
            {#if step.date}
              <span class="step-date">{step.date}</span>
            {/if}
          </div>
          {#if step.detail}
            <p class="step-detail">{step.detail}</p>
          {/if}
        </div>
      </div>
    {/each}
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

  .timeline-header {
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

  .timeline-title {
    margin: 0;
    font-size: var(--text-sm, 0.9rem);
    font-weight: 600;
    color: var(--text-primary, #0f172a);
  }

  .timeline-steps {
    display: flex;
    flex-direction: column;
    gap: 0;
  }

  .step-item {
    display: flex;
    gap: 0.75rem;
  }

  .step-marker-col {
    display: flex;
    flex-direction: column;
    align-items: center;
    width: 1.5rem;
  }

  .step-dot {
    width: 1.3rem;
    height: 1.3rem;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: var(--text-2xs);
    font-weight: 700;
    z-index: 1;
    background: var(--surface, #f1f5f9);
    border: 2px solid var(--rule, #cbd5e1);
    color: var(--text-secondary, #64748b);
  }

  .step-done .step-dot {
    background: #ecfdf5;
    border-color: #059669;
    color: #059669;
  }

  .step-current .step-dot {
    background: #eff6ff;
    border-color: #2563eb;
    color: #2563eb;
    box-shadow: 0 0 0 3px rgba(37, 99, 235, 0.2);
  }

  .step-blocked .step-dot {
    background: #fef2f2;
    border-color: #dc2626;
    color: #dc2626;
  }

  .step-line {
    flex: 1;
    width: 2px;
    background: var(--rule, #e2e8f0);
    min-height: 1.2rem;
    margin: 0.2rem 0;
  }

  .step-done .step-line {
    background: #a7f3d0;
  }

  .step-content {
    flex: 1;
    padding-bottom: 0.9rem;
  }

  .step-label-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 0.5rem;
  }

  .step-label {
    font-size: var(--text-xs, 0.8rem);
    font-weight: 600;
    color: var(--text-primary, #0f172a);
  }

  .step-date {
    font-size: var(--text-2xs);
    color: var(--text-secondary, #64748b);
  }

  .step-detail {
    margin: 0.2rem 0 0;
    font-size: var(--text-xs);
    color: var(--text-secondary, #475569);
    line-height: 1.35;
  }
</style>
