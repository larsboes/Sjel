// website/scripts/links.ts — every internal link in the built site resolves: the file exists,
// and a #fragment names an id on that page. External links are not fetched.
//
//   bun website/scripts/links.ts <site>     exit 1 and list each broken link
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, normalize } from "node:path";

export function brokenLinks(site: string): string[] {
  const pages = readdirSync(site, { recursive: true }).map(String).filter((f) => f.endsWith(".html"));
  const ids = new Map<string, Set<string>>();
  const idsOf = (file: string) => {
    if (!ids.has(file)) {
      const html = readFileSync(file, "utf8");
      ids.set(file, new Set([...html.matchAll(/\sid="([^"]+)"/g)].map((m) => m[1])));
    }
    return ids.get(file)!;
  };
  const broken: string[] = [];
  for (const rel of pages) {
    const file = join(site, rel);
    const html = readFileSync(file, "utf8");
    for (const m of html.matchAll(/\s(?:href|src)="([^"]+)"/g)) {
      const url = m[1];
      if (/^([a-z][a-z0-9+.-]*:|\/\/)/i.test(url)) continue;
      const [path, frag] = url.split("#", 2) as [string, string | undefined];
      let target = path ? normalize(join(dirname(file), decodeURI(path))) : file;
      if (existsSync(target) && statSync(target).isDirectory()) target = join(target, "index.html");
      if (!existsSync(target)) {
        broken.push(`${rel}: ${url} (no such file)`);
      } else if (frag && target.endsWith(".html") && !idsOf(target).has(frag)) {
        broken.push(`${rel}: ${url} (no #${frag} on that page)`);
      }
    }
  }
  return broken;
}

if (import.meta.main) {
  const site = process.argv[2] ?? "site";
  const broken = brokenLinks(site);
  for (const b of broken) console.error(`broken link: ${b}`);
  if (broken.length > 0) {
    console.error(`links: ${broken.length} broken link(s) in ${site}`);
    process.exit(1);
  }
  console.log(`links: every internal link in ${site} resolves`);
}
