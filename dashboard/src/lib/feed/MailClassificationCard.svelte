<script lang="ts">
  import type { MailCategory, MailContentExtension } from "$lib/api";

  interface Props {
    mail: MailContentExtension;
    mailCategory: MailCategory;
    lifecycleDate: (timestamp: string | null) => string;
  }

  let { mail, mailCategory, lifecycleDate }: Props = $props();
</script>

<section aria-labelledby="mail-classification-title">
  <p class="section-label" id="mail-classification-title">Mail classification</p>
  <dl>
    <div><dt>Category</dt><dd>{mailCategory}</dd></div>
    <div><dt>Method</dt><dd>{mail.classification_method}</dd></div>
    <div><dt>Revision</dt><dd class="mono">{mail.classification_version}</dd></div>
    {#if mail.gmail_action_at}
      <div><dt>Last Sjel action</dt><dd>{mail.gmail_action}</dd></div>
      <div><dt>Changed</dt><dd>{lifecycleDate(mail.gmail_action_at)}</dd></div>
    {/if}
    {#if mail.gmail_location}
      <div><dt>Gmail location</dt><dd>{mail.gmail_location}</dd></div>
    {/if}
    {#if mail.gmail_observed_at}
      <div><dt>Last checked</dt><dd>{lifecycleDate(mail.gmail_observed_at)}</dd></div>
    {/if}
    {#if mail.gmail_sync_status}
      <div><dt>Sync</dt><dd>{mail.gmail_sync_status}</dd></div>
    {/if}
    {#if mail.gmail_sync_action}
      <div><dt>Pending action</dt><dd>{mail.gmail_sync_action}</dd></div>
    {/if}
    {#if mail.purge_after}
      <div><dt>Sjel cleanup</dt><dd>{lifecycleDate(mail.purge_after)}</dd></div>
    {/if}
  </dl>
  {#if mail.rationale}
    <p class="classification-rationale">{mail.rationale}</p>
  {/if}
  {#if mail.gmail_sync_error}
    <p class="inline-error">{mail.gmail_sync_error}</p>
  {/if}
</section>

<style>
  section {
    padding: 1.15rem 1.25rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    background: var(--surface);
  }

  .section-label {
    margin-bottom: 0.6rem;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  dl {
    display: grid;
    gap: 0.45rem;
    margin: 0;
  }

  dl div {
    display: flex;
    justify-content: space-between;
    gap: 0.75rem;
    font-size: var(--text-xs);
  }

  dt {
    color: var(--text-tertiary);
  }

  dd {
    margin: 0;
    color: var(--text-primary);
    text-align: right;
  }

  .mono {
    font-family: var(--font-mono);
  }

  .classification-rationale {
    margin: 0.65rem 0 0;
    color: var(--text-secondary);
    font-size: var(--text-xs);
    line-height: 1.5;
  }

  .inline-error {
    margin: 0.5rem 0 0;
    color: var(--danger);
    font-size: var(--text-xs);
  }
</style>
