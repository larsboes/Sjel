<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import PageHeader from "$lib/PageHeader.svelte";
  import { sjelStatus, type SetupPlan, type SetupProbe } from "$lib/api";

  /**
   * What this machine still needs, and what applying it would do.
   *
   * The page renders `sjel setup plan --json` and never decides anything itself: the probes, the
   * steps and the guard all belong to `tools/sjel-cli/src/setup/`. Two things it deliberately
   * cannot do:
   *
   * - It cannot supply a credential. `POST /setup/apply` carries a URL and an interval and has no
   *   token field, so a replica's token stays on the CLI's stdin where it is not copied into a
   *   browser, a proxy and an access log. The card says so rather than pretending the form is
   *   complete.
   * - It does not poll. A plan is filesystem-only and cheap, but a verify leaves the machine and
   *   an apply writes, so both are asked for by a click; a timer would make an install happen
   *   because a tab was left open.
   */
  let plans = $state<SetupPlan[] | null>(null);
  let readError = $state<string | null>(null);
  let busy = $state<"plan" | "verify" | "apply" | null>(null);
  let actionError = $state<string | null>(null);
  let applied = $state<string[] | null>(null);
  /** The step list a reader is confirming. Null means no confirmation is open. */
  let confirming = $state<SetupPlan | null>(null);
  let url = $state("");
  let interval = $state("");

  async function load(): Promise<void> {
    busy = "plan";
    readError = null;
    try {
      plans = (await sjelStatus.setup()).components;
    } catch (e) {
      readError = e instanceof Error ? e.message : String(e);
    } finally {
      busy = null;
    }
  }

  async function verify(): Promise<void> {
    busy = "verify";
    actionError = null;
    try {
      const answer = await sjelStatus.setupVerify();
      // Verification replaces the probes and leaves the steps alone: they describe files, not a
      // connection, and re-planning on every check would hide a step the apply just completed.
      plans = (plans ?? []).map((plan) => {
        const found = answer.components.find((c) => c.component === plan.component);
        return found ? { ...plan, probes: found.probes } : plan;
      });
    } catch (e) {
      actionError = e instanceof Error ? e.message : String(e);
    } finally {
      busy = null;
    }
  }

  async function confirmApply(plan: SetupPlan): Promise<void> {
    busy = "apply";
    actionError = null;
    applied = null;
    try {
      const answer = await sjelStatus.setupApply({
        component: plan.component,
        ...(url.trim() ? { url: url.trim() } : {}),
        ...(interval.trim() && Number(interval) > 0 ? { interval_secs: Number(interval) } : {}),
      });
      const mine = answer.components.find((c) => c.component === plan.component);
      applied = mine?.applied ?? [];
      if (mine) {
        plans = (plans ?? []).map((p) => (p.component === plan.component ? { ...p, probes: mine.verify } : p));
      }
      confirming = null;
      url = "";
      interval = "";
    } catch (e) {
      actionError = e instanceof Error ? e.message : String(e);
    } finally {
      busy = null;
    }
  }

  /** How a probe reads in one word, so the glyph is never the only signal. */
  function markLabel(mark: SetupProbe["mark"]): string {
    switch (mark) {
      case "ok":
        return "in place";
      case "missing":
        return "missing";
      case "warn":
        return "check it";
      case "blocked":
        return "needs you";
    }
  }

  function applyable(plan: SetupPlan): number {
    return plan.steps.filter((s) => s.kind === "apply" && !s.done).length;
  }

  onMount(() => {
    void load();
  });
</script>

<PageHeader
  badge="Setup"
  title="What this machine still needs"
  desc="Probed from this machine, not remembered. Applying runs only the steps Sjel owns; a step whose file belongs to something else is printed with its path."
>
  {#snippet actions()}
    <div class="actions">
      <button class="ghost" onclick={load} disabled={busy !== null}>
        <Icon name="arrow-right" />
        Re-read
      </button>
      <button class="ghost" onclick={verify} disabled={busy !== null}>
        <Icon name="server" />
        Check the connection
      </button>
    </div>
  {/snippet}
</PageHeader>

{#if readError}
  <p class="error">{readError}</p>
{:else if plans === null}
  <p class="muted">Reading this machine…</p>
{:else if plans.length === 0}
  <p class="muted">Nothing to set up: no component has a plan on this build.</p>
{:else}
  {#each plans as plan (plan.component)}
    <section class="card">
      <header class="head">
        <div>
          <h2>{plan.component}</h2>
          <p class="purpose">{plan.purpose}</p>
        </div>
        {#if busy === "verify"}
          <span class="muted">asking the primary…</span>
        {/if}
      </header>

      <ul class="probes">
        {#each plan.probes as probe (probe.id)}
          <li class="probe {probe.mark}">
            <span class="mark" aria-hidden="true">{probe.mark === "ok" ? "✓" : probe.mark === "missing" ? "✗" : probe.mark === "warn" ? "!" : "·"}</span>
            <div class="probe-body">
              <p class="probe-line">
                <strong>{probe.label}</strong>
                <span class="sr">{markLabel(probe.mark)}</span>
                <span class="detail">{probe.detail}</span>
              </p>
            </div>
          </li>
        {/each}
      </ul>

      {#if plan.steps.length > 0}
        <h3>Steps</h3>
        <ul class="steps">
          {#each plan.steps as step}
            <li class="step {step.kind}">
              <span class="kind">{step.kind}</span>
              <div>
                <p class="step-line">{step.done ? "✓ " : ""}{step.label}</p>
                <p class="detail pre">{step.detail}</p>
              </div>
            </li>
          {/each}
        </ul>
      {/if}

      {#if applyable(plan) > 0}
        {#if confirming === plan}
          <div class="confirm">
            <p class="confirm-lead">
              Applying writes {applyable(plan)} setting(s) to the private overlay and then verifies.
              It does not touch a Cargo.toml, start a service or accept a credential — those stay on
              the command line, where a token never leaves stdin.
            </p>
            <label>
              Primary URL
              <input bind:value={url} placeholder="libsql://your-database.turso.io" spellcheck="false" />
            </label>
            <label>
              Pull interval, seconds
              <input bind:value={interval} inputmode="numeric" placeholder="60" />
            </label>
            <div class="confirm-actions">
              <button class="primary" onclick={() => confirmApply(plan)} disabled={busy !== null}>
                Apply {applyable(plan)} step(s)
              </button>
              <button class="ghost" onclick={() => (confirming = null)} disabled={busy !== null}>
                Cancel
              </button>
            </div>
            <p class="detail">
              The credential is not part of this form: run
              <code>sjel setup apply turso --confirm --token-stdin &lt; token.txt</code> on the machine.
            </p>
          </div>
        {:else}
          <button class="primary" onclick={() => (confirming = plan)} disabled={busy !== null}>
            Review and apply {applyable(plan)} step(s)
          </button>
        {/if}
      {/if}

      {#if applied}
        <div class="applied">
          {#each applied as line}
            <p class="detail">{line}</p>
          {/each}
        </div>
      {/if}
    </section>
  {/each}

  {#if actionError}
    <p class="error">{actionError}</p>
  {/if}

  <p class="muted foot">
    Read from <code>sjel setup plan --json</code>. The same three verbs run headless:
    <code>sjel setup plan|apply|verify turso</code>.
  </p>
{/if}

<style>
  .card {
    border: 1px solid var(--card-border);
    border-radius: 12px;
    padding: 1.1rem 1.2rem 1.3rem;
    margin-bottom: 1rem;
    background: var(--surface);
  }
  .head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 1rem;
  }
  h2 {
    font-size: 1.05rem;
    margin: 0;
  }
  .purpose {
    margin: 0.15rem 0 0.9rem;
    color: var(--text-secondary);
    font-size: 0.9rem;
  }
  h3 {
    font-size: 0.78rem;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-secondary);
    margin: 1.1rem 0 0.5rem;
  }
  .probes,
  .steps {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .probe {
    display: grid;
    grid-template-columns: 1.1rem 1fr;
    gap: 0.5rem;
    padding: 0.3rem 0;
    align-items: start;
  }
  .probe .mark {
    font-family: var(--font-mono);
    color: var(--text-secondary);
  }
  .probe.ok .mark {
    color: var(--success);
  }
  .probe.missing .mark {
    color: var(--danger);
  }
  .probe-line {
    margin: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    align-items: baseline;
  }
  .probe-line strong {
    font-weight: 600;
  }
  .step {
    display: grid;
    grid-template-columns: 4.4rem 1fr;
    gap: 0.6rem;
    padding: 0.35rem 0;
    border-top: 1px solid var(--card-border);
  }
  .step .kind {
    font-size: 0.72rem;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-secondary);
    padding-top: 0.15rem;
  }
  .step.apply .kind {
    color: var(--accent);
  }
  .step-line {
    margin: 0;
    font-weight: 550;
  }
  .detail {
    margin: 0.15rem 0 0;
    color: var(--text-secondary);
    font-size: 0.85rem;
  }
  .pre {
    white-space: pre-wrap;
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
  }
  .actions {
    display: flex;
    gap: 0.5rem;
  }
  button {
    font: inherit;
    border-radius: 8px;
    padding: 0.4rem 0.75rem;
    cursor: pointer;
    display: inline-flex;
    gap: 0.4rem;
    align-items: center;
  }
  button:disabled {
    opacity: 0.55;
    cursor: default;
  }
  .primary {
    background: var(--accent);
    color: white;
    border: 1px solid transparent;
    margin-top: 0.6rem;
  }
  .ghost {
    background: transparent;
    color: inherit;
    border: 1px solid var(--card-border);
  }
  .confirm {
    margin-top: 0.8rem;
    padding: 0.9rem 1rem;
    border: 1px solid var(--accent);
    border-radius: 10px;
  }
  .confirm-lead {
    margin: 0 0 0.7rem;
    font-size: 0.9rem;
  }
  .confirm label {
    display: block;
    font-size: 0.82rem;
    color: var(--text-secondary);
    margin-bottom: 0.5rem;
  }
  .confirm input {
    display: block;
    width: 100%;
    margin-top: 0.2rem;
    padding: 0.4rem 0.5rem;
    font: inherit;
    border: 1px solid var(--card-border);
    border-radius: 6px;
    background: var(--page-bg);
    color: inherit;
  }
  .confirm-actions {
    display: flex;
    gap: 0.5rem;
    margin-top: 0.4rem;
  }
  .applied {
    margin-top: 0.8rem;
    padding: 0.7rem 0.9rem;
    border-radius: 8px;
    background: var(--page-bg);
  }
  .error {
    color: var(--danger);
  }
  .muted {
    color: var(--text-secondary);
  }
  .foot {
    font-size: 0.85rem;
    margin-top: 0.4rem;
  }
  code {
    font-family: var(--font-mono);
    font-size: 0.85em;
  }
</style>
