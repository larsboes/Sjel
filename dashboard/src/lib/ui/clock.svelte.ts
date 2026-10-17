/**
 * The clock four surfaces tick, so the tree holds one way to ask what time it is.
 *
 * Measured 2026-10-17: four `setInterval` sites ticked a clock rather than fetching —
 * `+layout` at 1 s, `TimeGrid` and `TripDay` at 60 s, `HomeHorizon` at 30 s. Each wrote the
 * same three lines and each chose a different granularity, which is fine; what was not fine
 * is that "a clock" was one of two things a `setInterval` in this tree could be, so no gate
 * could tell a fetch that forgot its arrival time from a clock that is doing its job.
 *
 * This is not a poller. A clock has no data to be stale and nothing to fail; it only says
 * what time it is, on a beat, while a component is showing it.
 */

export class Clock {
  /** The current instant on this clock, refreshed once a beat while running. */
  at = $state(new Date());

  readonly intervalMs: number;
  #timer: ReturnType<typeof setInterval> | undefined;

  constructor(intervalMs: number) {
    this.intervalMs = intervalMs;
  }

  /** Read the clock at once, so a component mounted just after midnight is not a beat behind. */
  read(): Date {
    this.at = new Date();
    return this.at;
  }

  /** Tick while the component is mounted. Returns the cleanup `onMount` wants. */
  start(): () => void {
    this.read();
    if (this.#timer !== undefined) clearInterval(this.#timer);
    this.#timer = setInterval(() => void this.read(), this.intervalMs);
    return () => this.stop();
  }

  stop(): void {
    if (this.#timer !== undefined) clearInterval(this.#timer);
    this.#timer = undefined;
  }
}

/** A clock that ticks every `intervalMs`. */
export function clock(intervalMs: number): Clock {
  return new Clock(intervalMs);
}
