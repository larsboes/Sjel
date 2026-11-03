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
    title: "Agent Containment by Provider",
    badge: "Priority 0",
    desc: "Coding agents on untrusted providers run in an isolated sandbox with a network allow-list. Data is capped at data class c2 and pseudonymized before reaching cloud models.",
    href: "../docs/architecture/security.html",
  },
  {
    title: "iPhone Native App (Daily Driver)",
    badge: "Priority 1",
    desc: "GPUI mobile interface for daily essentials: session approvals, travel timelines, mail triage, offline document reading, and fast calendar rescheduling.",
    href: `${REPO}/blob/main/docs/plans/gpui.md#2-phone-priority-areas`,
  },
  {
    title: "One Workspace & Flat Crate Consolidation",
    badge: "Priority 2",
    desc: "Consolidating Sjel into a single root Cargo workspace with flat crates/, sweeping comments and retiring legacy web fragments.",
    href: `${REPO}/blob/main/docs/plans/restructure.md`,
  },
  {
    title: "Always-On Home Server Node",
    badge: "Hardware",
    desc: "Offloading 24/7 background feeds, price collection, travel alerts, and local c2/c3 models to a dedicated Mac mini home server.",
    href: `${REPO}/blob/main/PLAN.md#decisions`,
  },
];

const PLANNED_ITEMS: Item[] = [
  {
    title: "Standalone SQLite & Turso Replica Sync",
    badge: "Storage",
    desc: "Decoupled database per capability with offline-first local replicas on macOS and iPhone synced via Turso/libsql.",
    href: "../docs/architecture/storage.html",
  },
  {
    title: "Transit Corridor Reliability Ingest",
    badge: "Transit",
    desc: "Automated punctuality collection and delay monitoring for regional rail corridors while the Mac is closed.",
    href: `${REPO}/blob/main/docs/plans/transit.md`,
  },
  {
    title: "RoomPlan 3D LiDAR Capture",
    badge: "Scanning",
    desc: "Native iPhone LiDAR room scanning to capture room geometries and feed real-world floor plans into interior layouts.",
    href: `${REPO}/blob/main/docs/plans/scanning.md`,
  },
  {
    title: "Vault-Grounded Spaced Recall",
    badge: "Learning",
    desc: "Extracting atomic claim notes from Obsidian vaults with interactive teach and practice recall sessions on desktop and mobile.",
    href: `${REPO}/blob/main/docs/plans/learning.md`,
  },
  {
    title: "Fresh-User Notarized DMG & TestFlight Installer",
    badge: "Distribution",
    desc: "Turnkey friend trial without requiring Xcode, Rust, Bun or terminal commands.",
    href: `${REPO}/issues/235`,
  },
];

const LAUNCHED_GROUPS: MonthGroup[] = [
  {
    month: "November 2026",
    items: [
      {
        title: "Vector Technical Documentation Overhaul",
        badge: "Docs",
        desc: "Wide auto-scaling layout, task-based goal cards, sequential pagers, breadcrumbs, and zero-script static performance.",
        href: "../docs/index.html",
      },
      {
        title: "Native Bun Static Preview Server",
        badge: "v0.1.0-pre",
        desc: "Native Bun.serve static preview server with website/build --serve support and Python server retirement.",
        href: `${REPO}/commit/8f65e71c`,
      },
      {
        title: "Feed Cloud Jobs & Data-Class Lowering",
        badge: "GPUI",
        desc: "Interactive rationale prompt for lowering sensitive feed items before handing them to cloud agent workflows.",
        href: `${REPO}/commit/13521098`,
      },
      {
        title: "Inventory History & Layout Impact Preview",
        badge: "Interior",
        desc: "State transition inspection and spatial impact calculations directly inside the GPUI edit form.",
        href: `${REPO}/commit/13521098`,
      },
      {
        title: "Stay Ranking Board for Trip Plans",
        badge: "Travel",
        desc: "Hotel stay candidates stored as first-class items on trip plans with preference ranking.",
        href: `${REPO}/commit/13521098`,
      },
    ],
  },
  {
    month: "October 2026",
    items: [
      {
        title: "GPUI Native Desktop & iOS Shell",
        badge: "Core",
        desc: "Retired the Tauri app and web dashboard to standardize on a high-performance, single native Rust UI stack.",
        href: `${REPO}/commit/41d6e6a2`,
      },
      {
        title: "Low-Fare Rail Price Monitoring",
        badge: "Travel",
        desc: "Offline-first sparpreis alerts notifying once per new low fare on iPhone.",
        href: `${REPO}/blob/main/capabilities/sparpreis-watch/README.md`,
      },
      {
        title: "Robots-Compliant Wishlist Price Tracking",
        badge: "Inventory",
        desc: "Automated product price observations with budget alerts and unit price comparisons.",
        href: `${REPO}/blob/main/capabilities/wish-prices/README.md`,
      },
      {
        title: "Map Scroll & Zoom Controls",
        badge: "GPUI",
        desc: "System scroll direction alignment and Cmd+wheel zooming in native map views.",
        href: `${REPO}/commit/134d6bc9`,
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
