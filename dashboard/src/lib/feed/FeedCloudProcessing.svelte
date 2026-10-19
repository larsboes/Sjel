<script lang="ts">
  import { link } from "$lib/nav";
  import type { CloudCalendarCandidate } from "$lib/feed/cloud-calendar";
  import type {
    CloudProvider,
    CloudProviderUnavailableReason,
    CloudDerivativePreview,
    ContentItemDetail,
  } from "$lib/api";

  let {
    entry = $bindable(),
    cloudProviders = [],
    selectedProviderRole = $bindable(""),
    providerListLoaded = false,
    preparingCloudPreview = false,
    approvingCloudPreview = false,
    runningCloudJob = false,
    queueingCloudDerivative = false,
    cloudPreview = null,
    cloudPreviewError = null,
    calendarCandidates = [],
    calendarProposalBusy = null,
    calendarProposalSaved = new Set<string>(),
    calendarProposalError = null,
    onPrepareCloudPreview,
    onRunCloudJob,
    onQueueCloudDerivative,
    onProposeToCalendar,
    lifecycleDate,
  }: {
    entry: ContentItemDetail;
    cloudProviders?: CloudProvider[];
    selectedProviderRole?: string;
    providerListLoaded?: boolean;
    preparingCloudPreview?: boolean;
    approvingCloudPreview?: boolean;
    runningCloudJob?: boolean;
    queueingCloudDerivative?: boolean;
    cloudPreview?: CloudDerivativePreview | null;
    cloudPreviewError?: string | null;
    calendarCandidates?: CloudCalendarCandidate[];
    calendarProposalBusy?: string | null;
    calendarProposalSaved?: Set<string>;
    calendarProposalError?: string | null;
    onPrepareCloudPreview: () => void;
    onRunCloudJob: () => void;
    onQueueCloudDerivative: () => void;
    onProposeToCalendar: (candidate: CloudCalendarCandidate) => void;
    lifecycleDate: (iso: string | null) => string;
  } = $props();

  const PROVIDER_UNAVAILABLE_LABEL: Record<CloudProviderUnavailableReason, string> = {
    missing_credential: "No API key materialized on this machine",
    billing_expired_or_unknown: "Billing window expired or unknown",
    budget_unavailable: "Today's call ledger could not be read",
    daily_request_limit_reached: "Daily request limit reached",
  };

  function providerStateLabel(provider: CloudProvider): string {
    if (provider.available) return "Available";
    return provider.unavailable_reason === null
      ? "Unavailable, with no reason reported"
      : PROVIDER_UNAVAILABLE_LABEL[provider.unavailable_reason];
  }

  function providerBudgetLabel(provider: CloudProvider): string {
    const limit = `${provider.max_requests_per_day}/day`;
    if (provider.requests_remaining_today === null || provider.requests_used_today === null) {
      return `Unknown · ${limit} configured`;
    }
    return `${provider.requests_remaining_today} left · ${provider.requests_used_today} used · ${limit}`;
  }

  function providerTierLabel(tier: CloudProvider["data_tier"]): string {
    return tier === "pseudonymized_personal"
      ? "Reviewed Personal derivatives"
      : "Public derivatives only";
  }

  function providerBillingLabel(provider: CloudProvider): string {
    const mode = provider.billing_mode === "free_only" ? "Free tier only" : "Prepaid credit";
    return provider.credit_expires_on
      ? `${mode} · credit expires ${provider.credit_expires_on}`
      : `${mode} · no credit expiry`;
  }

  function cloudDispatchLabel(status: ContentItemDetail["cloud_processing"]["dispatch_status"]): string {
    if (status === "queued") return "Ready to run";
    if (status === "running") return "Running";
    if (status === "succeeded") return "Completed";
    if (status === "failed") return "Needs attention";
    return entry?.cloud_processing.status === "staged" ? "Staged locally" : "Not prepared";
  }
</script>

<section class="cloud-processing" aria-labelledby="cloud-processing-title">
  <div class="aside-title">
    <p class="section-label" id="cloud-processing-title">Cloud processing</p>
    <span class:staged={entry.cloud_processing.status === "staged"}>
      {entry.cloud_processing.status === "stale"
        ? "Review again"
        : cloudDispatchLabel(entry.cloud_processing.dispatch_status)}
    </span>
  </div>
  <p class="cloud-explanation">
    {entry.data_class.value === "c0"
      ? "Build a bounded copy and inspect it before a cloud task can use it."
      : "Redact obvious identifiers locally, then inspect the exact derivative before approval."}
  </p>
  <button
    class="btn cloud-preview-button"
    disabled={preparingCloudPreview || approvingCloudPreview}
    onclick={onPrepareCloudPreview}
  >
    {preparingCloudPreview
      ? "Preparing locally…"
      : entry.cloud_processing.status === "staged"
        ? "Review a fresh copy"
        : "Preview cloud-ready copy"}
  </button>
  {#if entry.cloud_processing.status === "staged"}
    {#if entry.cloud_processing.dispatch_status !== "not_queued"}
      <dl class="cloud-job">
        <div><dt>Provider role</dt><dd class="mono">{entry.cloud_processing.provider_role}</dd></div>
        <div><dt>Task</dt><dd>{entry.cloud_processing.task ?? "Content analysis"}</dd></div>
        <div><dt>State</dt><dd>{cloudDispatchLabel(entry.cloud_processing.dispatch_status)}</dd></div>
        <div><dt>Provider calls</dt><dd>{entry.cloud_processing.provider_calls}</dd></div>
      </dl>
      {#if entry.cloud_processing.dispatch_status === "queued"}
        <button
          class="btn btn-primary cloud-run-button"
          disabled={runningCloudJob}
          onclick={onRunCloudJob}
        >
          {runningCloudJob ? "Sending approved copy…" : "Run approved cloud analysis"}
        </button>
        <p class="cloud-boundary">This sends only the reviewed derivative to the selected provider.</p>
      {:else if entry.cloud_processing.dispatch_status === "failed"}
        {#if entry.cloud_processing.last_error}
          <p class="cloud-job-error">{entry.cloud_processing.last_error}</p>
        {/if}
        <button
          class="btn cloud-run-button"
          disabled={runningCloudJob || entry.cloud_processing.provider_calls >= 5}
          onclick={onRunCloudJob}
        >
          {runningCloudJob ? "Retrying…" : "Retry approved cloud analysis"}
        </button>
        {#if entry.cloud_processing.provider_calls >= 5}
          <p class="cloud-boundary">The five-call retry limit has been reached. Review and stage a fresh derivative.</p>
        {/if}
      {:else if entry.cloud_processing.dispatch_status === "running"}
        <p class="cloud-boundary">A provider call is in progress. Reload the entry to refresh its state.</p>
      {/if}

      {#if entry.cloud_processing.dispatch_status === "succeeded" && entry.cloud_processing.result}
        {@const result = entry.cloud_processing.result}
        <div class="cloud-result" aria-label="Cloud content analysis">
          <div class="cloud-result-head">
            <strong>{result.importance} importance</strong>
            {#if entry.cloud_processing.completed_at}
              <span>{lifecycleDate(entry.cloud_processing.completed_at)}</span>
            {/if}
          </div>
          <p>{result.summary}</p>
          <p class="cloud-rationale">{result.importance_rationale}</p>
          {#if result.important_dates.length > 0}
            <div class="cloud-result-group">
              <span class="section-label">Important dates</span>
              {#each result.important_dates as date, index (`${date.label}:${date.date}:${date.source_text}`)}
                {@const candidate = calendarCandidates.find((item) => item.key === `important_dates:${index}`)}
                <div class="cloud-result-item">
                  <strong>{date.label}</strong>
                  <span>{date.date ?? "Date not resolved"}</span>
                  <small>{date.source_text}</small>
                  {#if candidate}
                    <button
                      class="calendar-proposal-button"
                      disabled={calendarProposalBusy !== null || calendarProposalSaved.has(candidate.key)}
                      onclick={() => onProposeToCalendar(candidate)}
                    >
                      {calendarProposalSaved.has(candidate.key)
                        ? "In Calendar review"
                        : calendarProposalBusy === candidate.key
                          ? "Adding…"
                          : "Add to Calendar review"}
                    </button>
                  {/if}
                </div>
              {/each}
            </div>
          {/if}
          {#if result.action_items.length > 0}
            <div class="cloud-result-group">
              <span class="section-label">Actions</span>
              {#each result.action_items as action, index (`${action.text}:${action.due_date}`)}
                {@const candidate = calendarCandidates.find((item) => item.key === `action_items:${index}`)}
                <div class="cloud-result-item">
                  <strong>{action.text}</strong>
                  {#if action.due_date}<span>Due {action.due_date}</span>{/if}
                  {#if candidate}
                    <button
                      class="calendar-proposal-button"
                      disabled={calendarProposalBusy !== null || calendarProposalSaved.has(candidate.key)}
                      onclick={() => onProposeToCalendar(candidate)}
                    >
                      {calendarProposalSaved.has(candidate.key)
                        ? "In Calendar review"
                        : calendarProposalBusy === candidate.key
                          ? "Adding…"
                          : "Add deadline to Calendar review"}
                    </button>
                  {/if}
                </div>
              {/each}
            </div>
          {/if}
          {#if result.topics.length > 0}
            <div class="cloud-topics" aria-label="Topics">
              {#each result.topics as topic (topic)}<span>{topic}</span>{/each}
            </div>
          {/if}
          {#if calendarProposalSaved.size > 0}
            <a class="calendar-review-link" href={link("/calendar")}>Open Calendar review →</a>
          {/if}
          {#if calendarProposalError}
            <p class="cloud-job-error" role="alert">{calendarProposalError}</p>
          {/if}
        </div>
      {/if}
    {/if}
  {/if}
  {#if entry.cloud_processing.dispatch_status === "not_queued"}
    {#if cloudProviders.length > 0}
      <div class="provider-roster" role="radiogroup" aria-labelledby="cloud-provider-roster-title">
        <p class="section-label" id="cloud-provider-roster-title">Cloud providers</p>
        {#each cloudProviders as provider (provider.role)}
          <label
            class="provider-option"
            class:provider-selected={provider.role === selectedProviderRole}
            class:provider-unavailable={!provider.available}
          >
            <span class="provider-head">
              <input
                type="radio"
                name="cloud-provider"
                value={provider.role}
                bind:group={selectedProviderRole}
                disabled={!provider.available || queueingCloudDerivative}
              />
              <strong>{provider.name}</strong>
              <span class="mono">{provider.model}</span>
            </span>
            <dl class="provider-facts">
              <div><dt>Role</dt><dd class="mono">{provider.role}</dd></div>
              <div><dt>Endpoint</dt><dd>{provider.provider_label} · {provider.location}</dd></div>
              <div><dt>Accepts</dt><dd>{providerTierLabel(provider.data_tier)}</dd></div>
              <div><dt>Billing</dt><dd>{providerBillingLabel(provider)}</dd></div>
              <div><dt>Failover</dt><dd>Priority {provider.failover_priority}</dd></div>
              <div><dt>Budget today</dt><dd>{providerBudgetLabel(provider)}</dd></div>
              <div><dt>Input ceiling</dt><dd>{provider.max_input_tokens.toLocaleString("en-GB")} tokens</dd></div>
              <div>
                <dt>State</dt>
                <dd>
                  {providerStateLabel(provider)}
                  {#if provider.unavailable_reason}
                    <span class="mono">{provider.unavailable_reason}</span>
                  {/if}
                </dd>
              </div>
            </dl>
          </label>
        {/each}
      </div>
      <button
        class="btn cloud-queue-button"
        disabled={queueingCloudDerivative
          || !selectedProviderRole
          || entry.cloud_processing.status !== "staged"}
        onclick={onQueueCloudDerivative}
      >
        {queueingCloudDerivative ? "Queueing…" : "Queue for cloud processing"}
      </button>
      {#if entry.cloud_processing.status !== "staged"}
        <p class="provider-missing">Queueing needs a reviewed copy of this item first — preview one above.</p>
      {/if}
      {#if !cloudProviders.some((provider) => provider.available)}
        <p class="provider-missing">The reviewed providers are configured, but their keys have not been materialized on this machine.</p>
      {/if}
    {:else if providerListLoaded}
      <p class="provider-missing">No reviewed HTTPS cloud role is configured.</p>
    {/if}
    <p class="cloud-boundary">Approval stages only the reviewed copy. Queueing records provider intent without sending it.</p>
  {:else if entry.cloud_processing.dispatch_status === "queued"}
    <p class="cloud-boundary">Queued locally. Nothing has been sent until you run the approved analysis.</p>
  {/if}
  {#if cloudPreviewError && !cloudPreview}
    <p class="inline-error" aria-live="polite">{cloudPreviewError}</p>
  {/if}
</section>

<style>
  .section-label {
    margin: 0;
    color: var(--text-tertiary);
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }

  .aside-title {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 0.5rem;
  }

  .cloud-explanation,
  .cloud-boundary {
    margin: 0;
    color: var(--text-secondary);
    font-size: var(--text-xs);
    line-height: 1.5;
  }

  .cloud-preview-button {
    width: 100%;
    justify-content: center;
    margin-top: 0.8rem;
  }

  .provider-roster {
    display: grid;
    gap: 0.5rem;
    margin-top: 0.85rem;
  }

  .provider-roster .section-label {
    margin: 0;
  }

  .provider-option {
    display: grid;
    gap: 0.3rem;
    padding: 0.55rem 0.6rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    background: var(--card-bg);
    cursor: pointer;
  }

  .provider-selected {
    border-color: var(--primary);
  }

  .provider-unavailable {
    cursor: not-allowed;
    opacity: 0.72;
  }

  .provider-head {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0.4rem;
    color: var(--text-primary);
    font-size: var(--text-sm);
  }

  .provider-head .mono {
    color: var(--text-tertiary);
    font-size: 0.625rem;
    overflow-wrap: anywhere;
  }

  .provider-facts {
    display: grid;
    gap: 0.15rem;
    margin: 0.25rem 0 0;
    padding: 0;
  }

  .provider-facts div {
    display: flex;
    justify-content: space-between;
    gap: 0.6rem;
    padding: 0.12rem 0;
    font-size: 0.625rem;
  }

  .provider-facts dt {
    color: var(--text-tertiary);
  }

  .provider-facts dd {
    margin: 0;
    color: var(--text-secondary);
    overflow-wrap: anywhere;
    text-align: right;
  }

  .provider-facts .mono {
    color: var(--text-tertiary);
  }

  .provider-missing {
    margin: 0.4rem 0 0;
    color: var(--text-tertiary);
    font-size: 0.625rem;
    line-height: 1.45;
    overflow-wrap: anywhere;
  }

  .cloud-queue-button,
  .cloud-run-button {
    width: 100%;
    justify-content: center;
    margin-top: 0.65rem;
  }

  .cloud-job {
    display: grid;
    gap: 0.2rem;
    margin: 0.8rem 0 0;
    padding-top: 0.55rem;
    border-top: 1px solid var(--card-border);
  }

  .cloud-job div {
    display: flex;
    justify-content: space-between;
    gap: 0.5rem;
    font-size: var(--text-xs);
  }

  .cloud-job dt {
    color: var(--text-tertiary);
  }

  .cloud-job dd {
    margin: 0;
    color: var(--text-secondary);
  }

  .cloud-boundary {
    margin-top: 0.55rem;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  .cloud-job-error {
    margin: 0.65rem 0 0;
    color: var(--warning-ink);
    font-size: var(--text-2xs);
    line-height: 1.45;
  }

  .cloud-result {
    display: grid;
    gap: 0.7rem;
    margin-top: 0.8rem;
    padding-top: 0.8rem;
    border-top: 1px solid var(--card-border);
  }

  .cloud-result p {
    margin: 0;
    color: var(--text-secondary);
    font-size: var(--text-xs);
    line-height: 1.5;
  }

  .cloud-result .cloud-rationale {
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  .cloud-result-head,
  .cloud-result-item {
    display: grid;
    gap: 0.15rem;
  }

  .calendar-proposal-button {
    justify-self: start;
    margin-top: 0.2rem;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--primary);
    cursor: pointer;
    font: inherit;
    font-size: 0.625rem;
  }

  .calendar-proposal-button:disabled {
    cursor: default;
    opacity: 0.65;
  }

  .calendar-review-link {
    color: var(--primary);
    font-size: var(--text-2xs);
  }

  .cloud-result-head {
    grid-template-columns: 1fr auto;
    align-items: baseline;
    font-size: var(--text-xs);
  }

  .cloud-result-head strong {
    text-transform: capitalize;
  }

  .cloud-result-head span,
  .cloud-result-item span,
  .cloud-result-item small {
    color: var(--text-tertiary);
    font-size: 0.625rem;
  }

  .cloud-result-group {
    display: grid;
    gap: 0.45rem;
  }

  .cloud-result-item {
    padding-left: 0.6rem;
    border-left: 1px solid var(--card-border);
    font-size: var(--text-2xs);
  }

  .cloud-topics {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
  }

  .cloud-topics span {
    padding: 0.2rem 0.4rem;
    border: 1px solid var(--card-border);
    border-radius: 999px;
    color: var(--text-secondary);
    font-size: 0.625rem;
  }

  .staged {
    color: var(--success) !important;
  }

  .inline-error {
    margin: 0.5rem 0 0;
    padding: 0.5rem 0.75rem;
    border-radius: var(--radius-sm);
    background: var(--danger-soft);
    color: var(--danger);
    font-size: 0.8rem;
  }
</style>
