// website/scripts/routes.ts — each capability's declared routes, read from its source.
//
// A capability declares its HTTP surface once, in a `const ROUTES: &[route_manifest::Route]`
// block, and serves it at `GET /routes`. This reads the same block as text, the way
// tools/check-route-schemas.sh does, so the reference pages need no running server.
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

export interface DeclaredRoute {
  method: string;
  path: string;
  summary: string;
  /** The route declares a request body schema (`request_schema: Some(…)`). */
  schema: boolean;
}

const METHOD = /^(GET|POST|PUT|PATCH|DELETE)$/;

/** The string literals of a Rust source fragment, in order, with escapes resolved. */
export function stringLiterals(src: string): Array<{ value: string; end: number }> {
  const out: Array<{ value: string; end: number }> = [];
  for (let i = 0; i < src.length; i++) {
    if (src[i] === "/" && src[i + 1] === "/") {
      i = src.indexOf("\n", i);
      if (i < 0) break;
      continue;
    }
    if (src[i] !== '"') continue;
    let value = "";
    let j = i + 1;
    for (; j < src.length && src[j] !== '"'; j++) {
      if (src[j] !== "\\") {
        value += src[j];
        continue;
      }
      const next = src[++j];
      if (next === "\n") {
        // A line continuation drops the newline and the next line's leading whitespace.
        while (j + 1 < src.length && /\s/.test(src[j + 1])) j++;
      } else {
        value += next === "n" ? "\n" : next === "t" ? "\t" : next;
      }
    }
    out.push({ value, end: j });
    i = j;
  }
  return out;
}

/** The routes in one `const ROUTES` block: a method literal, then the path, then the summary. */
export function parseRoutes(block: string): DeclaredRoute[] {
  const lits = stringLiterals(block);
  const routes: DeclaredRoute[] = [];
  for (let k = 0; k + 2 < lits.length; k++) {
    if (!METHOD.test(lits[k].value) || !lits[k + 1].value.startsWith("/")) continue;
    const nextMethod = lits.slice(k + 3).find((l) => METHOD.test(l.value));
    const tail = block.slice(lits[k + 2].end, nextMethod ? nextMethod.end : block.length);
    routes.push({
      method: lits[k].value,
      path: lits[k + 1].value,
      summary: lits[k + 2].value,
      schema: /request_schema:\s*Some/.test(tail),
    });
    k += 2;
  }
  return routes;
}

/** The `const ROUTES` block of a source file, or null. */
export function routesBlock(src: string): string | null {
  const start = src.search(/const ROUTES\b/);
  if (start < 0) return null;
  const end = src.indexOf("\n];", start);
  return end < 0 ? null : src.slice(start, end);
}

/** Every route a capability declares, from the first `const ROUTES` block under its `src/`. */
export function capabilityRoutes(root: string, name: string): DeclaredRoute[] {
  const src = join(root, "capabilities", name, "src");
  if (!existsSync(src)) return [];
  const files = readdirSync(src, { recursive: true })
    .map(String)
    .filter((f) => f.endsWith(".rs"))
    .sort((a, b) => Number(!/^(main|server)\.rs$/.test(a)) - Number(!/^(main|server)\.rs$/.test(b)));
  for (const file of files) {
    const block = routesBlock(readFileSync(join(src, file), "utf8"));
    if (block) return parseRoutes(block);
  }
  return [];
}
