import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { leverState } from "../assets/lever-model.mjs";
import { exampleHtml } from "./build-example.mjs";

describe("ideal lever model", () => {
  for (const [arm, force] of [[0.5, 200], [1, 100], [2, 50], [4, 25]]) {
    test(`${arm} m balances the fixed load with ${force} N`, () => {
      const state = leverState(arm);
      expect(state.effortForce).toBe(force);
      expect(state.effortForce * arm).toBe(state.loadForce * state.loadArm);
      expect(state.moment).toBe(100);
      expect(300 - state.effortX).toBe(arm * 50);
      expect(state.loadX - 300).toBe(state.loadArm * 50);
      expect((199 - state.effortArrowTop) / (199 - state.loadArrowTop)).toBeCloseTo(force / 100);
    });
  }
  for (const arm of [-1, 0, 0.49, 4.1, NaN, Infinity, "2"]) {
    test(`rejects invalid arm ${String(arm)}`, () => {
      expect(() => leverState(arm)).toThrow(RangeError);
    });
  }
});

describe("standalone example", () => {
  test("generated HTML matches the checked-in artifact", () => {
    expect(readFileSync(new URL("../assets/lever.html", import.meta.url), "utf8")).toBe(exampleHtml());
  });
  test("uses the tested model and native accessible controls", () => {
    const html = exampleHtml();
    expect(html).toContain(readFileSync(new URL("../assets/lever-model.mjs", import.meta.url), "utf8")
      .replace("export function leverState(", "function leverState("));
    expect(html).toContain('type="range" min="0.5" max="4"');
    expect(html).toContain('<label for="arm">');
    expect(html).toContain('aria-live="polite"');
    expect(html).toContain('aria-labelledby="lever-title lever-description"');
    expect(html).toContain("This is not a motion simulation.");
  });
  test("contains no network resource or animation loop", () => {
    const html = exampleHtml();
    expect(html).toContain("default-src 'none'");
    expect(html).not.toMatch(/(?:src|href)=["']https?:/);
    expect(html).not.toMatch(/\b(?:fetch|requestAnimationFrame|setInterval)\s*\(/);
    expect(html).toContain("<noscript>");
  });
});
