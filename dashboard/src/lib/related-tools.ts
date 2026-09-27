export type RelatedToolContext = "travel-planning" | "rail-search";

export interface RelatedTool {
  id: string;
  name: string;
  url: string;
  kind: string;
  relation: "Alternative" | "Inspiration";
  contexts: RelatedToolContext[];
  goodAt: string;
  boundary: string;
  action: string;
}

export const RELATED_TOOLS: RelatedTool[] = [
  {
    id: "trek",
    name: "TREK",
    url: "https://demo.liketrek.com",
    kind: "Self-hosted",
    relation: "Alternative",
    contexts: ["travel-planning"],
    goodAt: "Planning together in real time, sharing invitations, and managing budgets and packing lists.",
    boundary:
      "A polished group interface. Sjel instead connects the plan to your own events, sources, and notes.",
    action: "Open demo",
  },
  {
    id: "tripit",
    name: "TripIt",
    url: "https://www.tripit.com/web/free",
    kind: "Cloud service",
    relation: "Alternative",
    contexts: ["travel-planning"],
    goodAt: "Turning booking confirmations from email into a travel itinerary automatically.",
    boundary:
      "Less manual entry, but travel and booking data live with an external service.",
    action: "Open TripIt",
  },
  {
    id: "besser-bahn",
    name: "Besser Bahn",
    url: "https://github.com/chuk-development/Besser-Bahn",
    kind: "Android app",
    relation: "Alternative",
    contexts: ["rail-search"],
    goodAt: "Live journey guidance, connection forecasts, and split tickets on a phone.",
    boundary:
      "More specialised for a journey in progress; Sjel keeps the connection within the wider travel plan.",
    action: "View project",
  },
  {
    id: "betterbahn",
    name: "BetterBahn",
    url: "https://betterbahn.de",
    kind: "Self-hosted",
    relation: "Inspiration",
    contexts: ["rail-search"],
    goodAt: "Exploring split ticketing as a self-contained, inspectable rail workflow.",
    boundary:
      "There is currently no official hosted calculator; the project points to local use.",
    action: "Open project page",
  },
];

export function relatedToolsFor(context: RelatedToolContext): RelatedTool[] {
  return RELATED_TOOLS.filter((tool) => tool.contexts.includes(context));
}
