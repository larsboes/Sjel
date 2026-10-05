<script lang="ts">
  import type { Snippet } from "svelte";

  /**
   * The row shell every inbox in this app sits on.
   *
   * Home's ladder, the feed inbox and the mail proposals had each grown their own row —
   * three grids, three paddings, three ideas of where the actions go. This is the one
   * shape: a mark, a body that may shrink to nothing, and actions that never do.
   *
   * It is deliberately never `role="option"`. Every real row holds a title link and one
   * or two buttons, and an option must not contain focusable descendants; the queue is a
   * list and the keyboard cursor moves real DOM focus onto the row instead.
   */
  let {
    id,
    role = "listitem",
    current = false,
    dimmed = false,
    tone = "none",
    href,
    mark,
    children,
    actions,
    secondary,
    meta,
  }: {
    /** Stable DOM id, so a cursor can find the element to focus. */
    id?: string;
    role?: "listitem" | "article";
    /** The keyboard cursor is on this row. Announced as `aria-current`, not selected. */
    current?: boolean;
    /** The row is spent — decided, expired, superseded — and stays in place greyed.
     *  Visual only: the content is still read, because a reader who cannot see the
     *  opacity must still be told what the row says. */
    dimmed?: boolean;
    /** The band's spine segment. `none` draws no spine at all. */
    tone?: "alarm" | "now" | "owed" | "offer" | "none";
    /** The row's primary destination. Renders a stretched hit area behind the content,
     *  so the whole row is clickable without wrapping the nested links. */
    href?: string;
    mark?: Snippet;
    children: Snippet;
    actions?: Snippet;
    /** Actions a reader rarely takes — dismiss, snooze. They wait for the same three asks
     *  that open the meta line, and are always shown where there is no hover. */
    secondary?: Snippet;
    meta?: Snippet;
  } = $props();
</script>

<svelte:element
  this={role === "listitem" ? "li" : "article"}
  {id}
  {role}
  class="row tone-{tone}"
  class:current
  class:dimmed
  class:linked={href !== undefined}
  tabindex="-1"
  aria-current={current ? "true" : undefined}
>
  {#if href}
    <!-- aria-hidden and untabbable: the title inside `children` is the accessible link,
         and two links to the same place would be read twice. -->
    <a class="hit" {href} tabindex="-1" aria-hidden="true">&nbsp;</a>
  {/if}

  {#if mark}<span class="mark">{@render mark()}</span>{/if}

  <div class="body">
    {@render children()}
    {#if meta}<div class="meta">{@render meta()}</div>{/if}
  </div>

  {#if actions || secondary}
    <div class="actions">
      {#if actions}{@render actions()}{/if}
      {#if secondary}<span class="secondary">{@render secondary()}</span>{/if}
    </div>
  {/if}
</svelte:element>

<style>
  .row.dimmed {
    opacity: 0.55;
  }

  .row {
    position: relative;
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: start;
    gap: var(--space-4);
    padding: var(--space-4) var(--space-5) var(--space-4) var(--space-4);
    border-bottom: 1px solid var(--card-border);
    list-style: none;
    transition:
      background-color var(--motion-fast) ease,
      opacity var(--motion-slow) var(--ease-out),
      transform var(--motion-slow) var(--ease-out);
  }

  /* A row arrives rather than appears: on first paint, when a band opens, when the queue
     gains an item. Transform and opacity only, so the rows below do not reflow. */
  @starting-style {
    .row {
      opacity: 0;
      transform: translateY(4px);
    }
  }

  .row:last-child {
    border-bottom: 0;
  }

  .row:hover {
    background-color: var(--surface);
  }

  /* The spine. Two pixels of the band's tone against the row's leading edge — the one
   * place rank is visible without reading a number. Suppressed at `tone="none"`. */
  .row::before {
    content: "";
    position: absolute;
    inset: 0.55rem auto 0.55rem 0;
    width: 3px;
    border-radius: 3px;
    background-color: transparent;
  }

  .tone-alarm::before { background-color: var(--band-alarm); }
  .tone-now::before { background-color: var(--band-now); }
  .tone-owed::before { background-color: var(--band-owed); }
  .tone-offer::before { background-color: var(--band-offer); }

  /* The cursor is a HAIRLINE and a tint, not a filled block. A solid panel behind the
     selected row competed with the row's own content and read heavier than the alarm
     band above it, which inverted the ranking the ladder exists to show. */
  .row.current {
    background-color: color-mix(in srgb, var(--primary) 6%, transparent);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--primary) 22%, transparent);
    border-radius: var(--radius-md);
  }

  .row:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .hit {
    position: absolute;
    inset: 0;
    z-index: 0;
    overflow: hidden;
    text-indent: -999em;
  }

  .mark,
  .body,
  .actions {
    position: relative;
    z-index: 1;
  }

  .mark {
    display: grid;
    place-items: center;
    height: 1.75rem;
    width: 1.75rem;
    border-radius: var(--radius-sm);
    background-color: var(--surface);
    color: var(--text-secondary);
  }

  .body {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: var(--space-1);
  }

  .meta {
    margin-top: var(--space-1);
  }

  /* Q96 one level down: the ladder discloses by band, and the row discloses too. A row
   * says what it is and what it is called; the reason it is here waits to be asked.
   *
   * The ask needs no control, so the row grows none: the keyboard cursor, hover and
   * focus each open it. The line stays in the DOM and in the accessibility tree —
   * only its height collapses — because a reason a screen reader cannot reach is not
   * disclosed, it is deleted.
   *
   * Guarded on `pointer: fine`, so where there is no hover there is no reveal and the
   * line simply stays open. A touch reader must not have to guess. */
  @media (pointer: fine) {
    .meta {
      display: grid;
      grid-template-rows: 0fr;
      margin-top: 0;
      transition: grid-template-rows var(--motion-fast) ease, margin-top var(--motion-fast) ease;
    }

    .meta > :global(*) {
      min-height: 0;
      overflow: hidden;
    }

    .row:hover .meta,
    .row:focus-within .meta,
    .row.current .meta {
      grid-template-rows: 1fr;
      margin-top: var(--space-1);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .meta {
      transition: none;
    }
  }

  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .secondary {
    display: contents;
  }

  /* The secondary actions disclose on the same three asks as the meta line. Opacity, not
     display: the space stays reserved, so the primary action never jumps sideways, and
     the buttons stay in the tab order — focus-within is one of the asks. */
  @media (pointer: fine) {
    .secondary > :global(*) {
      opacity: 0;
      transition: opacity var(--motion-fast) ease;
    }

    .row:hover .secondary > :global(*),
    .row:focus-within .secondary > :global(*),
    .row.current .secondary > :global(*) {
      opacity: 1;
    }
  }

  /* Below the tablet step the actions wrap under the body rather than squeezing the
     title into three words. The mark keeps its column so the spine stays readable. */
  @media (width < 48rem) {
    .row {
      grid-template-columns: auto minmax(0, 1fr);
      row-gap: var(--space-3);
    }

    .actions {
      grid-column: 2;
      flex-wrap: wrap;
    }
  }
</style>
