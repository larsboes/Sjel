import { trips, type TripPlan } from "../../api";
import { link } from "../../nav";
import { RESCALE, type DecisionKind } from "../decisions";

/** True while any leg is still undecided, or no route exists at all. */
export function tripNeedsPlanning(plan: TripPlan): boolean {
  return (
    plan.stages.length === 0 ||
    plan.stages.some((stage) => stage.status === "planning" || stage.status === "option_selected")
  );
}

/**
 * Names the leg that is actually open, not how many are.
 *
 * Counting produced the byte-identical sentence "One stage still needs a decision." under
 * every trip on the page, three in a row, which told the operator nothing and read as
 * boilerplate. The leg and its state are both already in the data.
 */
export function tripGap(plan: TripPlan): string {
  if (plan.stages.length === 0) return "no route planned yet";
  const open = plan.stages.filter(
    (stage) => stage.status === "planning" || stage.status === "option_selected",
  );
  const first = open[0];
  if (!first) return "everything booked";
  // `option_selected` is a real distinction: a connection is chosen, so the remaining act
  // is booking it, not deciding it.
  const state = first.status === "option_selected" ? "chosen but not booked" : "no connection chosen";
  const rest = open.length > 1 ? `, +${open.length - 1} more` : "";
  // The leg only earns its words on a multi-stage trip. On a single-stage one it just
  // repeats the destination the row already shows above it.
  if (plan.stages.length < 2) return `${state}${rest}`;
  const leg = [first.origin?.name, first.destination?.name].filter(Boolean).join(" → ");
  return leg ? `${leg}, ${state}${rest}` : `${state}${rest}`;
}

/**
 * Band 800 — PRD §8.1, a trip that needs planning.
 *
 * Old expression: `800 + (days <= 14 ? max(0, 400 - days * 20) : 0)`, reaching 1200 on the
 * day of departure — which is how a trip starting today outranked every host-watch
 * finding. The slope is unchanged; only its range is.
 */
const trip: DecisionKind<TripPlan[], TripPlan> = {
  key: "trip",
  band: 800,
  label: "Travel",
  capability: "trips",
  view: "TripRow",

  load: () => trips.list(),
  rows: (plans, ctx) =>
    plans.filter((plan) => plan.date_end >= ctx.todayKey && tripNeedsPlanning(plan)),
  id: (plan) => plan.id,
  title: (plan) => plan.title,
  urgency: (plan, ctx) => {
    const days = ctx.daysUntil(plan.date_start);
    return RESCALE(days <= 14 ? Math.max(0, 400 - days * 20) : 0, 400);
  },
  // The trip with everything joined to it (lib/context); its itinerary is one click on from there.
  href: (plan) => link(`/context?trip=${encodeURIComponent(plan.id)}`),

  whyHere: (plan) => tripGap(plan),
  startOrDueAt: (plan) => plan.date_start,
  candidateStatus: (plan) => (plan.status === "draft" ? "proposed" : "accepted"),
  // trips_plans has no data-class column, so there is nothing honest to report.
  dataClass: () => null,
  processingRoute: () => null,
};

export default trip;
