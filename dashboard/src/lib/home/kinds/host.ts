import { axonStatus, type HostWatchFinding } from "../../api";
import { link } from "../../nav";
import { RESCALE, type DecisionKind } from "../decisions";

/**
 * Band 900 — PRD §9, a resource rule broken on this machine.
 *
 * Above a trip and below the capability-health card, because a runaway process is worse
 * than a plan that can wait and less urgent than "Sjel is not running". The longer a
 * condition has persisted the higher it sits: a process that has been pinning a core
 * since Tuesday is the one to look at.
 *
 * Old expression: `900 + min(90, days * 10)`, saturating at nine days.
 */
const host: DecisionKind<HostWatchFinding[], HostWatchFinding> = {
  key: "host",
  band: 900,
  label: "Host watch",
  capability: null,
  view: "HostRow",

  // Swallowed on purpose, and the only kind that does. The machine may simply never have
  // run the watch; a "Host watch unavailable" banner on a healthy machine is the false
  // alarm the watcher's own README refuses to produce.
  load: (ctx) => axonStatus.hostWatch(ctx.signal).catch(() => []),
  rows: (findings) => findings,
  id: (finding) => finding.id,
  title: (finding) => finding.title,
  urgency: (finding, ctx) =>
    RESCALE(Math.min(90, Math.max(0, -ctx.daysUntil(finding.first_seen)) * 10), 90),
  href: () => link("/systems"),

  // The note is multi-line and is commands to copy; the why-here line is its opening.
  whyHere: (finding) => finding.note.split("\n")[0].trim(),
  startOrDueAt: (finding) => finding.first_seen,
  candidateStatus: () => "open",
  dataClass: () => null,
  processingRoute: () => null,
};

export default host;
