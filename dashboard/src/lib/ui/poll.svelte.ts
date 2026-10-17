/**
 * The one place the dashboard polls, so no page invents its own fetch-and-timer.
 *
 * Generalised from the two sites that already had the right shape: `lib/capabilities.svelte.ts`
 * took its interval as an argument and polled only while a component was subscribed, and
 * `lib/SyncStatus.svelte` stopped on `visibilitychange`. The other fifteen `setInterval`
 * sites (measured 2026-10-17) each restarted that reasoning and none of the thirteen that
 * fetch data kept the instant its data arrived — which is what made "older than its refresh
 * interval" unsayable on every page at once.
 *
 * Three rules live here rather than at sixteen call sites:
 *
 * - a poller declares its beat, so the age of the view can be measured against it;
 * - it records when data last arrived, so a stale reading is a state and not a guess;
 * - it asks only while something is subscribed and the window is in front, and catches up
 *   on return, so a background tab stops spending the machine's time on a screen nobody
 *   is looking at.
 *
 * `data` is `$state.raw`: every fetch replaces the whole value, so a deep proxy would only
 * cost reads. It survives a failed attempt — a table that empties itself on one bad poll is
 * worse than a stale one, and the age of the view is what tells the reader which it is.
 */

export type PollOptions<T> = {
  /** How often to ask. Declared because a view past its beat shows its age. */
  intervalMs: number;
  /** The read. A rejection is a failed attempt, not a thrown page. */
  fetch: () => Promise<T>;
  /** Ask at once on the first start rather than waiting a whole beat. Default true. */
  immediate?: boolean;
  /** Skip attempts while the window is hidden. Default true. */
  pauseWhenHidden?: boolean;
};

/** Anything that can be handed over as a promise, so a synchronous read needs no wrapper. */
const message = (error: unknown) => (error instanceof Error ? error.message : String(error));

export class Poller<T> {
  /** The last value that arrived, kept across a failed attempt. */
  data = $state.raw<T | null>(null);
  /** The last failure, cleared by the next success. */
  error = $state<string | null>(null);
  /** When `data` last arrived, or null before the first arrival. */
  lastArrived = $state<number | null>(null);
  /**
   * The clock the age is measured against. Advanced by the poller's own beat rather than
   * by a second timer: a page showing its age already re-renders on every beat, and a
   * separate 1 s tick to age a string is a clock nobody asked for.
   */
  nowMs = $state(Date.now());
  /** The beat this poller runs at. Retuned by a subscriber that wants a faster one. */
  intervalMs: number;

  #read: () => Promise<T>;
  #immediate: boolean;
  #pauseWhenHidden: boolean;
  #timer: ReturnType<typeof setInterval> | undefined;
  /**
   * A signal, not a counter. `running` is read by `Freshness`'s derived line, and a plain
   * count would let that line be computed once before `start()` and then never again: the
   * first evaluation short-circuits at "not running", so it never subscribes to the arrival
   * time it is supposed to be measuring. Measured 2026-10-17, on /packs with a forced fetch
   * failure: the page knew the view was a minute old and the chip stayed empty.
   */
  #subscribers = $state(0);
  #holds = 0;
  #inflight = false;

  constructor(options: PollOptions<T>) {
    this.intervalMs = options.intervalMs;
    this.#read = options.fetch;
    this.#immediate = options.immediate ?? true;
    this.#pauseWhenHidden = options.pauseWhenHidden ?? true;
  }

  /** True while something is subscribed: a surface that expects new data. */
  get running(): boolean {
    return this.#subscribers > 0;
  }

  /**
   * One attempt now. Overlapping attempts are dropped rather than queued: a read that takes
   * longer than the beat is a slow surface, and two of them in flight would race to write.
   */
  async refresh(): Promise<void> {
    if (this.#inflight) return;
    this.#inflight = true;
    this.nowMs = Date.now();
    try {
      this.data = await this.#read();
      this.lastArrived = Date.now();
      this.error = null;
    } catch (error) {
      this.error = message(error);
    } finally {
      this.#inflight = false;
      // A failed attempt is also a beat: the age of the view has to move for the failure to
      // become visible at all.
      this.nowMs = Date.now();
    }
  }

  /**
   * Take a value that arrived outside a poll as the new data: the answer to a write, which is
   * the freshest state this surface has. Without this the caller would keep a second copy of
   * the same value and the page would have two ideas about when it last heard.
   */
  adopt(value: T): void {
    this.data = value;
    this.lastArrived = Date.now();
    this.error = null;
  }

  /**
   * Hold the beat while a write of this surface's own state is in flight, so a read that
   * started before the write cannot land after the write's answer and undo it. Time still
   * passes: a save slow enough to miss a beat shows the view going stale, which is true.
   */
  hold(): () => void {
    this.#holds += 1;
    let released = false;
    return () => {
      if (released) return;
      released = true;
      this.#holds -= 1;
    };
  }

  /**
   * Poll while at least one component is mounted; stop when the last one leaves.
   *
   * A later subscriber asking for a faster beat retunes the running timer, so the shared
   * capability store follows its fastest reader instead of the first one. A slower one does
   * not slow the others down.
   */
  start(intervalMs: number = this.intervalMs): () => void {
    this.#subscribers += 1;
    if (this.#subscribers === 1) {
      this.#arm(intervalMs);
      this.#listen();
      if (this.#immediate) void this.refresh();
    } else if (intervalMs < this.intervalMs) {
      this.#arm(intervalMs);
    }
    let stopped = false;
    return () => {
      if (stopped) return;
      stopped = true;
      this.#subscribers -= 1;
      if (this.#subscribers === 0) this.#disarm();
    };
  }

  #arm(intervalMs: number): void {
    this.intervalMs = intervalMs;
    if (this.#timer !== undefined) clearInterval(this.#timer);
    this.#timer = setInterval(() => this.#beat(), intervalMs);
  }

  #disarm(): void {
    if (this.#timer !== undefined) clearInterval(this.#timer);
    this.#timer = undefined;
    this.#unlisten();
  }

  #beat(): void {
    this.nowMs = Date.now();
    if (this.#holds > 0) return;
    if (this.#pauseWhenHidden && typeof document !== "undefined" && document.visibilityState === "hidden") {
      return;
    }
    void this.refresh();
  }

  /**
   * Back in the foreground: read at once rather than waiting out a beat. One listener for
   * the life of the subscription, and it is the poller's own — a page that forgets to remove
   * one leaks it across every navigation.
   */
  #onVisible = (): void => {
    if (document.visibilityState === "visible") void this.refresh();
  };

  #listen(): void {
    if (typeof document === "undefined") return;
    document.addEventListener("visibilitychange", this.#onVisible);
  }

  #unlisten(): void {
    if (typeof document === "undefined") return;
    document.removeEventListener("visibilitychange", this.#onVisible);
  }
}

/** A poller, without the `new`. */
export function poll<T>(options: PollOptions<T>): Poller<T> {
  return new Poller<T>(options);
}
