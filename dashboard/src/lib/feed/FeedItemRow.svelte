<script lang="ts" module>
  /** The kind labels the feed shows, in one place rather than one per surface. */
  export const FEED_KIND_LABEL: Record<string, string> = {
    youtube: "YouTube",
    instagram: "Instagram",
    podcast: "Podcast",
    article: "Article",
    mail: "Mail",
    github: "GitHub",
    arxiv: "arXiv",
    reddit: "Reddit",
  };
</script>

<script lang="ts">
  import type { Snippet } from "svelte";
  import type { FeedEntry } from "../api";
  import { link } from "../nav";
  import Icon from "../Icon.svelte";
  import ListRow from "../ListRow.svelte";
  import { tip } from "../tip";
  import RowMeta from "../RowMeta.svelte";
  import FactorBars, { type Factor } from "../FactorBars.svelte";

  /**
   * One feed item, everywhere a feed item is triaged.
   *
   * Home's ladder and /feed's inbox had grown two rows for the same object, with
   * different actions in different places, and the keyboard triage pass needs one shape to
   * bind to. Callback props rather than `createEventDispatcher`, which Svelte 5 keeps only
   * for compatibility.
   *
   * The class is read off the entry, never passed in and never guessed. `GET /comms/feed`
   * has published `data_class` on the LIST since 2026-09-06
   * (`capabilities/comms/src/server/contracts.rs:174`), so the chip states what comms
   * states. `undefined` still happens and still renders nothing: comms' `FeedFullItem` —
   * what `POST /ingest` and `GET /feed/:id` answer with — carries no class, and
   * `toListEntry` in `routes/feed/+page.svelte` builds a list row from one of those. A
   * missing class is "not stated", which is not the same claim as any of the four.
   *
   * Two of the props below exist because /feed's inbox moved onto this row on 2026-09-07
   * and had them: a decided row greys in place instead of leaving, and the inbox shows the
   * full evaluation breakdown where Home shows factor bars. Both follow `meta`'s rule —
   * the caller replaces a default only where it knows more.
   */
  let {
    entry,
    id,
    current = false,
    busy = false,
    dense = false,
    tone = "offer",
    decided = null,
    undoHint,
    meta,
    detail,
    onkeep,
    ondismiss,
    onopen,
    onexternal,
    onundo,
  }: {
    entry: FeedEntry;
    /** Stable DOM id for the keyboard cursor. */
    id: string;
    current?: boolean;
    /** This row's own action is in flight. */
    busy?: boolean;
    /** Inbox density: no factor bars, no preview line. */
    dense?: boolean;
    tone?: "alarm" | "now" | "owed" | "offer" | "none";
    /** The row has been decided and is greyed in place. Its actions become one Undo. */
    decided?: "keeper" | "dismissed" | null;
    /** Appended to the verdict, e.g. a keyboard hint. Only where one exists. */
    undoHint?: string;
    /** Replaces the default provenance line — used where the caller knows more. */
    meta?: Snippet;
    /** Replaces the default factor bars — same rule as `meta`. */
    detail?: Snippet;
    onkeep?: () => void;
    ondismiss?: () => void;
    /** Called when the row's title is activated by something other than a click. */
    onopen?: () => void;
    /** Called when the original-source link is activated. */
    onexternal?: () => void;
    onundo?: () => void;
  } = $props();

  const href = $derived(link(`/feed/${encodeURIComponent(entry.id)}`));
  const kindLabel = $derived(FEED_KIND_LABEL[entry.kind] ?? entry.kind);

  const factors = $derived<Factor[]>(
    (entry.evaluation?.factors ?? []).map((factor) => ({
      key: factor.key,
      label: factor.label,
      score: factor.score,
      weight: factor.weight,
      // A glyph, not a second hue: the travel factor used to be painted with the reserved
      // success colour, and its context link only renders in the non-compact form.
      mark: factor.context?.kind === "trip" ? "map-pin" : undefined,
      rationale: factor.rationale,
      context: factor.context
        ? { label: factor.context.label, terms: factor.context.matched_terms }
        : null,
    })),
  );

  const dataClass = $derived(entry.data_class ?? null);
  const preview = $derived(entry.summary ?? entry.digest_preview);
  const whyHere = $derived(
    entry.evaluation?.explanation ?? entry.relevance?.rationale ?? "",
  );
</script>

<ListRow {id} {current} {tone} dimmed={decided !== null}>
  <!-- Greyed in place, not removed: the cursor stays where the operator left it and the
       decision is one keystroke away from being retracted. -->
  {#snippet mark()}<Icon name="feed" size={15} />{/snippet}

  <span class="row-kind">
    <!-- The separator is written as an expression, not as literal text: Svelte trims the
         leading whitespace of a block, so ` · ` inside `{#if}` renders as `GitHub· llvm`. -->
    {kindLabel}{#if entry.author}{" · "}{entry.author}{/if}{#if entry.relevance}{" · matches "}{entry.relevance.profile_label}{/if}
  </span>
  <a class="row-title" {href} onclick={() => onopen?.()}>{entry.title ?? entry.url}</a>

  {#if !dense && preview}
    <p class="row-text">{preview}</p>
    {#if !entry.summary}
      <!-- No summary of its own: past the on-device window, so the enrichment drain left
           it and the digest drain took it through the cloud instead. Labelled so the two
           are not confused. -->
      <span class="from-digest">from the digest</span>
    {/if}
  {/if}

  {#if detail}
    <div class="detail">{@render detail()}</div>
  {:else if !dense && factors.length > 0}
    <div class="factors"><FactorBars {factors} compact weighted /></div>
  {/if}

  {#snippet meta()}
    <!-- `whyHere || dataClass`, not `whyHere` alone: a c1 item with no evaluation and no
         profile match has nothing to say about its rank and still has a class to state,
         and the old condition dropped the whole line for it. -->
    {#if meta}{@render meta()}{:else if whyHere || dataClass}<RowMeta
        {whyHere}
        {dataClass}
        candidateStatus="proposed"
      />{/if}
  {/snippet}

  {#snippet actions()}
    {#if decided}
      <span class="verdict mono">
        {decided === "keeper" ? "kept" : "dismissed"}{undoHint ? ` · ${undoHint}` : ""}
      </span>
      {#if onundo}<button class="btn" type="button" onclick={() => onundo()}>Undo</button>{/if}
    {:else}
    <a class="btn" {href}>Read</a>
    {#if onkeep}
      <button
        class="btn btn-soft"
        type="button"
        disabled={busy}
        onclick={() => onkeep()}
      >
        {#if busy}<Icon name="loader" size={13} />{:else if entry.status === "keeper"}Kept{:else}Keep{/if}
      </button>
    {/if}
    {/if}
  {/snippet}

  <!-- The source link and Dismiss are the rare asks: Read and Keep are what a triage pass
       does. A decided row offers only Undo. -->
  {#snippet secondary()}
    {#if !decided}
    <a
      class="btn"
      href={entry.url}
      target="_blank"
      rel="noreferrer"
      aria-label="Original"
      use:tip={"Open the original"}
      onclick={() => onexternal?.()}
    >
      <Icon name="external" size={13} />
    </a>
    {#if ondismiss}
      <button
        class="btn"
        type="button"
        disabled={busy}
        aria-label="Dismiss feed entry"
        use:tip={"Dismiss"}
        onclick={() => ondismiss()}
      >
        <Icon name="close" size={13} />
      </button>
    {/if}
    {/if}
  {/snippet}
</ListRow>

<style>
  .detail,
  .factors {
    margin-top: var(--space-2);
    max-width: 34rem;
  }

  .from-digest {
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  .verdict {
    align-self: center;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }
</style>
