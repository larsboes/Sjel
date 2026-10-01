import { describe, expect, test } from "bun:test";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { addWatch, nameFromUrl, readRegisters, systemRows } from "./research-registers.ts";

test("every upstream row carries the one-line summary the Projects page shows", () => {
  const { upstreams } = readRegisters();
  const missing = upstreams.filter((u) => !u.summary).map((u) => u.name);
  const long = upstreams.filter((u) => u.summary.length > 110).map((u) => u.name);
  expect(missing).toEqual([]);
  expect(long).toEqual([]);
});

test("an overlay or local address is not published as a link", () => {
  const rows = systemRows({
    a: { url: "overlay:systems.local.toml", kind: "service", local: "yes", why: "x" },
    b: { url: "local", kind: "project", local: "yes", why: "y" },
    c: { url: "https://ollama.com/", kind: "tool", local: "no", why: "z" },
  });
  expect(rows.map((r) => r.url)).toEqual([null, null, "https://ollama.com/"]);
  expect(rows[2].local).toBe(false);
});

describe("addWatch", () => {
  const root = () => {
    const dir = mkdtempSync(join(tmpdir(), "sjel-upstreams-"));
    writeFileSync(join(dir, "upstreams.toml"), '[gpui]\nurl = "https://x"\nverdict = "watch"\n');
    return dir;
  };
  const opts = (dir: string) => ({ root: dir, license: async () => "MIT", today: "2026-09-30" });

  test("appends a watch row that parses and reads back", async () => {
    const dir = root();
    const row = await addWatch({ url: "https://github.com/Contrastive-LM/CLM", summary: 'A "quoted" line' }, opts(dir));
    expect(row).toEqual({ name: "clm", url: "https://github.com/Contrastive-LM/CLM", verdict: "watch", license: "MIT", summary: 'A "quoted" line' });
    const table = Bun.TOML.parse(readFileSync(join(dir, "upstreams.toml"), "utf8")) as Record<string, Record<string, string>>;
    expect(table.clm.why).toBe("noted 2026-09-30 from the dashboard's Projects page. Not audited.");
    expect(table.clm.summary).toBe('A "quoted" line');
    expect(table.gpui.verdict).toBe("watch");
  });

  test("refuses a duplicate, http, an empty or long summary, and a bad name", async () => {
    const dir = root();
    const refuse = (req: { url: string; summary: string; name?: string }) => expect(addWatch(req, opts(dir))).rejects.toThrow();
    await refuse({ url: "https://github.com/zed-industries/gpui", summary: "x" });
    await refuse({ url: "http://example.org/a/b", summary: "x" });
    await refuse({ url: "https://example.org/a/b", summary: "  " });
    await refuse({ url: "https://example.org/a/b", summary: "x".repeat(111) });
    await refuse({ url: "https://example.org/a/b", summary: "x", name: "Bad Name" });
    expect(readFileSync(join(dir, "upstreams.toml"), "utf8")).not.toContain("[b]");
  });

  test("names a row after the repository or model", () => {
    expect(nameFromUrl("https://huggingface.co/Contrastive-LM/CLM-v0.1-8B")).toBe("clm-v0-1-8b");
    expect(nameFromUrl("https://ollama.com/library/nimble")).toBe("nimble");
  });
});
