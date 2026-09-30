// No component may reference a custom property nothing declares.
//
// `var(--radius-lg)` was written at four call sites and declared at none, so three of them
// rendered square corners while every other card in the app was 10px rounded — a defect
// that is silent by construction: an unresolved var() with no fallback drops the whole
// declaration and CSS carries on. `--background` in the feed entry page is the same fault.
//
// The rule has to accept a component-local property, or it is a nuisance rather than a
// gate: a component that declares its own `--hour-h` and uses it two selectors later is
// making correct use of the mechanism. So a reference passes when the name is declared in
// app.css `:root`, OR declared anywhere in the same file, OR set through an inline `style`
// attribute or a `style:--x` binding in that file, OR carries a fallback.

import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { extname, join } from "node:path";

const SRC = join(import.meta.dir, "../dashboard/src");
const APP_CSS = join(SRC, "app.css");

/** Escapes hatch, one entry, each with the stream that owes the fix. */
const KNOWN: Record<string, string> = {
  "routes/feed/[id]/+page.svelte": "--background (owner: feed-personalization)",
};

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const full = join(dir, name);
    if (statSync(full).isDirectory()) return sources(full);
    return [".svelte", ".css"].includes(extname(name)) ? [full] : [];
  });
}

const relative = (file: string) => file.slice(SRC.length + 1);

/** `var(--name` with no comma after it — a reference with no fallback. */
const REFERENCE = /var\(\s*(--[a-z0-9-]+)\s*([,)])/gi;
const DECLARATION = /(^|[;{\s])(--[a-z0-9-]+)\s*:/gi;
/** `style:--x=` and `style="… --x: …"` both set a property from the template, and neither
 *  is preceded by the `;{` or whitespace a stylesheet declaration is. */
const INLINE = /(--[a-z0-9-]+)\s*[:=]/gi;

function declaredIn(text: string): Set<string> {
  const names = new Set<string>();
  for (const match of text.matchAll(DECLARATION)) names.add(match[2]);
  for (const match of text.matchAll(INLINE)) names.add(match[1]);
  return names;
}

/** The `:root` block of app.css, which is the shared vocabulary. */
function rootTokens(): Set<string> {
  const css = readFileSync(APP_CSS, "utf8");
  const names = new Set<string>();
  for (const block of css.matchAll(/:root[^{]*\{([^}]*)\}/g)) {
    for (const match of block[1].matchAll(DECLARATION)) names.add(match[2]);
  }
  return names;
}

describe("every custom property a component reads is declared somewhere it can see", () => {
  const files = sources(SRC);
  const root = rootTokens();

  test("there are sources to scan, so a passing run means something", () => {
    expect(files.length).toBeGreaterThan(30);
    expect(root.size).toBeGreaterThan(40);
  });

  test("app.css declares the properties this refresh added", () => {
    for (const name of [
      "--text-2xs", "--text-xs", "--text-sm", "--text-base", "--text-md",
      "--text-lg", "--text-xl", "--text-2xl",
      "--leading-tight", "--leading-normal", "--leading-relaxed", "--tracking-tight",
      "--space-1", "--space-2", "--space-3", "--space-4",
      "--space-5", "--space-6", "--space-7", "--space-8",
      "--radius-lg", "--measure", "--header-h", "--header-stack", "--rule",
      "--band-alarm", "--band-now", "--band-owed", "--band-offer",
      "--warning-ink", "--kind-event", "--motion-fast", "--motion-base", "--focus-ring",
    ]) {
      expect([name, root.has(name)]).toEqual([name, true]);
    }
  });

  test("no file reads a property that neither app.css nor the file itself declares", () => {
    const offenders: string[] = [];
    for (const file of files) {
      const text = readFileSync(file, "utf8");
      const local = declaredIn(text);
      const unresolved = new Set<string>();
      for (const match of text.matchAll(REFERENCE)) {
        const [, name, terminator] = match;
        if (terminator === ",") continue; // has a fallback
        if (root.has(name) || local.has(name)) continue;
        unresolved.add(name);
      }
      if (unresolved.size === 0) continue;
      const note = KNOWN[relative(file)];
      offenders.push(`${relative(file)}: ${[...unresolved].join(", ")}${note ? ` [known: ${note}]` : ""}`);
    }
    // Only the KNOWN list may appear, and the message names the owner.
    expect(offenders.filter((line) => !line.includes("[known:"))).toEqual([]);
  });

  test("the escape list stays exactly one entry, so it cannot grow quietly", () => {
    expect(Object.keys(KNOWN)).toHaveLength(1);
  });

  test("a component-local property is not flagged", () => {
    // The mechanism used correctly: ten of these exist today and v1's rule broke on all.
    const local = declaredIn(`
      <div style:--map-height="12rem"></div>
      <style>.map { height: var(--map-height); --overlay-width: 30rem; width: var(--overlay-width); }</style>
    `);
    expect(local.has("--map-height")).toBe(true);
    expect(local.has("--overlay-width")).toBe(true);
  });
});

// The same rule in the other direction: no primitive is declared that nothing uses.
//
// The suite above catches a component reading a property nobody declared. This catches the
// mirror image, and it had four on 2026-09-08: `.card-interactive`, `.glass`, `.glass-lit`
// and `.tnum` — all documented in `dashboard/README.md` and in app.css's own header, all
// with zero consumers across 96 components. Dead CSS is cheap to ship and expensive to
// read: the next person budgets for a glass system that no surface uses, and the README
// tells them it is there.
//
// A class counts as used when a component names it in a `class` attribute or a `class:`
// directive. A component's own `<style>` mentioning `.card` does not count — that is a
// scoped rule of its own that happens to share the name.
//
// The match is on the whole class token, not on a word boundary. `\bcard\b` also matches
// `card-interactive`, and `\btable\b` also matches `table-wrap` — which is the one hole
// that mattered here, because every primitive in this file is the prefix of a longer name
// somebody really does write. Verified 2026-09-08 by declaring `.zz` in app.css, writing
// `class="zz-decoration"` in one component, and watching the rule below stay green.

describe("no primitive is declared that nothing uses", () => {
  /** Class names app.css declares, read from selector positions only. */
  function declaredClasses(): string[] {
    const css = readFileSync(APP_CSS, "utf8")
      .replace(/\/\*[\s\S]*?\*\//g, "")
      // The grain SVG carries `www.w3.org` and the @font-face srcs carry `.woff2`.
      // Both read as class selectors to a regex and are neither.
      .replace(/url\([^)]*\)/g, "");
    const names = new Set<string>();
    // Every run of text that ends at a `{` is a selector — including one nested inside an
    // @media or @supports block, which a split on `}` would have skipped.
    for (const match of css.matchAll(/([^{}]*)\{/g)) {
      for (const found of match[1].matchAll(/\.([a-z][a-z0-9-]*)/gi)) names.add(found[1]);
    }
    // `.dark` is set on <html> by the theme toggle, never written in a component's markup.
    names.delete("dark");
    return [...names].sort();
  }

  /**
   * True when `text` writes `name` as a WHOLE class token.
   *
   * `(?<![\w-])name(?![\w-])`, not `\bname\b`: a hyphen is a word boundary to a regex and
   * is not one to CSS, so `\btable\b` counted `class="table-wrap"` as a use of `.table`.
   */
  function namesClass(text: string, name: string): boolean {
    const token = `(?<![\\w-])${name}(?![\\w-])`;
    return new RegExp(`class="[^"]*${token}|class=\\{[^}]*${token}|class:${name}(?![\\w-])`).test(
      text,
    );
  }

  /** The components that put `name` in a class attribute or a `class:` directive. */
  function consumers(name: string): string[] {
    return sources(SRC)
      .filter((file) => file.endsWith(".svelte") && namesClass(readFileSync(file, "utf8"), name))
      .map(relative);
  }

  /**
   * The escape list for unused primitives in app.css.
   * Empty now that `routes/backup/+page.svelte` adopts `.table`.
   */
  const KNOWN: Record<string, string> = {};

  test("the reader finds the primitives and not the font URLs", () => {
    const declared = declaredClasses();
    for (const name of ["card", "btn", "btn-primary", "tag", "input", "table", "mono", "num"]) {
      expect([name, declared.includes(name)]).toEqual([name, true]);
    }
    for (const name of ["org", "w3", "woff2"]) {
      expect([name, declared.includes(name)]).toEqual([name, false]);
    }
  });

  test("a longer class that merely starts with the name is not a use of it", () => {
    // The hole this closes, as four assertions rather than as a claim in a comment.
    expect(namesClass('<div class="card">', "card")).toBe(true);
    expect(namesClass('<div class="a card b">', "card")).toBe(true);
    expect(namesClass('<div class="card-interactive">', "card")).toBe(false);
    expect(namesClass('<div class="table-wrap">', "table")).toBe(false);
    expect(namesClass("<div class:card-lit>", "card")).toBe(false);
    expect(namesClass("<div class:card>", "card")).toBe(true);
  });

  test("every class app.css declares is named by at least one component", () => {
    const unused = declaredClasses()
      .filter((name) => consumers(name).length === 0)
      .map((name) => (KNOWN[name] ? `${name} [known: ${KNOWN[name]}]` : name));
    expect(unused.filter((line) => !line.includes("[known:"))).toEqual([]);
  });

  test("the escape list stays empty now that all primitives have consumers", () => {
    expect(Object.keys(KNOWN)).toEqual([]);
  });

  test("the consumer count is real, so a passing run means something", () => {
    // Without this, a reader that matched nothing would pass the test above by measuring
    // nothing at all — which is how a dead-code gate goes green on a dead codebase.
    expect(consumers("card").length).toBeGreaterThan(10);
    expect(consumers("btn").length).toBeGreaterThan(10);
    expect(consumers("a-name-nothing-uses")).toEqual([]);
  });
});
