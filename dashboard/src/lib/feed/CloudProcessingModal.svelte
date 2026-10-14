<script lang="ts">
  import { modal } from "$lib/modal";
  import type { CloudDerivativePreview, DataClass } from "$lib/api";

  interface Props {
    cloudPreview: CloudDerivativePreview;
    approvingCloudPreview: boolean;
    cloudPreviewError: string | null;
    dataClassLabel: (dataClass: DataClass) => string;
    onClose: () => void;
    onApprove: () => void;
  }

  let {
    cloudPreview,
    approvingCloudPreview,
    cloudPreviewError,
    dataClassLabel,
    onClose,
    onApprove,
  }: Props = $props();
</script>

<div class="modal-backdrop">
  <div
    class="preview-modal"
    use:modal={{
      onClose,
      canClose: () => !approvingCloudPreview,
    }}
    role="dialog"
    aria-modal="true"
    aria-labelledby="cloud-preview-title"
    tabindex="-1"
  >
    <header class="preview-head">
      <div>
        <p class="section-label">Local preview</p>
        <h2 id="cloud-preview-title">Review the cloud-ready copy</h2>
      </div>
      <button
        class="icon-button"
        type="button"
        aria-label="Close preview"
        disabled={approvingCloudPreview}
        onclick={onClose}
      >×</button>
    </header>

    <div class="preview-facts">
      <span>{dataClassLabel(cloudPreview.original_data_class)} source</span>
      <span>→</span>
      <span>{dataClassLabel(cloudPreview.derivative_data_class)} derivative</span>
      <span>{cloudPreview.redaction_count} redactions</span>
      <span>{cloudPreview.truncated ? "Bounded at 16,000 characters" : "Complete bounded document"}</span>
    </div>

    {#if cloudPreview.redaction_receipt}
      <p class="redaction-receipt">{cloudPreview.redaction_receipt}</p>
    {/if}

    {#if cloudPreview.redactions.length > 0}
      <div class="redaction-ledger" aria-label="Local entity redactions">
        <span class="section-label">Detected locally</span>
        {#each cloudPreview.redactions as finding (finding.entity_type)}
          <span><code>{finding.marker}</code> {finding.entity_type.replaceAll("_", " ")} × {finding.count}</span>
        {/each}
      </div>
    {/if}

    <div class="preview-document" aria-label="Document proposed for cloud processing">
      <pre>{cloudPreview.document}</pre>
    </div>

    <div class="preview-limitations">
      <p class="section-label">Review limits</p>
      <ul>
        {#each cloudPreview.limitations as limitation}
          <li>{limitation}</li>
        {/each}
      </ul>
    </div>

    {#if cloudPreviewError}<p class="inline-error" aria-live="polite">{cloudPreviewError}</p>{/if}
    <footer class="preview-actions">
      <p>This approval stores the exact reviewed copy locally. Provider calls: 0.</p>
      <button class="btn" disabled={approvingCloudPreview} onclick={onClose}>Cancel</button>
      <button class="btn btn-primary" disabled={approvingCloudPreview} onclick={onApprove}>
        {approvingCloudPreview ? "Staging…" : "Approve cloud-ready copy"}
      </button>
    </footer>
  </div>
</div>

<style>
  .section-label {
    margin-bottom: 0.35rem;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .inline-error {
    margin: 0.5rem 1.5rem 0;
    color: var(--danger);
    font-size: var(--text-xs);
  }

  .modal-backdrop {
    position: fixed;
    z-index: 100;
    inset: 0;
    display: grid;
    place-items: center;
    padding: 1.5rem;
    background: color-mix(in srgb, var(--card-bg) 72%, transparent);
    backdrop-filter: blur(8px);
  }

  .preview-modal {
    display: flex;
    width: min(62rem, 100%);
    max-height: min(52rem, calc(100vh - 3rem));
    flex-direction: column;
    overflow: hidden;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-lg);
    background: var(--card-bg);
    box-shadow: 0 1.5rem 5rem rgb(0 0 0 / 0.18);
  }

  .preview-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 1rem;
    padding: 1.4rem 1.5rem 1rem;
  }

  .preview-head .section-label {
    margin-bottom: 0.35rem;
  }

  .preview-head h2 {
    margin: 0;
    font-size: 1.35rem;
    font-weight: 580;
    letter-spacing: -0.02em;
  }

  .icon-button {
    width: 2rem;
    height: 2rem;
    border: 0;
    background: transparent;
    color: var(--text-tertiary);
    font: inherit;
    font-size: 1.4rem;
    cursor: pointer;
  }

  .preview-facts {
    display: flex;
    flex-wrap: wrap;
    gap: 0.45rem;
    padding: 0 1.5rem 1rem;
  }

  .preview-facts span {
    padding: 0.3rem 0.5rem;
    border: 1px solid var(--card-border);
    border-radius: 999px;
    color: var(--text-secondary);
    font-size: var(--text-2xs);
  }

  .redaction-receipt {
    margin: 0.75rem 1.5rem 0;
    padding: 0.5rem 0.65rem;
    border-left: 3px solid var(--warning);
    background: var(--warning-soft);
    color: var(--text-primary);
    font-size: 0.78rem;
  }

  .redaction-ledger {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.4rem 0.75rem;
    padding: 0.7rem 1.5rem;
    border-bottom: 1px solid var(--card-border);
    color: var(--text-secondary);
    font-size: var(--text-2xs);
  }

  .redaction-ledger .section-label {
    margin: 0;
  }

  .redaction-ledger code {
    color: var(--primary);
    font-family: var(--font-mono);
  }

  .preview-document {
    min-height: 12rem;
    flex: 1;
    overflow: auto;
    margin: 0 1.5rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    background: var(--surface);
  }

  .preview-document pre {
    margin: 0;
    padding: 1.25rem;
    color: var(--text-primary);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    line-height: 1.6;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .preview-limitations {
    padding: 1rem 1.5rem 0;
  }

  .preview-limitations .section-label {
    margin-bottom: 0.4rem;
  }

  .preview-limitations ul {
    margin: 0;
    padding-left: 1rem;
    color: var(--text-secondary);
    font-size: var(--text-2xs);
    line-height: 1.5;
  }

  .preview-actions {
    display: flex;
    align-items: center;
    gap: 0.55rem;
    padding: 1rem 1.5rem 1.35rem;
  }

  .preview-actions p {
    flex: 1;
    margin: 0;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  @media (max-width: 48rem) {
    .modal-backdrop {
      padding: 0;
    }

    .preview-modal {
      width: 100%;
      max-height: 100vh;
      min-height: 100vh;
      border: 0;
      border-radius: 0;
    }

    .preview-actions {
      align-items: stretch;
      flex-direction: column;
    }

    .preview-actions p {
      width: 100%;
    }
  }
</style>
