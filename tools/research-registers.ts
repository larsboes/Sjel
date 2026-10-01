#!/usr/bin/env bun
// The two project registers, as the dashboard's /research/projects page reads them.
//
//   bun tools/research-registers.ts     print {upstreams, systems} as JSON on stdout
//   bun tools/research-registers.ts watch --url <url> --summary <text> [--name <id>]
//                                       append a `watch` row to upstreams.toml, print it as JSON
//
// Bun parses the TOML because Vite runs under Node, which has no TOML parser, and adding one
// to the dashboard would be a dependency for two files. dashboard/vite/research.ts runs this
// at build time. `why` is not exported for upstreams: its median is about 590 characters, it
// is the audit, and `summary` is the line written for a reader (upstreams.toml header).

import { appendFileSync, readFileSync } from "node:fs";
import { join } from "node:path";

import { SJEL_ROOT } from "./lib/demo-endpoints.ts";

export interface UpstreamRow {
  name: string;
  url: string;
  verdict: string;
  license: string;
  summary: string;
}

export interface SystemRow {
  name: string;
  /** Null where the real address lives in the private overlay. */
  url: string | null;
  kind: string;
  local: boolean;
  why: string;
}

type Table = Record<string, Record<string, unknown>>;

const text = (value: unknown): string => (typeof value === "string" ? value : "");

export function upstreamRows(table: Table): UpstreamRow[] {
  return Object.entries(table).map(([name, row]) => ({
    name,
    url: text(row.url),
    verdict: text(row.verdict),
    license: text(row.license),
    summary: text(row.summary),
  }));
}

export function systemRows(table: Table): SystemRow[] {
  return Object.entries(table).map(([name, row]) => {
    const url = text(row.url);
    return {
      name,
      // systems.toml: private repos name "overlay:systems.local.toml", and the core repo itself
      // says "local". Neither is an address a visitor can open.
      url: /^https?:\/\//.test(url) ? url : null,
      kind: text(row.kind),
      local: row.local === "yes",
      why: text(row.why),
    };
  });
}

export function readRegisters(root = SJEL_ROOT): { upstreams: UpstreamRow[]; systems: SystemRow[] } {
  const parse = (file: string) => Bun.TOML.parse(readFileSync(join(root, file), "utf8")) as Table;
  return { upstreams: upstreamRows(parse("upstreams.toml")), systems: systemRows(parse("systems.toml")) };
}

export interface WatchRequest {
  url: string;
  summary: string;
  name?: string;
}

export type LicenseLookup = (url: string) => Promise<string>;

const NAME = /^[a-z0-9][a-z0-9-]*$/;
const SUMMARY_MAX = 110;

/** The row name a URL suggests: the last path segment of a repository or model id. */
export function nameFromUrl(url: string): string {
  const segments = new URL(url).pathname.split("/").filter(Boolean);
  const last = segments[1] ?? segments[0] ?? new URL(url).hostname.split(".")[0];
  return last.toLowerCase().replace(/[^a-z0-9-]+/g, "-").replace(/^-+|-+$/g, "");
}

/** The SPDX id GitHub or Hugging Face reports for a URL, or "" when neither answers. */
export const lookupLicense: LicenseLookup = async (url) => {
  const { hostname, pathname } = new URL(url);
  const [owner, repo] = pathname.split("/").filter(Boolean);
  if (!owner || !repo) return "";
  const signal = AbortSignal.timeout(8000);
  try {
    if (hostname === "github.com") {
      const r = await fetch(`https://api.github.com/repos/${owner}/${repo}`, { signal });
      const body = (await r.json()) as { license?: { spdx_id?: string } };
      const id = body.license?.spdx_id ?? "";
      return id === "NOASSERTION" ? "" : id;
    }
    if (hostname === "huggingface.co") {
      const r = await fetch(`https://huggingface.co/api/models/${owner}/${repo}`, { signal });
      const body = (await r.json()) as { cardData?: { license?: string } };
      return body.cardData?.license ?? "";
    }
  } catch {
    // An unreachable API leaves the license empty. The audit that replaces `watch` fills it.
  }
  return "";
};

/**
 * Append one `watch` row. Refuses a duplicate name, a URL that is not https, an empty or
 * long summary, and a name TOML would need to quote. A watch row grants nothing
 * (upstreams.toml header), so this writes no verdict a reader could mistake for an audit.
 */
export async function addWatch(
  request: WatchRequest,
  opts: { root?: string; license?: LicenseLookup; today?: string } = {},
): Promise<UpstreamRow> {
  const root = opts.root ?? SJEL_ROOT;
  let parsed: URL;
  try {
    parsed = new URL(request.url);
  } catch {
    throw new Error(`The URL is not valid: ${request.url}`);
  }
  if (parsed.protocol !== "https:") throw new Error("The URL must start with https://.");
  const summary = request.summary.trim();
  if (!summary) throw new Error("The summary is empty. Write one line.");
  if (summary.length > SUMMARY_MAX) {
    throw new Error(`The summary has ${summary.length} characters. The limit is ${SUMMARY_MAX}.`);
  }
  const name = (request.name?.trim() || nameFromUrl(request.url)).toLowerCase();
  if (!NAME.test(name)) throw new Error(`The name "${name}" is not valid. Use a-z, 0-9 and "-".`);
  const file = join(root, "upstreams.toml");
  const table = Bun.TOML.parse(readFileSync(file, "utf8")) as Table;
  if (name in table) throw new Error(`The name "${name}" is already in upstreams.toml.`);
  const license = await (opts.license ?? lookupLicense)(request.url);
  const today = opts.today ?? new Date().toISOString().slice(0, 10);
  const row: UpstreamRow = { name, url: request.url, verdict: "watch", license, summary };
  const why = `noted ${today} from the dashboard's Projects page. Not audited.`;
  const text = readFileSync(file, "utf8");
  appendFileSync(
    file,
    `${text.endsWith("\n") ? "" : "\n"}\n[${name}]\n` +
      `url = ${JSON.stringify(row.url)}\n` +
      `verdict = "watch"\n` +
      `license = ${JSON.stringify(license)}\n` +
      `summary = ${JSON.stringify(summary)}\n` +
      `why = ${JSON.stringify(why)}\n`,
  );
  return row;
}

function flag(args: string[], name: string): string | undefined {
  const i = args.indexOf(`--${name}`);
  return i >= 0 ? args[i + 1] : undefined;
}

if (import.meta.main) {
  const args = process.argv.slice(2);
  if (args[0] === "watch") {
    try {
      const row = await addWatch({
        url: flag(args, "url") ?? "",
        summary: flag(args, "summary") ?? "",
        name: flag(args, "name"),
      });
      console.log(JSON.stringify(row));
    } catch (e) {
      console.error(e instanceof Error ? e.message : String(e));
      process.exit(1);
    }
  } else {
    console.log(JSON.stringify(readRegisters()));
  }
}
