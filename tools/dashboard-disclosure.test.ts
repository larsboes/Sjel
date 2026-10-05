// Progressive disclosure and motion, held to the primitives that carry them.
//
// The rule (dashboard/README.md, "Disclosure and motion"): a hint is `use:tip`, never the
// `title` attribute; a floating sheet is a native `popover` with `.popover`; a row's rare
// actions go in ListRow's `secondary`; and motion reads the --motion-* tokens and names
// what it moves. Measured 2026-10-05 at 3e1a132a, before the first slice landed on Home:
// 96 `title=` attributes on HTML elements, 80 transition or animation declarations with a
// literal duration, and 20 `transition: all`. Four slices the same day took all three to
// zero outside one file.
//
// So these are rules, not ceilings: no file may carry one. The escape list names the one
// file still owed and why, and a test keeps it from growing. Infinite loops (a pulse, a
// shimmer) are exempt from the duration rule — their cadence is the design, not a
// transition between states.
//
// Not gated: transitions on layout properties. ListRow's meta line animates
// `grid-template-rows` on purpose (dashboard-row-disclosure.test.ts) and there is no
// transform that collapses a height. Review catches the rest.

import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { extname, join, relative } from "node:path";
import { placeTip } from "../dashboard/src/lib/tip.ts";

const SRC = join(import.meta.dir, "../dashboard/src");

/** Files allowed to carry `title` hints, with the reason. Shrinks; never grows. */
const KNOWN: Record<string, string> = {
  "routes/interior/+page.svelte":
    "another session had it open on 2026-10-05; migrate once that work lands",
};

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const full = join(dir, name);
    if (statSync(full).isDirectory()) return sources(full);
    return [".svelte", ".css"].includes(extname(name)) ? [full] : [];
  });
}

/** `title=` written on an HTML element. A component prop (`<PageHeader title=…>`) and a
 *  query string (`?title=`) are not tooltips. An <iframe> needs its title: it is the
 *  frame's accessible name, not a hint. */
export function htmlTitles(text: string): number {
  let count = 0;
  for (const match of text.matchAll(/\stitle=/g)) {
    const open = text.lastIndexOf("<", match.index);
    const tag = /^<([a-zA-Z][\w-]*)/.exec(text.slice(open))?.[1];
    if (!tag || tag !== tag.toLowerCase() || tag === "iframe") continue;
    count += 1;
  }
  return count;
}

const DURATION = /(?<![\w.-])\d*\.?\d+m?s\b/;

/** Transition and animation declarations, with their values. */
function motion(text: string): string[] {
  return [...text.matchAll(/(?:transition|animation)\s*:\s*([^;{}]*);/g)].map((m) => m[1]);
}

const files = sources(SRC).map((file) => ({ file: relative(SRC, file), text: readFileSync(file, "utf8") }));

describe("hints are use:tip, not title", () => {
  test("the reader skips component props, query strings and iframes", () => {
    expect(htmlTitles('<button\n  class="btn"\n  title="Dismiss">')).toBe(1);
    expect(htmlTitles('<PageHeader title="Home" />')).toBe(0);
    expect(htmlTitles('<a href="/calendar?title=x">')).toBe(0);
    expect(htmlTitles('<iframe title="Panel" src="/x">')).toBe(0);
  });

  test("no file carries one outside the escape list", () => {
    const offenders = files
      .filter(({ file, text }) => htmlTitles(text) > 0 && !KNOWN[file])
      .map(({ file, text }) => `${file}: ${htmlTitles(text)}`);
    expect(offenders).toEqual([]);
  });

  test("the escape list holds only files that still need it", () => {
    // A migrated file left on the list would let it regress in silence.
    for (const file of Object.keys(KNOWN)) {
      const text = files.find((f) => f.file === file)?.text ?? "";
      expect([file, htmlTitles(text) > 0]).toEqual([file, true]);
    }
    expect(Object.keys(KNOWN).length).toBeLessThanOrEqual(1);
  });
});

describe("motion reads the tokens and names what it moves", () => {
  const values = files.flatMap(({ text }) => motion(text)).filter((v) => !v.includes("infinite"));

  test("no declaration carries a literal duration", () => {
    expect(values.filter((v) => DURATION.test(v))).toEqual([]);
  });

  test("no declaration uses `transition: all`", () => {
    // `all` animates whatever the next edit changes, including layout properties.
    expect(values.filter((v) => /^\s*all\b/.test(v))).toEqual([]);
  });

  test("the duration reader is not fooled by tokens or easing numbers", () => {
    expect(DURATION.test("opacity var(--motion-fast) ease")).toBe(false);
    expect(DURATION.test("transform 120ms cubic-bezier(0.16, 1, 0.3, 1)")).toBe(true);
    expect(DURATION.test("color 0.15s ease")).toBe(true);
  });
});

describe("the floating primitives stay native", () => {
  const appCss = files.find((f) => f.file === "app.css")?.text ?? "";

  test(".popover fades out as well as in", () => {
    // Without allow-discrete the sheet leaves on the frame it closes, mid-fade.
    expect(appCss).toContain("@starting-style");
    expect(appCss).toContain("display var(--motion-base) allow-discrete");
  });

  test("every element that declares .popover also declares the popover attribute", () => {
    // The class without the attribute is a box with opacity 0: invisible and unreachable.
    for (const { file, text } of files.filter((f) => f.file.endsWith(".svelte"))) {
      for (const match of text.matchAll(/class="[^"]*(?<![\w-])popover(?![\w-])[^"]*"/g)) {
        const open = text.lastIndexOf("<", match.index);
        const close = text.indexOf(">", match.index);
        expect([file, /\spopover(?:[\s=>]|$)/.test(text.slice(open, close + 1))]).toEqual([file, true]);
      }
    }
  });
});

describe("placeTip keeps the tip on screen", () => {
  const viewport = { width: 1000, height: 800 };
  const size = { width: 100, height: 24 };

  test("above and centred when there is room", () => {
    expect(placeTip({ top: 200, left: 450, width: 100, height: 30 }, size, viewport)).toEqual({
      top: 170,
      left: 450,
    });
  });

  test("below when the anchor is at the top edge", () => {
    expect(placeTip({ top: 10, left: 450, width: 100, height: 30 }, size, viewport).top).toBe(46);
  });

  test("clamped at both sides", () => {
    expect(placeTip({ top: 200, left: 0, width: 20, height: 30 }, size, viewport).left).toBe(8);
    expect(placeTip({ top: 200, left: 990, width: 10, height: 30 }, size, viewport).left).toBe(892);
  });
});
