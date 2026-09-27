import { axonStatus, type AxonStatusHealth } from "../../api";
import { link } from "../../nav";
import type { DecisionKind } from "../decisions";

/**
 * Band 10000 — PRD §8.1, System health. Nothing outranks "Sjel is not running".
 *
 * One row or none, and its urgency is zero: there is nothing above it for urgency to
 * break a tie against.
 */
const system: DecisionKind<AxonStatusHealth | null, AxonStatusHealth> = {
  key: "system",
  band: 10_000,
  label: "System status",
  capability: null,
  view: "SystemRow",

  load: () => axonStatus.health(),
  rows: (health) => (health && !health.ok ? [health] : []),
  id: () => "health",
  title: () => "Check autostart",
  urgency: () => 0,
  href: () => link("/capabilities"),

  whyHere: () => "At least one service that should be running is not responding.",
  startOrDueAt: () => null,
  candidateStatus: () => "open",
  dataClass: () => null,
  processingRoute: () => null,
};

export default system;
