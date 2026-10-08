// Three markup habits that are invisible to everyone who can see the page.
//
// Swept in `dashboard/src/` on 2026-09-08, and each count is the reason this file exists
// rather than a style note:
//
//   <th> with no scope             34   of 45. A header cell with no scope is a cell a
//                                       screen reader guesses the direction of, and it
//                                       guesses per browser. Nine of them are the finance
//                                       positions table, where every row is numbers.
//   controls with no name           4   Four `<input>`s named only by a placeholder, which
//                                       is not an accessible name: it disappears on the
//                                       first keystroke, so a reader arriving at a
//                                       half-filled field is told "edit text" and nothing.
//   <button> in a <form>, no type   3   Defaults to submit. The three here happen to be
//                                       the submit buttons, so nothing is broken today —
//                                       the defect is the next Cancel button somebody adds
//                                       beside one, which submits the form on click.
//
// The scan reads MARKUP only: `<script>` and `<style>` are cut out first, and so are HTML
// comments. Two `<input type="number">` in prose comments — RetrospectiveForm and
// PlanEditor each explain why a bind is held as a number — were counted as unlabelled
// controls by the first version of this scan, which is how a gate ends up with an escape
// list for something that was never a defect.

import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { extname, join } from "node:path";

const SRC = join(import.meta.dir, "../dashboard/src");

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const full = join(dir, name);
    if (statSync(full).isDirectory()) return sources(full);
    return extname(name) === ".svelte" ? [full] : [];
  });
}

const relative = (file: string) => file.slice(SRC.length + 1);

/**
 * A component's template, with script, style and comments blanked out.
 *
 * Blanked rather than deleted, and newlines kept: every offset in the result is still the
 * offset in the file, so a failure below can name the line an editor will open at. A gate
 * that reports the wrong line is a gate people stop reading.
 */
export function markup(text: string): string {
  const blank = (region: string) => region.replace(/[^\n]/g, " ");
  return text
    .replace(/<script[\s\S]*?<\/script>/g, blank)
    .replace(/<style[\s\S]*?<\/style>/g, blank)
    .replace(/<!--[\s\S]*?-->/g, blank);
}

/** Line number of an offset, so a failure names a place rather than a file. */
const lineOf = (text: string, index: number) => text.slice(0, index).split("\n").length;

const FILES = sources(SRC);

describe("the scan reads templates, not comments about templates", () => {
  test("there are sources to scan, so a passing run means something", () => {
    expect(FILES.length).toBeGreaterThan(80);
  });

  test("script, style and comment text are cut out", () => {
    const sample = `<script>const a = "<th>";</script>\n<!-- <input type="number"> -->\n<style>.x{}</style>\n<th>Real</th>`;
    expect(markup(sample).trim()).toBe("<th>Real</th>");
  });

  test("blanking keeps every line where it was, so a failure names the right one", () => {
    const sample = `<script>\n  // <th>\n</script>\n<th>Real</th>`;
    const stripped = markup(sample);
    expect(stripped.split("\n")).toHaveLength(4);
    expect(lineOf(stripped, stripped.indexOf("<th>Real"))).toBe(4);
  });

  test("the templates still hold the elements under test", () => {
    const all = FILES.map((file) => markup(readFileSync(file, "utf8"))).join("\n");
    // 45 until 2026-10-08, when the hand-built tables moved to the kit's DataTable. Its
    // <th scope> now covers every data table, so the floor only proves the scan still reads.
    expect(all.match(/<th\b/g)?.length ?? 0).toBeGreaterThanOrEqual(8);
    expect(all.match(/<button\b/g)?.length ?? 0).toBeGreaterThanOrEqual(200);
  });
});

describe("every header cell says which way it heads", () => {
  test("no <th> is missing scope", () => {
    const offenders: string[] = [];
    for (const file of FILES) {
      const template = markup(readFileSync(file, "utf8"));
      for (const match of template.matchAll(/<th\b[^>]*>/g)) {
        if (/\bscope\s*=/.test(match[0])) continue;
        offenders.push(`${relative(file)}:${lineOf(template, match.index)}`);
      }
    }
    expect(offenders.sort()).toEqual([]);
  });
});

describe("a button in a form says whether it submits", () => {
  /** Character ranges covered by a `<form>…</form>`. */
  function formRegions(template: string): [number, number][] {
    return [...template.matchAll(/<form\b/g)].map((match) => {
      const end = template.indexOf("</form>", match.index);
      return [match.index, end === -1 ? template.length : end] as [number, number];
    });
  }

  test("the region reader finds the forms", () => {
    // Fourteen on 2026-09-08. A floor, so adding a form is not a failure here.
    const forms = FILES.reduce(
      (sum, file) => sum + formRegions(markup(readFileSync(file, "utf8"))).length,
      0,
    );
    expect(forms).toBeGreaterThanOrEqual(14);
  });

  test("no <button> inside a <form> relies on the implicit submit default", () => {
    const offenders: string[] = [];
    for (const file of FILES) {
      const template = markup(readFileSync(file, "utf8"));
      const regions = formRegions(template);
      for (const match of template.matchAll(/<button\b[^>]*>/g)) {
        const at = match.index;
        if (/\btype\s*=/.test(match[0])) continue;
        if (!regions.some(([start, end]) => at > start && at < end)) continue;
        offenders.push(`${relative(file)}:${lineOf(template, at)}`);
      }
    }
    expect(offenders.sort()).toEqual([]);
  });
});

describe("every control a reader types into has a name", () => {
  /** Types that carry no value a reader supplies, so they need no name of their own. */
  const NAMELESS = ["hidden", "submit", "button", "reset", "image"];

  test("no input, select or textarea is named only by its placeholder", () => {
    const offenders: string[] = [];
    for (const file of FILES) {
      const template = markup(readFileSync(file, "utf8"));
      for (const match of template.matchAll(/<(input|select|textarea)\b([^>]*?)\/?>/g)) {
        const attrs = match[2];
        const type = /type\s*=\s*"([^"]*)"/.exec(attrs)?.[1] ?? "text";
        if (NAMELESS.includes(type)) continue;
        if (/aria-label\b|aria-labelledby\b/.test(attrs)) continue;
        // `<label for="x">` elsewhere in the same component names an `id="x"`.
        const id = /\bid\s*=\s*"([^"]*)"/.exec(attrs)?.[1];
        if (id && template.includes(`for="${id}"`)) continue;
        // Wrapped by a `<label>` that has not closed yet.
        const before = template.slice(0, match.index);
        if (before.lastIndexOf("<label") > before.lastIndexOf("</label>")) continue;
        offenders.push(`${relative(file)}:${lineOf(template, match.index)}`);
      }
    }
    expect(offenders.sort()).toEqual([]);
  });
});
