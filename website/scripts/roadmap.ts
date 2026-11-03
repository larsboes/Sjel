// website/scripts/roadmap.ts — the public Sjel roadmap, styled after Obsidian's roadmap
// with Active, Planned, and Launched milestone timelines.
//
//   bun website/scripts/roadmap.ts --out <site>     write <site>/roadmap/index.html
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { esc, page, REPO } from "./style.ts";

const ROOT = join(import.meta.dir, "../..");

interface Item {
  title: string;
  badge: string;
  desc: string;
  href: string;
}

interface MonthGroup {
  month: string;
  items: Item[];
}

const ACTIVE_ITEMS: Item[] = [
  {
    title: "GPUI Desktop & Mobile Port (Ongoing)",
    badge: "Core UI",
    desc: "Unifying all macOS and iOS surfaces in Zed's native GPUI Rust framework. Delivering sub-10ms rendering, offline trip itineraries, calendar rescheduling, and session approvals.",
    href: `${REPO}/blob/main/docs/plans/gpui.md`,
  },
  {
    title: "Agent Containment & Privacy Gates",
    badge: "Security",
    desc: "Isolating coding agents by provider trust. Sandboxing untrusted providers, capping data access, and automatically pseudonymizing sensitive facts up to data class c2.",
    href: "../docs/architecture/security.html",
  },
  {
    title: "Flat Workspace Consolidation",
    badge: "Architecture",
    desc: "Restructuring Sjel into a lean, single Cargo workspace with flat crates/, eliminating lingering dashboard dependencies and trimming codebase footprint.",
    href: `${REPO}/blob/main/docs/plans/restructure.md`,
  },
];

const PLANNED_ITEMS: Item[] = [
  {
    title: "Offline-First Replica Sync (Turso)",
    badge: "Storage",
    desc: "Decoupled SQLite database per capability with offline-first local replicas on macOS and iPhone synced via Turso/libsql.",
    href: "../docs/architecture/storage.html",
  },
  {
    title: "Always-On Home Server Node",
    badge: "Hardware",
    desc: "Offloading continuous background ingestion, travel alerts, and c2/c3 local model inference to a dedicated Mac mini home server.",
    href: `${REPO}/blob/main/PLAN.md#decisions`,
  },
  {
    title: "Turnkey Mac DMG & TestFlight Distribution",
    badge: "Distribution",
    desc: "Notarized macOS installer and TestFlight distribution enabling household members and friends to run Sjel without dev toolchains (#235).",
    href: `${REPO}/issues/235`,
  },
  {
    title: "Transit Corridor Reliability Ingest",
    badge: "Travel",
    desc: "Automated punctuality collection and delay monitoring for regional rail corridors while the Mac is closed.",
    href: `${REPO}/blob/main/docs/plans/transit.md`,
  },
];

const LAUNCHED_GROUPS: MonthGroup[] = [
  {
    month: "Initial Milestones",
    items: [
      {
        title: "Single Native GPUI Foundation",
        badge: "Core UI",
        desc: "Consolidated desktop and mobile clients onto Zed's GPUI framework, fully retiring the legacy Tauri app and web dashboard.",
        href: `${REPO}/commit/41d6e6a2`,
      },
      {
        title: "Autonomous Rail Fare Tracker",
        badge: "Travel",
        desc: "Background sparpreis rail price monitoring with local notifications on iPhone whenever a new lowest fare is observed.",
        href: `${REPO}/blob/main/capabilities/sparpreis-watch/README.md`,
      },
      {
        title: "Targeted Wishlist Price Comparisons",
        badge: "Inventory",
        desc: "Robots-compliant daily product price tracking, need-based candidate comparisons, and automated alert thresholds.",
        href: `${REPO}/blob/main/capabilities/wish-prices/README.md`,
      },
      {
        title: "Deterministic Relational Storage",
        badge: "Storage",
        desc: "Unified SQLite storage layer with WAL mode, immediate write locking, and isolated backup/restore verification.",
        href: "../docs/architecture/storage.html",
      },
    ],
  },
];

function renderCard(item: Item): string {
  const isExt = item.href.startsWith("http");
  return `
    <a class="roadmap-card" href="${esc(item.href)}"${isExt ? ' target="_blank" rel="noopener"' : ""}>
      <div class="roadmap-card-head">
        <span class="roadmap-card-title">${esc(item.title)}${isExt ? ' <span class="ext" aria-hidden="true">↗</span>' : ""}</span>
        <span class="roadmap-badge">${esc(item.badge)}</span>
      </div>
      <p class="roadmap-card-desc">${esc(item.desc)}</p>
    </a>`;
}

export function renderRoadmap(): string {
  const activeHtml = `
    <section class="roadmap-section">
      <div class="roadmap-status active">
        <h2><span aria-hidden="true">●</span> Active</h2>
      </div>
      <div class="roadmap-cards">
        ${ACTIVE_ITEMS.map(renderCard).join("")}
      </div>
    </section>`;

  const plannedHtml = `
    <section class="roadmap-section">
      <div class="roadmap-status planned">
        <h2><span aria-hidden="true">⌛</span> Planned</h2>
      </div>
      <div class="roadmap-cards">
        ${PLANNED_ITEMS.map(renderCard).join("")}
      </div>
    </section>`;

  const launchedHtml = `
    <section class="roadmap-section">
      <div class="roadmap-status launched">
        <h2><span aria-hidden="true">🚀</span> Launched</h2>
      </div>
      <div class="roadmap-timeline">
        ${LAUNCHED_GROUPS.map(
          (grp) => `
          <div class="timeline-group">
            <div class="timeline-date">${esc(grp.month)}</div>
            <div class="roadmap-cards">
              ${grp.items.map(renderCard).join("")}
            </div>
          </div>`,
        ).join("")}
      </div>
    </section>`;

  return `
    <div class="roadmap-wrap">
      <div class="roadmap-hero">
        <nav class="crumbs" aria-label="Breadcrumb">
          <a href="../index.html">Sjel</a><span class="sep">/</span><span class="current">Roadmap</span>
        </nav>
        <h1>Roadmap</h1>
        <p>We're building the personal sovereign system for local memory, feeds, and client apps. Track active development, planned priorities, and shipped milestones below.</p>
        <div class="subtext">For concrete work packages and architectural decisions, read the <a href="${REPO}/blob/main/PLAN.md">Sjel Plan</a>. Issues and friend trials are tracked on <a href="${REPO}/issues">GitHub</a>.</div>
      </div>
      ${activeHtml}
      ${plannedHtml}
      ${launchedHtml}
    </div>`;
}

export function build(out: string): void {
  const html = page({
    title: "Roadmap — Sjel",
    description: "Sjel roadmap: active development, planned work, and launched milestones.",
    root: "../",
    current: "roadmap",
    body: renderRoadmap(),
    footer: `<p>Tracked in <code>PLAN.md</code> and <code>docs/plans/</code> · <a href="${REPO}/blob/main/PLAN.md">View Sjel Plan</a></p>`,
  });
  const file = join(out, "roadmap/index.html");
  mkdirSync(dirname(file), { recursive: true });
  writeFileSync(file, html);
}

if (import.meta.main) {
  const args = process.argv.slice(2);
  const outIdx = args.indexOf("--out");
  const out = outIdx >= 0 ? args[outIdx + 1] : join(ROOT, "site");
  build(out);
  console.log(`roadmap: wrote ${join(out, "roadmap/index.html")}`);
}
