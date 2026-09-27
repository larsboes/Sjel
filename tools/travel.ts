#!/usr/bin/env bun
/**
 * The travel composition, without a browser.
 *
 * The logic that makes the Travel workspace useful -- matching a scouting
 * opportunity to an existing plan inside 75 km, then asking calendar whether the
 * dates are feasible, then ranking the six resulting states -- is 253 lines of
 * pure TypeScript in `dashboard/src/lib/travel/travel-candidates.ts`. No
 * capability computes any of it, and `svelte.config.js` is adapter-static with a
 * file-copy deployment, so there is no SvelteKit server route to call either.
 * The composition was reachable only by opening the page.
 *
 * This adds a second caller to the same functions. It deliberately does not copy
 * them: a second home for the 75 km rule is how the two answers start
 * disagreeing, and the whole point is that the CLI and the page agree.
 *
 * Stepping stone, on purpose. The decision is to move this composition into a
 * capability so an agent can reach it over HTTP alone, which reopens what
 * `capabilities/trips/README.md` currently argues about the dashboard being the
 * composition edge. Until that is settled, this is the cheap version that moves
 * no state and adds no endpoint -- and it is the empirical test of whether the
 * capability-side version is needed at all.
 */

import { execFileSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import {
  assessTravelCandidates,
  calendarCandidatesFor,
  DESTINATION_MATCH_RADIUS_KM,
  type TravelCandidateAssessment,
} from "../dashboard/src/lib/travel/travel-candidates.ts";

const SJEL_ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

interface RegistryEntry {
  name: string;
  port: string;
}

/** Ports come from the manifest registry, never from a constant here. */
function baseUrl(capability: string): string {
  const entries = JSON.parse(
    execFileSync(join(SJEL_ROOT, "tools/capability.sh"), ["registry"], { encoding: "utf8" }),
  ) as RegistryEntry[];
  const entry = entries.find((row) => row.name === capability && row.port);
  if (!entry) throw new Error(`capability '${capability}' has no registered HTTP surface`);
  return `http://127.0.0.1:${entry.port}`;
}

/**
 * A capability that is down is not the same as one with nothing to say. Every
 * caller here decides for itself which of those it can tolerate, because the
 * assessment has a distinct state for an absent calendar and none for an absent
 * scouting.
 */
async function getJson(url: string): Promise<unknown> {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`GET ${url} answered ${response.status}`);
  return response.json();
}

async function postJson(url: string, body: unknown): Promise<unknown> {
  const response = await fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!response.ok) throw new Error(`POST ${url} answered ${response.status}`);
  return response.json();
}

function today(): string {
  // Local date, matching what the page passes: a UTC date would move the
  // day boundary for anyone east of Greenwich, and this is a Berlin machine.
  const now = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

async function candidates(asJson: boolean): Promise<number> {
  const day = today();

  // Same two reads the page does, in the same order. `travel_candidate` is a
  // field on an opportunity's event_route, not a filter the endpoint takes:
  // the narrowing happens inside assessTravelCandidates, and doing it here
  // instead would be the second copy this tool exists to avoid.
  const opportunities = (
    (await getJson(`${baseUrl("scouting")}/opportunities`)) as {
      opportunities: Parameters<typeof assessTravelCandidates>[0];
    }
  ).opportunities;

  const plans = (await getJson(`${baseUrl("trips")}/api/plans`)) as Parameters<
    typeof assessTravelCandidates
  >[1];

  // Calendar is optional by design. `assessTravelCandidates` has a
  // `calendar_unavailable` state precisely so an unreachable calendar degrades
  // the answer instead of failing it, and reproducing that here rather than
  // throwing is what makes the CLI's output comparable to the page's.
  const wanted = calendarCandidatesFor(opportunities, plans, day);
  const verdicts = new Map();
  let calendarAvailable = false;
  if (wanted.length > 0) {
    try {
      const answer = (await postJson(`${baseUrl("calendar")}/api/verdicts`, {
        candidates: wanted,
      })) as { verdicts?: Array<{ id: string }> };
      for (const verdict of answer.verdicts ?? []) verdicts.set(verdict.id, verdict);
      calendarAvailable = true;
    } catch (error) {
      process.stderr.write(`calendar unavailable: ${(error as Error).message}\n`);
    }
  } else {
    calendarAvailable = true;
  }

  const assessed = assessTravelCandidates(opportunities, plans, verdicts, day, calendarAvailable);

  if (asJson) {
    process.stdout.write(`${JSON.stringify(assessed, null, 2)}\n`);
    return 0;
  }

  if (assessed.length === 0) {
    process.stdout.write("No travel candidates.\n");
    return 0;
  }
  process.stdout.write(
    `${assessed.length} travel candidate(s), destination match within ${DESTINATION_MATCH_RADIUS_KM} km:\n\n`,
  );
  for (const item of assessed as TravelCandidateAssessment[]) {
    const when = item.opportunity.starts_at?.slice(0, 10) ?? "undated";
    process.stdout.write(`${item.state.padEnd(20)} ${when}  ${item.opportunity.title}\n`);
    process.stdout.write(`${" ".repeat(20)} ${item.reason}\n\n`);
  }
  return 0;
}

function usage(): void {
  process.stdout.write(
    [
      "Usage: bun tools/travel.ts <command>",
      "",
      "  candidates [--json]   Scouting travel candidates, matched against trip plans",
      "                        and checked for calendar feasibility. The same",
      "                        assessment the Travel page renders.",
      "",
    ].join("\n"),
  );
}

const [command, ...rest] = process.argv.slice(2);
switch (command) {
  case "candidates":
    process.exit(await candidates(rest.includes("--json")));
    break;
  case undefined:
  case "help":
  case "--help":
  case "-h":
    usage();
    process.exit(0);
    break;
  default:
    process.stderr.write(`travel: unknown command '${command}'\n`);
    usage();
    process.exit(1);
}
