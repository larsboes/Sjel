<script lang="ts">
  import { tip } from "$lib/tip";
  import MarkdownDocument from "$lib/feed/MarkdownDocument.svelte";
  import MermaidDiagram from "$lib/feed/MermaidDiagram.svelte";
  import ChartFigure from "$lib/feed/ChartFigure.svelte";
  import { comms, type ContentDigest, type ContentItemDetail } from "$lib/api";

  let {
    entry = $bindable(),
  }: {
    entry: ContentItemDetail;
  } = $props();

  let digestBusy = $state<"standard" | "detailed" | "diagram" | "chart" | null>(null);
  let digestError = $state<string | null>(null);
  let focusInput = $state("");

  const digestText = $derived(
    entry?.digest?.state === "generated" ? entry.digest.text : null,
  );

  const SHAPE_LABEL: Record<string, string> = {
    brief: "Brief",
    standard: "Standard",
    sectioned: "Sectioned",
    none: "None",
  };

  const digestRungLabel = $derived(
    entry?.digest
      ? `${SHAPE_LABEL[entry.digest.shape] ?? entry.digest.shape}${
          entry.digest.depth === "detailed" ? " · you asked for more" : ""
        }`
      : "",
  );

  function digestExplanation(digest: ContentDigest): string {
    switch (digest.state) {
      case "generated":
        return "";
      case "skipped_short":
        return "Too short to be worth a digest — the source is already the summary. Press More detail to force one.";
      case "remote_refused":
        return "This item is not Public and the configured model is not local, so nothing was sent.";
      case "local_refused":
        return "Secret content enters no prompt, so nothing was sent to any model.";
      case "unconfigured":
        return "No summarization model is configured on this machine.";
      case "timeout":
        return "The local model did not answer in time.";
      case "capacity_aborted":
        return "The local server ran out of memory part-way through this one. It will retry on its own.";
      case "empty_response":
        return "The local model answered with nothing.";
      default:
        return digest.last_error ?? "The local model could not be reached.";
    }
  }

  function diagramExplanation(digest: ContentDigest): string {
    if (digest.diagram_error) return digest.diagram_error;
    return digestExplanation({
      ...digest,
      state: (digest.diagram_state ?? "model_error") as ContentDigest["state"],
    });
  }

  function focusTerms(): string[] {
    return focusInput
      .split(",")
      .map((term) => term.trim())
      .filter(Boolean);
  }

  async function runDigest(depth: "standard" | "detailed") {
    if (!entry || digestBusy) return;
    digestBusy = depth;
    digestError = null;
    try {
      const digest = await comms.digest(entry.source, entry.id, { depth, focus: focusTerms() });
      entry.digest = digest;
      focusInput = digest.focus.join(", ");
    } catch (cause) {
      digestError = cause instanceof Error ? cause.message : String(cause);
    } finally {
      digestBusy = null;
    }
  }

  async function runDiagram() {
    if (!entry || digestBusy) return;
    digestBusy = "diagram";
    digestError = null;
    try {
      entry.digest = await comms.diagram(entry.source, entry.id);
    } catch (cause) {
      digestError = cause instanceof Error ? cause.message : String(cause);
    } finally {
      digestBusy = null;
    }
  }

  async function runChart() {
    if (!entry || digestBusy) return;
    digestBusy = "chart";
    digestError = null;
    try {
      entry.digest = await comms.chart(entry.source, entry.id);
    } catch (cause) {
      digestError = cause instanceof Error ? cause.message : String(cause);
    } finally {
      digestBusy = null;
    }
  }
</script>

<section class="note digest" aria-labelledby="digest-title">
  <div class="digest-head">
    <p class="section-label" id="digest-title">Digest</p>
    {#if digestText}
      <span class="digest-rung">{digestRungLabel}</span>
    {/if}
  </div>

  {#if digestText}
    <MarkdownDocument content={digestText} compact />
  {:else if entry.digest}
    <p class="digest-note">{digestExplanation(entry.digest)}</p>
  {:else}
    <p class="digest-note">Not generated yet.</p>
  {/if}

  {#if entry.digest?.focus?.length}
    <p class="digest-focus">
      Asked to focus on: {entry.digest.focus.join(", ")}
    </p>
  {/if}
  {#if entry.digest && entry.digest.redactions > 0}
    <p class="digest-redactions">
      {entry.digest.redactions}
      {entry.digest.redactions === 1 ? "entity was" : "entities were"} redacted before this
      was stored — this item's class requires redaction, and a digest must not republish
      what the sweep removed.
    </p>
  {/if}

  <div class="digest-controls">
    <label class="digest-field">
      <span>Focus on</span>
      <input
        type="text"
        placeholder="optional — comma-separated, e.g. cost, evaluation"
        bind:value={focusInput}
        disabled={digestBusy !== null}
      />
    </label>
    <div class="digest-buttons">
      <button
        class="btn"
        disabled={digestBusy !== null}
        onclick={() => runDigest("standard")}
        use:tip={"Regenerate at the rung this source's length earns"}
      >
        {digestBusy === "standard" ? "Digesting…" : "Regenerate"}
      </button>
      <button
        class="btn btn-primary"
        disabled={digestBusy !== null}
        onclick={() => runDigest("detailed")}
        use:tip={"One rung further up the same ladder"}
      >
        {digestBusy === "detailed" ? "Digesting…" : "More detail"}
      </button>
      <button
        class="btn"
        disabled={digestBusy !== null}
        onclick={runDiagram}
        use:tip={"Draw this item as a Mermaid diagram"}
      >
        {digestBusy === "diagram" ? "Drawing…" : "Diagram"}
      </button>
      <button
        class="btn"
        disabled={digestBusy !== null}
        onclick={runChart}
        use:tip={"Pull the item's numbers out and plot them"}
      >
        {digestBusy === "chart" ? "Reading…" : "Chart"}
      </button>
    </div>
  </div>

  {#if entry.digest?.chart}
    <ChartFigure data={entry.digest.chart} />
  {:else if entry.digest?.chart_state && entry.digest.chart_state !== "generated"}
    <p class="digest-note">
      {entry.digest.chart_state === "skipped_short"
        ? "No chart: this source has no set of comparable numbers in it. Most prose does not."
        : `No chart: ${entry.digest.chart_error ?? "the local model could not be reached."}`}
    </p>
  {/if}

  {#if entry.digest?.diagram}
    <MermaidDiagram source={entry.digest.diagram} />
  {:else if entry.digest?.diagram_state && entry.digest.diagram_state !== "generated"}
    <p class="digest-note">No diagram: {diagramExplanation(entry.digest)}</p>
  {/if}

  {#if digestError}<p class="inline-error" aria-live="polite">{digestError}</p>{/if}
  {#if entry.digest}
    <p class="digest-provenance">
      {entry.digest.source_chars.toLocaleString()} characters of source · produced by
      <code>{entry.digest.producer}</code>
    </p>
  {/if}
</section>

<style>
  .note {
    padding: 1.25rem 1.5rem;
    border-radius: var(--radius-md);
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-left: 2px solid var(--primary);
  }

  .digest {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .digest-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 1rem;
  }

  .digest-rung {
    color: var(--text-tertiary);
    font-size: var(--text-xs);
    letter-spacing: 0.02em;
  }

  .digest-note,
  .digest-focus,
  .digest-redactions,
  .digest-provenance {
    margin: 0;
    color: var(--text-secondary);
    font-size: 0.8rem;
    line-height: 1.55;
  }

  .digest-redactions {
    color: var(--warning-ink);
  }

  .digest-provenance {
    color: var(--text-tertiary);
    font-size: 0.7rem;
  }

  .digest-provenance code {
    font-family: var(--font-mono);
    font-size: 0.7rem;
    overflow-wrap: anywhere;
  }

  .digest-controls {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: 0.75rem;
    padding-top: 0.25rem;
    border-top: 1px solid var(--card-border);
  }

  .digest-field {
    display: flex;
    flex: 1 1 16rem;
    flex-direction: column;
    gap: 0.3rem;
  }

  .digest-field span {
    color: var(--text-tertiary);
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }

  .digest-field input {
    padding: 0.45rem 0.6rem;
    border: 1px solid var(--input-border);
    border-radius: var(--radius-sm);
    background: var(--input-bg);
    color: var(--text-primary);
    font-family: inherit;
    font-size: 0.85rem;
  }

  .digest-buttons {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
  }

  .inline-error {
    margin: 0;
    padding: 0.5rem 0.75rem;
    border-radius: var(--radius-sm);
    background: var(--danger-soft);
    color: var(--danger);
    font-size: 0.8rem;
  }

  .section-label {
    margin: 0;
    color: var(--text-tertiary);
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
</style>
