import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { exampleHtml } from "./build-example.mjs";
import { leverState } from "../assets/lever-model.mjs";

function chromeBinary() {
  if (process.env.CHROME_BIN) {
    assert(existsSync(process.env.CHROME_BIN), "CHROME_BIN does not exist.");
    return process.env.CHROME_BIN;
  }
  const mac = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
  if (existsSync(mac)) return mac;
  const result = spawnSync("which", ["chromium", "chromium-browser", "google-chrome"], { encoding: "utf8" });
  const binary = (result.stdout ?? "").trim().split("\n").find((path) => path && existsSync(path));
  assert(binary, "Chrome/Chromium is unavailable. Set CHROME_BIN. No browser was downloaded.");
  return binary;
}

/** Capture a public example with isolated temporary browser profiles and test its input wiring.
 * @throws {Error} If the browser, model state, readouts or responsive layout check fails.
 */
export function checkExample(outputDirectory) {
  const chrome = chromeBinary();
  const output = resolve(outputDirectory);
  mkdirSync(output, { recursive: true });
  const scratch = mkdtempSync(join(tmpdir(), "sjel-interactive-figure-"));
  const cases = [
    { name: "wide", width: 900, height: 700, arm: 2 },
    { name: "phone", width: 390, height: 780, arm: 2 },
    { name: "minimum", width: 900, height: 700, arm: 0.5 },
    { name: "maximum", width: 900, height: 700, arm: 4 },
    { name: "dark", width: 900, height: 700, arm: 2, dark: true },
  ];
  try {
    const results = cases.map((test) => {
      const file = join(scratch, test.name + ".html");
      const probe = `<script>const control=document.getElementById("arm");control.value=${test.arm};control.focus();control.dispatchEvent(new Event("input",{bubbles:true}));document.documentElement.dataset.overflow=String(document.documentElement.scrollWidth>innerWidth);document.documentElement.dataset.scheme=matchMedia("(prefers-color-scheme: dark)").matches?"dark":"light";</script>`;
      writeFileSync(file, exampleHtml().replace("</body>", probe + "</body>"), { flag: "wx" });
      const screenshot = join(output, test.name + ".png");
      const args = [
        "--headless=new", "--no-first-run", "--no-default-browser-check",
        "--disable-background-networking", "--disable-component-update", "--disable-sync", "--disable-extensions",
        "--user-data-dir=" + join(scratch, test.name + "-profile"),
        "--window-size=" + test.width + "," + test.height,
        "--screenshot=" + screenshot, "--dump-dom",
        ...(test.dark ? ["--force-dark-mode"] : []), pathToFileURL(file).href,
      ];
      const run = spawnSync(chrome, args, { encoding: "utf8", timeout: 30000, maxBuffer: 4 * 1024 * 1024 });
      assert(!run.error && run.status === 0, "Browser check failed: " + (run.error?.message ?? run.stderr.slice(0, 300)));
      const state = leverState(test.arm);
      assert(run.stdout.includes(`data-effort-force="${state.effortForce}"`), "Rendered force differs from model.");
      assert(run.stdout.includes(`data-effort-arm="${test.arm}"`), "Input event did not update state.");
      assert(run.stdout.includes(`>${state.effortForce.toFixed(1)} N effort</text>`), "SVG label did not update.");
      assert(run.stdout.includes(`data-overflow="false"`), "Page overflows horizontally.");
      assert(existsSync(screenshot), "Browser did not write a screenshot.");
      const scheme = /data-scheme="(dark|light)"/.exec(run.stdout)?.[1];
      if (test.dark) assert.equal(scheme, "dark", "Dark scheme was not exercised.");
      return { ...test, scheme, effortForce: state.effortForce, screenshot };
    });
    writeFileSync(join(output, "checks.json"), JSON.stringify({ checks: results, limitation: "Input events were dispatched programmatically. Native keyboard actions and Obsidian embedding were not exercised." }, null, 2) + "\n");
    return results;
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.length !== 3) {
    console.error("Usage: node scripts/check-example.mjs <output-directory>");
    process.exitCode = 2;
  } else {
    try { console.log(JSON.stringify(checkExample(process.argv[2]), null, 2)); }
    catch (error) { console.error(error.message); process.exitCode = 1; }
  }
}
