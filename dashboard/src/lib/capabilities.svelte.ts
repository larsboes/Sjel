import { axonStatus, hasPanel, type CapabilityView } from "./api";

/**
 * What this machine actually runs, asked of sjel-status rather than compiled in.
 *
 * One module-level rune store rather than a per-component fetch: the nav, the home
 * page and the capabilities page all render the same answer, and three independent
 * polls of the same endpoint would make them disagree for up to a poll interval.
 */
class CapabilityStore {
  items = $state<CapabilityView[]>([]);
  /** Who answers `/api/links` for which kinds. A derived string, so an effect that reads it
   *  reruns when the declarations change (the first poll arriving, a capability enabled) and
   *  not on every poll that returns the same registry. */
  linksKey = $derived(this.items.map((c) => `${c.name}=${(c.links_to ?? []).join("|")}`).join(","));
  offline = $state(false);
  loading = $state(true);
  #timer: ReturnType<typeof setInterval> | undefined;
  #subscribers = 0;

  async refresh(): Promise<void> {
    try {
      this.items = await axonStatus.capabilities();
      this.offline = false;
    } catch {
      // sjel-status is down. The shell keeps rendering, it just cannot say what is up.
      // The last known list is deliberately kept: a nav that empties itself on one
      // failed poll is worse than a stale one.
      this.offline = true;
    } finally {
      this.loading = false;
    }
  }

  /** Poll while at least one component is mounted; stop when the last one leaves. */
  subscribe(intervalMs = 15_000): () => void {
    this.#subscribers += 1;
    if (this.#subscribers === 1) {
      void this.refresh();
      this.#timer = setInterval(() => void this.refresh(), intervalMs);
    }
    return () => {
      this.#subscribers -= 1;
      if (this.#subscribers === 0 && this.#timer !== undefined) {
        clearInterval(this.#timer);
        this.#timer = undefined;
      }
    };
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
