// tools/fetch-basemap.ts — vendor the basemap's fixed half into dashboard/static/basemap/.
//
// The problem this closes, measured 2026-09-06 from this machine with warm DNS: a cold map
// paid ~0.9 s of serialized round trips to tiles.openfreemap.org before it could ask for a
// single tile.
//
//   style JSON        43 KB   0.23 s
//   TileJSON /planet  19 KB   0.19 s   <- a second hop, because the style declares the
//                                         vector source by `url:` rather than inline `tiles:`
//   sprite JSON+PNG  147 KB   0.31 s
//   glyph PBFs       632 KB   ~0.2 s each, six requests for three fontstacks x two ranges
//
// None of that is data about the operator, none of it changes between page loads, and all of
// it is on the critical path before the first tile request leaves the browser. So it is
// fetched once, here, and served from the same origin as the shell.
//
// What deliberately stays remote: the vector tiles and the Natural Earth raster. Those are
// the part that is large (a z6 tile measured 269 KB) and the part a self-hosted PMTiles
// deployment would replace -- the seam `upstreams.toml` [openfreemap] already declares and
// `capabilities/places/ISA.md` names under "Not yet specified". This change does not take
// that decision; it removes the latency that does not depend on it.
//
// The pin, and its cost. The TileJSON's `tiles` URL carries a dated planet snapshot
// (`/planet/<YYYYMMDD_HHMMSS>_pt/{z}/{x}/{y}.pbf`), which is how OpenFreeMap rolls the
// planet forward. Inlining it is what removes the hop, and it pins the basemap to that
// snapshot until this script runs again. That is the trade taken: a month-old road network
// under a map of where money was spent is not a defect, and the refresh is one command.
// The snapshot id is written into the manifest so the age is readable rather than guessed.
//
//   tools/fetch-basemap            # refresh every vendored asset and the manifest
//   tools/fetch-basemap --check    # exit 1 if the vendored set is missing or incomplete
//
// Provenance is recorded in dashboard/static/basemap/LICENSE.md, following the precedent
// dashboard/static/fonts/OFL.txt already sets for vendored type.

import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const SJEL_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const OUT_DIR = join(SJEL_ROOT, "dashboard", "static", "basemap");

/** The documented hosted style, the one constant `dashboard/src/lib/map/style.ts` used to
 *  point the browser at directly. `upstreams.toml` [openfreemap] governs it. */
const UPSTREAM_STYLE = "https://tiles.openfreemap.org/styles/liberty";
const UPSTREAM_ORIGIN = "https://tiles.openfreemap.org";

/** Where the browser reaches the vendored copies. Same origin as the shell, so these cost a
 *  loopback read rather than a transatlantic round trip. */
const PUBLIC_BASE = "/basemap";

// Codepoint ranges vendored for every fontstack the style names.
//
// Four, not all ~300 that Noto Sans is published in. `latinizeLabels` below drops the
// `name:nonlatin` half of every label, so the map draws Latin script and only Latin script —
// which makes the vendored set a closed question rather than an open-ended one, and these four
// close it. Measured on the default view (Germany, z4.2) after the change: **zero** glyph
// requests leave this machine.
//
//   0-255      Basic Latin and Latin-1 Supplement
//   256-511    Latin Extended-A, start of Extended-B
//   512-767    the rest of Extended-B and IPA — reached by romanised names
//   7680-7935  Latin Extended Additional — Vietnamese, and Welsh/Irish diacritics
//
// A range outside the set still falls back to the upstream host through the surface module's
// transformRequest, so the failure mode of getting this wrong is a slow label, not a missing
// one.
const VENDORED_RANGES = ["0-255", "256-511", "512-767", "7680-7935"] as const;

interface Manifest {
  /** When this vendoring ran, so a reader can see the pin's age without a network call. */
  fetched_at: string;
  /** The dated planet snapshot the style's `tiles` URL was pinned to. */
  planet_snapshot: string;
  /** `<fontstack>/<range>` keys that exist under fonts/. The browser's transformRequest
   *  sends a miss upstream instead of rendering a labelless map. */
  glyphs: string[];
}

async function get(url: string): Promise<Response> {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${response.status} ${response.statusText} for ${url}`);
  return response;
}

async function getBytes(url: string): Promise<Uint8Array> {
  return new Uint8Array(await (await get(url)).arrayBuffer());
}

/**
 * Where a vendored asset goes, refusing any path that leaves `OUT_DIR`.
 *
 * Not every `relativePath` here is a literal. The glyph writes name a fontstack read
 * out of the upstream style JSON (`text-font`, whatever that document says), so a
 * fontstack called `../../../../src/routes` would have put a downloaded PBF into the
 * dashboard's source tree. `join` resolves the `..` and says nothing. CodeQL
 * js/http-to-file-access, alert 69: the file write and the path both come from the
 * network, and the path is the half that matters.
 *
 * Exported so the refusal can be tested. The containment test is on the RESOLVED
 * path, not on the string, because `fonts/x/../../..` has no `..` prefix to look for.
 */
export function vendoredTarget(relativePath: string): string {
  const target = resolve(OUT_DIR, relativePath);
  if (target !== OUT_DIR && !target.startsWith(OUT_DIR + sep)) {
    throw new Error(`refusing to write outside the vendored basemap directory: ${relativePath}`);
  }
  return target;
}

function write(relativePath: string, bytes: Uint8Array | string): void {
  const target = vendoredTarget(relativePath);
  mkdirSync(dirname(target), { recursive: true });
  writeFileSync(target, bytes);
}

/** Every distinct `text-font` stack the style's layers name, so the vendored set follows the
 *  style rather than a list that goes stale the next time upstream restyles a layer. */
function fontstacks(style: Record<string, unknown>): string[] {
  const found = new Set<string>();
  for (const layer of (style.layers ?? []) as Array<Record<string, unknown>>) {
    const layout = layer.layout as Record<string, unknown> | undefined;
    const fonts = layout?.["text-font"];
    if (Array.isArray(fonts)) for (const font of fonts) if (typeof font === "string") found.add(font);
  }
  return [...found].sort();
}

/**
 * Label in Latin script only, and stop asking for the other 290 glyph ranges.
 *
 * Liberty's `text-field` renders `name:latin` concatenated with `name:nonlatin`, so a European
 * overview asks for Greek, Cyrillic, Hebrew, Arabic, Devanagari, Thai, Georgian and more.
 * Measured on the default view (Germany, z4.2) on 2026-09-06: **29 glyph requests to the
 * upstream host across 19 ranges**, ~2 MB, every one of them on the critical path for a label.
 *
 * Vendoring those ranges would be ~4.5 MB in a public repository. Rendering them costs a
 * request per range per fontstack, forever. The third option is to decide what the map is: a
 * personal map, read by one person, in Latin script. `name:latin` already carries the
 * romanised form of every place — Athína, Moskva, Kyiv — so the label survives; only the
 * second line in the local script goes.
 *
 * Reversible in one place: delete this call and re-run, and the ranges come back through the
 * surface module's upstream fallback exactly as before.
 */
function latinizeLabels(style: Record<string, unknown>): void {
  const latinOnly = ["coalesce", ["get", "name:latin"], ["get", "name_en"], ["get", "name"]];
  for (const layer of (style.layers ?? []) as Array<Record<string, unknown>>) {
    const layout = layer.layout as Record<string, unknown> | undefined;
    const field = layout?.["text-field"];
    // Only the two-script form. A layer labelling something else -- a road `ref`, say -- is
    // left exactly as upstream wrote it.
    if (Array.isArray(field) && JSON.stringify(field).includes("name:nonlatin")) {
      layout!["text-field"] = latinOnly;
    }
  }
}

async function refresh(): Promise<void> {
  const style = (await (await get(UPSTREAM_STYLE)).json()) as Record<string, unknown>;
  const sources = style.sources as Record<string, Record<string, unknown>>;

  // Inline the vector source's TileJSON. This is the hop that disappears.
  const vector = Object.entries(sources).find(([, source]) => source.type === "vector" && source.url);
  if (!vector) throw new Error("the upstream style declares no vector source with a `url`");
  const [vectorName, vectorSource] = vector;
  const tileJson = (await (await get(vectorSource.url as string)).json()) as Record<string, unknown>;
  const tiles = tileJson.tiles as string[];
  delete vectorSource.url;
  Object.assign(vectorSource, {
    tiles,
    minzoom: tileJson.minzoom,
    maxzoom: tileJson.maxzoom,
    bounds: tileJson.bounds,
    // Attribution is a licence obligation, not decoration: inlining the TileJSON is what
    // would otherwise have dropped it, because that is the document it was carried in.
    attribution: tileJson.attribution,
  });

  const snapshot = /\/planet\/([^/]+)\//.exec(tiles[0] ?? "")?.[1] ?? "unknown";

  latinizeLabels(style);

  const spriteBase = style.sprite as string;
  const glyphTemplate = style.glyphs as string;
  style.sprite = `${PUBLIC_BASE}/sprite/ofm`;
  style.glyphs = `${PUBLIC_BASE}/fonts/{fontstack}/{range}.pbf`;

  for (const suffix of ["", "@2x"] as const) {
    write(`sprite/ofm${suffix}.json`, await getBytes(`${spriteBase}${suffix}.json`));
    write(`sprite/ofm${suffix}.png`, await getBytes(`${spriteBase}${suffix}.png`));
  }

  const glyphs: string[] = [];
  for (const stack of fontstacks(style)) {
    for (const range of VENDORED_RANGES) {
      const url = glyphTemplate
        .replace("{fontstack}", encodeURIComponent(stack))
        .replace("{range}", range);
      write(`fonts/${stack}/${range}.pbf`, await getBytes(url));
      glyphs.push(`${stack}/${range}`);
    }
  }

  const manifest: Manifest = {
    fetched_at: new Date().toISOString().slice(0, 10),
    planet_snapshot: snapshot,
    glyphs,
  };
  write("manifest.json", `${JSON.stringify(manifest, null, 2)}\n`);
  write("style.json", `${JSON.stringify(style, null, 2)}\n`);

  const bytes = glyphs.length;
  console.log(
    `basemap vendored: ${bytes} glyph ranges across ${fontstacks(style).length} fontstacks, ` +
      `sprite at 1x and 2x, style pinned to planet snapshot ${snapshot}.`,
  );
  console.log(`upstream fallback for any range outside ${VENDORED_RANGES.join(", ")}: ${UPSTREAM_ORIGIN}`);
}

/** Does the vendored set on disk match its own manifest?
 *
 *  Not a repository gate and deliberately not wired into `tools/doctor`. These files are
 *  committed, so a checkout always has them and the check could never fire there; doctor is
 *  about the machine, not the tree. `tools/dashboard-basemap.test.ts` is the real gate and it
 *  runs in CI. This flag is for the one moment neither covers -- immediately after a refresh,
 *  before the result is committed, when a half-written fetch is actually possible. */
function check(): number {
  const manifestPath = join(OUT_DIR, "manifest.json");
  if (!existsSync(manifestPath)) {
    console.error(`basemap: no manifest at ${manifestPath}. Run tools/fetch-basemap.`);
    return 1;
  }
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8")) as Manifest;
  const missing = [
    "style.json",
    "sprite/ofm.json",
    "sprite/ofm.png",
    "sprite/ofm@2x.json",
    "sprite/ofm@2x.png",
    ...manifest.glyphs.map((key) => `fonts/${key}.pbf`),
  ].filter((relativePath) => !existsSync(join(OUT_DIR, relativePath)));

  if (missing.length > 0) {
    console.error(`basemap: ${missing.length} vendored file(s) missing:`);
    for (const relativePath of missing) console.error(`  ${relativePath}`);
    return 1;
  }
  console.log(
    `basemap: ${manifest.glyphs.length} glyph ranges, sprite and style present; ` +
      `planet snapshot ${manifest.planet_snapshot}, vendored ${manifest.fetched_at}.`,
  );
  return 0;
}

// Guarded, so tools/dashboard-basemap.test.ts can import `vendoredTarget` without a
// bare import deleting fonts/ and re-fetching the planet. The precedent was
// tools/doctor.ts, whose test imported it the same way until the doctor moved to Rust on
// 2026-10-02.
if (import.meta.main) {
  if (process.argv.includes("--check")) {
    process.exit(check());
  } else {
    // Only the generated subtrees, so a fontstack upstream stops naming leaves rather than
    // lingering. LICENSE.md lives in the same directory and is written by hand, not here.
    for (const generated of ["fonts", "sprite"]) {
      const path = join(OUT_DIR, generated);
      if (existsSync(path)) rmSync(path, { recursive: true });
    }
    await refresh();
  }
}
