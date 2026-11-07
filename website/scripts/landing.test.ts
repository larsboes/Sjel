import { describe, expect, test } from "bun:test";
import { renderLanding } from "./landing.ts";
import { THEME_INIT_SCRIPT } from "./style.ts";

function contrastRatio(foreground: string, background: string): number {
  const luminance = (hex: string) => {
    const channels = hex.match(/[0-9a-f]{2}/gi)!.map((channel) => parseInt(channel, 16) / 255);
    const linear = channels.map((value) => value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4);
    return 0.2126 * linear[0]! + 0.7152 * linear[1]! + 0.0722 * linear[2]!;
  };
  const first = luminance(foreground);
  const second = luminance(background);
  return (Math.max(first, second) + 0.05) / (Math.min(first, second) + 0.05);
}

describe("landing page", () => {
  test("every capability is listed and links to its reference page, an unmapped one under Platform", () => {
    const html = renderLanding({
      units: [
        { name: "trips", kind: "capability" },
        { name: "brand-new", kind: "capability" },
        { name: "agent", kind: "lib" },
      ],
      upstreams: [],
    });
    expect(html).toContain('href="reference/trips.html"');
    expect(html).toContain('href="reference/brand-new.html"');
    const platform = html.slice(html.indexOf("<h3>Platform"));
    expect(platform).toContain("brand-new");
    expect(html).toContain("<h2>2 capabilities, enabled per machine.</h2>");
    expect(html).not.toContain("<h3>Assistant");
  });

  test("describes the gate accurately and includes reduced-motion support", () => {
    const html = renderLanding({ units: [], upstreams: [] });
    expect(html).toContain("Your life data, at home.");
    expect(html).toContain('"admitted": false');
    expect(html).toContain("c2 has no cloud representation");
    expect(html).not.toContain("Elena Rostova");
    expect(html).not.toContain("Type-Level Privacy");
    expect(html).toContain("prefers-reduced-motion: reduce");
  });

  test("includes accessible theme control, anti-flash bootstrap, and signature turquoise accents", () => {
    const html = renderLanding({ units: [], upstreams: [] });
    expect(html).toContain('class="theme-ctrl"');
    expect(html).toContain('data-theme-val="system"');
    expect(html).toContain('data-theme-val="light"');
    expect(html).toContain('data-theme-val="dark"');
    expect(html).toContain('aria-label="System theme"');
    expect(html).toContain('aria-label="Light theme"');
    expect(html).toContain('aria-label="Dark theme"');
    expect(html).toContain('localStorage.getItem("theme")');
    // Signature Sjel turquoise brand tokens & accents
    expect(html).toContain("--teal:");
    expect(html.toLowerCase()).toContain("#16748d");
    expect(html.toLowerCase()).toContain("#6cc3d8");
    // The character shortcut is active only while focus is inside its visible control.
    expect(html).toContain('class="theme-kbd"');
    expect(html).toContain("Focus this control, then press T to cycle theme");
    expect(html).toContain('group.contains(el)');
    expect(html).toContain('var next = order[(order.indexOf(current) + 1) % order.length]');
    expect(html).toContain('current = val;');
    expect(html).not.toContain('localStorage.getItem("theme");\n      if (s === "dark" || s === "light") now = s;');
  });

  test("cycles themes in memory when localStorage throws, only while a theme button is focused", () => {
    const handlers = new Map<string, (event: any) => void>();
    const attributes = new Map<string, string>();
    const makeButton = (value: string) => ({
      getAttribute: (name: string) => name === "data-theme-val" ? value : attributes.get(`${value}:${name}`) ?? null,
      setAttribute: (name: string, attribute: string) => attributes.set(`${value}:${name}`, attribute),
      addEventListener: (name: string, handler: (event: any) => void) => handlers.set(`${value}:${name}`, handler),
      classList: { add() {}, remove() {} },
    });
    const buttons = [makeButton("system"), makeButton("light"), makeButton("dark")];
    const activeElement = {};
    let focused = false;
    let keydown: ((event: any) => void) | undefined;
    const root = { setAttribute() {}, removeAttribute() {} };
    const doc = {
      querySelectorAll: () => buttons,
      querySelector: (selector: string) => selector === ".theme-ctrl"
        ? { contains: () => focused }
        : { textContent: "", classList: { add() {}, remove() {} } },
      documentElement: root,
      activeElement,
    };
    const win = { addEventListener: (_name: string, handler: (event: any) => void) => { keydown = handler; } };
    const blockedStorage = {
      getItem() { throw new Error("blocked"); },
      setItem() { throw new Error("blocked"); },
      removeItem() { throw new Error("blocked"); },
    };
    const source = THEME_INIT_SCRIPT.replace(/^<script>/, "").replace(/<\/script>$/, "");
    new Function("window", "document", "localStorage", "setTimeout", "clearTimeout", source)(
      win, doc, blockedStorage, () => 1, () => {},
    );
    const pressT = () => keydown?.({ key: "t", metaKey: false, ctrlKey: false, altKey: false, preventDefault() {} });
    pressT();
    expect(attributes.get("system:aria-pressed")).toBe("true");
    focused = true;
    pressT();
    expect(attributes.get("light:aria-pressed")).toBe("true");
    pressT();
    expect(attributes.get("dark:aria-pressed")).toBe("true");
  });

  test("includes ASCII architecture diagram and blueprint corner tick accents", () => {
    const html = renderLanding({ units: [], upstreams: [] });
    // Architecture diagram switcher and ASCII rendering
    expect(html).toContain('class="arch-frame blueprint"');
    expect(html).toContain('data-arch="vector"');
    expect(html).toContain('data-arch="ascii"');
    expect(html).toContain('role="group" aria-label="Diagram format"');
    expect(html).toContain('aria-controls="arch-vector"');
    expect(html).toContain('aria-controls="arch-ascii"');
    expect(html).toContain('aria-pressed="true"');
    expect(html).toContain('id="arch-vector" role="region"');
    expect(html).toContain('id="arch-ascii" role="region"');
    expect(html).toContain('aria-label="ASCII architecture diagram" hidden');
    expect(html).toContain('vectorView.hidden = showAscii;');
    expect(html).toContain('asciiView.hidden = !showAscii;');
    expect(html).not.toContain('role="tab"');
    expect(html).not.toContain('aria-selected');
    expect(html).toContain('class="arch-canvas arch-ascii"');
    expect(html).toContain("┌─────────────────┐");
    expect(html).toContain("iPhone app");
    expect(html).toContain("Node (authoritative writes)");
    // Blueprint tick marks
    expect(html).toContain('class="classes blueprint"');
    expect(html).toContain('class="cls-tick"');
  });

  test("resolves component foreground/background contrast above WCAG AA in both themes", () => {
    const pairs = [
      // Terminal syntax, active architecture/theme controls, on the dark terminal surface.
      ["#e2eeea", "#0b1517"], ["#aabbbc", "#0b1517"], ["#6cc3d8", "#0b1517"],
      ["#e6eded", "#070c0d"], ["#aabbbc", "#070c0d"], ["#6cc3d8", "#070c0d"],
      // Privacy class labels use the body foreground and code surface in each theme.
      ["#132022", "#edf3f4"], ["#e6eded", "#162428"],
    ];
    for (const [foreground, background] of pairs) {
      expect(contrastRatio(foreground!, background!)).toBeGreaterThanOrEqual(4.5);
    }
  });

  test("keeps terminal syntax legible and preserves navigation at tablet widths", () => {
    const html = renderLanding({ units: [], upstreams: [] });
    expect(html).toContain(".term .k { color: var(--term-fg)");
    expect(html).toContain(".term .v { color: var(--term-dim)");
    expect(html).toContain("--term-dim: #aabbbc");
    expect(html).toContain("--term-bg: #0b1517");
    expect(html).toContain("--term-bg: #070c0d");
    expect(html).toContain("--term-fg: #e2eeea");
    expect(html).toContain("--term-fg: #e6eded");
    expect(html).toContain("--teal-bright: #6cc3d8");
    expect(html).toContain('.theme-btn.active, .theme-btn[aria-pressed="true"] { background: var(--term-bg); color: var(--term-fg)');
    expect(html).toContain("@media (max-width: 900px)");
    expect(html).not.toContain("nav.bar .links a.opt { display: none; }");
    expect(html).toContain("nav.bar .links { gap: .85rem; font-size: .84rem; flex-wrap: wrap; justify-content: flex-end; }");
  });

  test("articulates cross-domain joins and connecting the dots for combined analysis", () => {
    const html = renderLanding({ units: [], upstreams: [] });
    expect(html).toContain('id="joins"');
    expect(html).toContain("Single-purpose tools hold fragments. Sjel connects the dots.");
    expect(html).toContain("Hub Record · trip:plan:8f4c21");
    expect(html).toContain("cal:entry · Calendar");
    expect(html).toContain("transit · Travel");
    expect(html).toContain("fin:tx · Finance");
    expect(html).toContain("ent &amp; place · Network");
    expect(html).toContain("Combined query value");
    expect(html).toContain("Safe local synthesis");
    expect(html).toContain("The scenarios below use synthetic records; they are not live Sjel data or demonstrated query results.");
    expect(html).toContain("Illustrative query · not an executed command or result");
    expect(html).toContain("does not currently provide this query command or verify these results");
    expect(html).not.toContain("sjel ask");
    expect(html).not.toContain("0 bytes sent to cloud");
  });

  test("renders interactive simulators and SVG wire pulses", () => {
    const html = renderLanding({ units: [], upstreams: [] });
    // SVG wire pulses along topology paths
    expect(html).toContain('class="wire-pulse wire-pulse-phone"');
    expect(html).toContain('class="wire-pulse wire-pulse-mac"');
    expect(html).toContain('class="wire-pulse wire-pulse-model"');
    expect(html).toContain("@keyframes wire-flow");
    // Vector brand hedgehog mark
    expect(html).toContain('src="hedgehog.svg"');
    // Scenario tabs for Connected Intelligence explorer
    expect(html).toContain('class="joins-toolbar"');
    expect(html).toContain('data-scenario="trip"');
    expect(html).toContain('data-scenario="health"');
    expect(html).toContain('data-scenario="lease"');
    // Gate test bench controls
    expect(html).toContain('class="gate-ctrl-bar"');
    expect(html).toContain('data-gate-cls="c0"');
    expect(html).toContain('data-gate-cls="c1"');
    expect(html).toContain('data-gate-cls="c2"');
    expect(html).toContain('data-gate-cls="c3"');
    expect(html).toContain('data-gate-target="local"');
    expect(html).toContain('data-gate-target="cloud"');
    expect(html).toContain("GATE_EVAL");
  });
});
