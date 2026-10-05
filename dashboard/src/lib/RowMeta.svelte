<script lang="ts">
  import type { DataClass } from "./home/decisions";
  import { tip } from "$lib/tip";

  /**
   * The one place a row states why it is here and where its rank came from.
   *
   * PRD:227 — "Evidence before automation. Every claim the system shows keeps its source
   * and its decision state. Agents rank and explain." A row that moved because a model
   * said so and cannot say why is the failure that principle names, so `method` renders
   * beside `whyHere` whenever a model produced the rank.
   *
   * Every optional field renders nothing when null. The data class is shown where a
   * capability publishes one — mail, feed, finance, calendar, vault tasks and scouting
   * since B50 — and is silent where none does, rather than guessed: the dashboard owns no
   * data (dashboard/README.md:7-9) and a class it invented would be a false provenance
   * claim.
   *
   * The chip names the CLASS, where the page it replaces printed the word "Redacted" on
   * the strict two. A deliberate departure from the design, recorded here rather than left
   * silent: nothing on this row is redacted — a mail row renders its subject and its
   * snippet in full — so "Redacted" claimed a reduction that had not happened. The class
   * is the true statement, it is what says where the row may be processed, and it stays
   * inert on rank either way.
   *
   * The four WORDS are not this file's to choose. `PRD Sjel.md` §6.1's class table names
   * them — C0 Public, C1 Mine, C2 Others, C3 Secret — under a ruling whose own sentence is
   * "two vocabularies standing side by side is the one outcome not allowed", and
   * `libs/content-item/src/lib.rs` (`DataClass::new`) is the implementation the PRD names.
   * This component printed Public/Money/Private/Sensitive until 2026-09-08: "Private" is
   * the pre-Q27 name of the retired `vault` class, and "Money" was wrong on its face —
   * every calendar entry, every vault note and every unclassified feed item is c1, and
   * none of them is about money. B50 lights the chip on five more surfaces, so it had to
   * go first.
   *
   * Corrected by the verifier, 2026-09-08. This paragraph also said "nothing rendered it
   * on a c1 row before B50, which is how it survived". That is not true and the reason
   * matters: `content_item::DataClass::classify_mail` ends "Mail metadata is Mine by
   * default", so c1 is the ORDINARY class of a triage row, and `kinds/mail.ts` has fed it
   * to `MailRow`, which has handed it here, for as long as the chip has existed. Home has
   * been printing "Money" on ordinary mail. How many live rows is not measurable from the
   * repo — it needs the store — so the honest statement is that the code path was always
   * there, not that nothing walked it.
   */
  let {
    whyHere,
    dataClass = null,
    processingRoute = null,
    method = null,
    candidateStatus,
  }: {
    whyHere: string;
    dataClass?: DataClass | null;
    processingRoute?: "local" | "cloud" | null;
    /** The classifier or evaluator that produced the rank, e.g. a model revision. */
    method?: string | null;
    candidateStatus?: "proposed" | "accepted" | "open";
  } = $props();

  const CLASS_LABEL: Record<DataClass, string> = {
    c0: "Public",
    c1: "Mine",
    c2: "Others",
    c3: "Secret",
  };

  /**
   * A literal outside the vocabulary reads back as the STRICTEST word, never the loosest.
   *
   * The same arm `libs/content-item/src/lib.rs` (`DataClass::new`) carries for the same
   * reason — "a stale literal must not render as Public". `dataClass` is typed, so this is
   * unreachable from a caller svelte-check saw; it is reachable from JSON, which is where
   * every class on this row comes from. Without it the lookup returned undefined and the
   * chip rendered EMPTY, so a class nobody could read looked like no class at all.
   */
  const label = $derived(dataClass === null ? null : (CLASS_LABEL[dataClass] ?? CLASS_LABEL.c3));

  const redacted = $derived(label === CLASS_LABEL.c2 || label === CLASS_LABEL.c3);
</script>

<p class="why">
  {#if whyHere}<span class="text">{whyHere}</span>{/if}
  {#if method}<span class="method mono" use:tip={"What ranked this row"}>{method}</span>{/if}
  {#if label}
    <!-- Visible, and with no effect on rank: the ranking policy states plainly that the
         data class does not move a score. It says where the row may be processed. -->
    <span class="class" class:redacted>{label}</span>
  {/if}
  {#if processingRoute}<span class="route">{processingRoute}</span>{/if}
  {#if candidateStatus === "accepted"}
    <!-- Only `accepted` renders. Every row on a decision ladder is by definition proposed
         or open, so those two words were a chip that appeared on every row and told the
         reader nothing; `accepted` is the one that marks a call already made. -->
    <span class="status">accepted</span>
  {/if}
</p>

<style>
  .why {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--space-1) var(--space-3);
    margin: 0;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
    line-height: var(--leading-normal);
  }

  .text {
    color: var(--text-secondary);
  }

  .method,
  .route,
  .status {
    font-size: var(--text-2xs);
  }

  .class {
    padding: 0 0.3em;
    border-radius: var(--radius-sm);
    background-color: var(--surface);
  }

  .class.redacted {
    color: var(--warning-ink);
    background-color: var(--warning-soft);
  }
</style>
