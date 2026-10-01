/**
 * Which model answers a task, decided from what this device can reach right now.
 *
 * The ladder is PRD Q118 (2026-09-25): the phone's own model when it has one, then the Mac,
 * then rules. A rung that cannot run says why, and the caller shows which rung answered. Nothing
 * here performs an action: a model result is text for a card, and every write still goes through
 * the card's own confirm path.
 *
 * Pure: no I/O, so the order and the window arithmetic are tested in `vite/intelligence-ladder.test.ts`.
 */

export type Rung = 'on-device' | 'local' | 'mac' | 'rules';

export const RUNG_LABEL: Record<Rung, string> = {
  'on-device': 'On this device',
  local: 'Local assistant',
  mac: 'Mac',
  rules: 'Rules',
};

/** The order a task tries rungs in. `rules` is last and always available. */
export const LADDER: readonly Rung[] = ['on-device', 'local', 'mac', 'rules'];

export interface RungStatus {
  rung: Rung;
  available: boolean;
  /** Why the rung cannot run, in the plugin's or probe's own word. Unset when available. */
  reason?: string;
  /** Tokens shared by prompt and reply. Unset when unknown. */
  contextTokens?: number;
}

export type TaskKind = 'summarize' | 'classify' | 'extract' | 'interpret' | 'draft';

export interface ModelTask {
  kind: TaskKind;
  instructions?: string;
  prompt: string;
  maxResponseTokens?: number;
}

/** Mirrors `libs/summarize/src/lib.rs`: `CHARS_PER_TOKEN` (sized for dense text, so it
 *  overestimates) and `PROMPT_OVERHEAD_TOKENS` (instructions and chat envelope). */
export const CHARS_PER_TOKEN = 3;
export const PROMPT_OVERHEAD_TOKENS = 400;
export const DEFAULT_REPLY_TOKENS = 512;

/** Same question as `fits_window` in `libs/summarize/src/lib.rs`: does prompt plus reply fit?
 *  An unknown window fits; the backend then refuses with its own error and the ladder moves on. */
export function fitsWindow(task: ModelTask, contextTokens: number | undefined): boolean {
  if (contextTokens === undefined) return true;
  const chars = task.prompt.length + (task.instructions?.length ?? 0);
  const needed =
    Math.floor(chars / CHARS_PER_TOKEN) +
    PROMPT_OVERHEAD_TOKENS +
    (task.maxResponseTokens ?? DEFAULT_REPLY_TOKENS);
  return needed <= contextTokens;
}

export interface Skip {
  rung: Rung;
  reason: string;
}

export interface Plan {
  /** Rungs to try, in order. Always ends with `rules`. */
  tries: Rung[];
  /** Rungs left out, with the reason, for the source line under a card. */
  skipped: Skip[];
}

export function plan(task: ModelTask, statuses: readonly RungStatus[]): Plan {
  const byRung = new Map(statuses.map((status) => [status.rung, status]));
  const tries: Rung[] = [];
  const skipped: Skip[] = [];
  for (const rung of LADDER) {
    if (rung === 'rules') {
      tries.push(rung);
      continue;
    }
    const status = byRung.get(rung);
    if (!status || !status.available) {
      skipped.push({ rung, reason: status?.reason ?? 'not probed' });
    } else if (!fitsWindow(task, status.contextTokens)) {
      skipped.push({ rung, reason: 'too long for this model' });
    } else {
      tries.push(rung);
    }
  }
  return { tries, skipped };
}

/** Error codes that mean "this rung cannot take the task; try the next one". Any other failure
 *  is shown to the reader instead of silently falling through. */
export const FALL_THROUGH_CODES: ReadonlySet<string> = new Set([
  'context_length_exceeded',
  'unavailable',
  'busy',
  'unsupported_locale',
]);

/** Plain words for the reasons the plugin and the probes report. */
export function reasonText(reason: string): string {
  switch (reason) {
    case 'osTooOld':
      return 'needs a newer iOS';
    case 'deviceNotEligible':
      return 'this device has no Apple Intelligence';
    case 'appleIntelligenceNotEnabled':
      return 'Apple Intelligence is off in Settings';
    case 'modelNotReady':
      return 'the model is still downloading';
    case 'systemNotReady':
      return 'not ready yet';
    case 'unsupportedLocale':
      return 'this language is not supported';
    default:
      return reason;
  }
}
