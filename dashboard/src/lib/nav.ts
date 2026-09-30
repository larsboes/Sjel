/**
 * An internal destination, made absolute against the configured base path.
 *
 * SvelteKit rewrites asset URLs for `paths.base` and deliberately does NOT touch an `href`
 * you wrote yourself — it cannot tell an app route from an outbound link. On a real machine
 * the base is empty and this is the identity function, which is why every raw absolute href
 * worked for a year and then sent every click in the published demo to the domain root,
 * where GitHub answers with its own 404 (#170).
 *
 * Every internal link goes through here, and `tools/dashboard-nav-links.test.ts` fails the
 * build on one that does not — the failure is invisible in the only environment anyone
 * develops in.
 *
 * The base is HANDED to this module rather than imported from `$app/paths`, which is not a
 * style choice. That alias is resolved by Vite and does not exist under plain `bun test`, so
 * importing it here put it into the import graph of every module that builds a link —
 * including `calendar/types.ts`, which an existing test imports directly and which then
 * failed to resolve at all. Exactly one module reads the alias now, and it is the root
 * layout, which cannot run outside SvelteKit anyway.
 */
let configuredBase = "";

/** Called once from the root layout, before any component renders. */
export const setBase = (value: string): void => {
  configuredBase = value;
};

export const link = (path: string): string => `${configuredBase}${path}`;

export interface NavItem {
  href: string;
  label: string;
  icon: string;
  /**
   * The capability whose absence makes this destination pointless.
   *
   * Named for the published demo (#168), which runs on a subset of capabilities and has to
   * hide what it cannot show — a nav item leading to a page of error cards teaches a visitor
   * the software is broken rather than that the demo is partial. It is a real fact about the
   * route either way: /finance without the finance capability is an empty page on a real
   * machine too, and nothing but this line says so.
   *
   * Absent on /: Home draws from several capabilities and degrades to the ones present.
   */
  capability?: string;
  /**
   * The capabilities the shell starts when this destination is opened.
   *
   * Defaults to `[capability]`. It is a separate field because the two facts differ:
   * /travel is pointless without `transit`, which is what `capability` says, but the page
   * also reads `trips` and neither would start on its own. Before this, /finance and /map
   * were the only two primary destinations that started nothing at all, so on a cold
   * machine they rendered an error card until the operator went to /capabilities.
   */
  starts?: string[];
  /**
   * This destination draws a MapLibre surface, so the shell may start fetching the library
   * before the click lands. The two map routes cost ~1 MB of already-lazy chunk plus the
   * vendored style; hovering the link is a strong enough signal to begin, and beginning is
   * what turns the first frame from a wait into a paint.
   *
   * A boolean rather than a function, because this module must stay importable under plain
   * `bun test` — see the note on `setBase` above. The root layout owns the dynamic import.
   */
  warmsMap?: true;
}

/** Daily work stays visible. Machine administration sits one level deeper. */
export const PRIMARY_NAV: NavItem[] = [
  { href: "/", label: "Home", icon: "home" },
  { href: "/calendar", label: "Calendar", icon: "calendar", capability: "calendar" },
  { href: "/feed", label: "Feed", icon: "feed", capability: "comms" },
  { href: "/travel", label: "Travel", icon: "map-pin", capability: "transit", starts: ["transit", "trips"], warmsMap: true },
  { href: "/map", label: "Map", icon: "globe", capability: "places", warmsMap: true },
  { href: "/finance", label: "Finance", icon: "database", capability: "finance" },
  // Ein Ziel in der Shell und nicht nur ein Panel: das ist der Unterschied, den PRD Q59
  // ausdruecklich nennt, und der Grund, aus dem die Capability nach core Sjel gezogen ist.
  { href: "/interior", label: "Interior", icon: "layout", capability: "interior" },
  // PRD Q117: Sjel is the system of record for people; the page edits capabilities/entities.
  { href: "/people", label: "People", icon: "users", capability: "entities" },
];

/**
 * Projects and operations are real destinations, but not part of every daily pass.
 * Capability-owned sites are discovered on /projects rather than growing the main bar.
 */
export const UTILITY_NAV: NavItem[] = [
  { href: "/projects", label: "Projects", icon: "graduation" },
  { href: "/systems", label: "Systems", icon: "server", capability: "sjel-status" },
  { href: "/capabilities", label: "Capabilities", icon: "boxes", capability: "sjel-status" },
  { href: "/backup", label: "Backup", icon: "database", capability: "sjel-status" },
  { href: "/self", label: "Self-model", icon: "compass", capability: "sjel-status" },
  { href: "/packs", label: "Packs", icon: "boxes", capability: "sjel-status" },
];

/**
 * The capabilities a pathname needs running, by longest matching prefix.
 *
 * Longest-prefix rather than first-match, and PRIMARY before UTILITY, because `/` would
 * otherwise claim every path in the app. `/` itself yields nothing: Home reads seven
 * capabilities and starts each on demand through its own registry, which is a per-kind
 * decision this table cannot make.
 */
export const capabilityForPath = (pathname: string): string[] => {
  const candidates = [...PRIMARY_NAV, ...UTILITY_NAV].filter(
    (item) => item.href !== "/" && (pathname === item.href || pathname.startsWith(`${item.href}/`)),
  );
  const best = candidates.sort((a, b) => b.href.length - a.href.length)[0];
  if (!best) return [];
  return best.starts ?? (best.capability ? [best.capability] : []);
};

/** Drop the destinations a given set of missing capabilities makes pointless. */
export const withoutCapabilities = (items: NavItem[], missing: Set<string>): NavItem[] =>
  items.filter((item) => !item.capability || !missing.has(item.capability));

export const titleCase = (s: string): string => s.charAt(0).toUpperCase() + s.slice(1);
