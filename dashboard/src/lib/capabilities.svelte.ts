import { sjelStatus, hasPanel, type CapabilityView } from "./api";
import { poll } from "./ui/poll.svelte";

/**
 * What this machine actually runs, asked of sjel-status rather than compiled in.
 *
 * One module-level rune store rather than a per-component fetch: the nav, the home
 * page and the capabilities page all render the same answer, and three independent
 * polls of the same endpoint would make them disagree for up to a poll interval.
 *
 * The fetch, the beat and the arrival time are `ui/poll.svelte.ts`'s. What is left here is
 * the registry itself: the derived key an effect watches, the offline read, and the
 * panels a shell page can open. A page that shows this list can hand `polled` to
 * `ui/Freshness.svelte` and say how old the view is.
 */
class CapabilityStore {
  #polled = poll({ intervalMs: 15_000, fetch: () => sjelStatus.capabilities() });

  /** The last list that arrived. A failed poll keeps it: an empty nav is worse than a stale one. */
  get items(): CapabilityView[] {
    return this.#polled.data ?? [];
  }

  /** Who answers `/api/links` for which kinds. A derived string, so an effect that reads it
   *  reruns when the declarations change (the first poll arriving, a capability enabled) and
   *  not on every poll that returns the same registry. */
  linksKey = $derived(this.items.map((c) => `${c.name}=${(c.links_to ?? []).join("|")}`).join(","));

  /** sjel-status is down. The shell keeps rendering, it just cannot say what is up. */
  get offline(): boolean {
    return this.#polled.error !== null;
  }

  get loading(): boolean {
    return this.#polled.data === null && this.#polled.error === null;
  }

  /** The age of this view, for a page that shows the registry. */
  get polled() {
    return this.#polled;
  }

  async refresh(): Promise<void> {
    await this.#polled.refresh();
  }

  /**
   * Poll while at least one component is mounted; stop when the last one leaves.
   *
   * The beat follows the fastest reader, so the capabilities page's 5 s wins over the
   * default 15 s while it is open. It does not slow back down when that reader leaves: one
   * extra read every 15 s is cheaper than a timer that retunes under the pages still on it.
   */
  subscribe(intervalMs = 15_000): () => void {
    return this.#polled.start(intervalMs);
  }

  byName(name: string): CapabilityView | undefined {
    return this.items.find((c) => c.name === name);
  }

  /** Capabilities that serve their own UI, in registry order. */
  get panels(): CapabilityView[] {
    return this.items.filter(hasPanel);
  }
}

export const capabilities = new CapabilityStore();
