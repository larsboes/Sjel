// What a Home row is allowed to say about a data class, and what it must not.
//
// B50 (PRD §13.1) asked for the class to be carried onto every list contract. The half that
// keeps going wrong is not the carrying, it is the claiming: eight of ten kinds answered
// `() => null` while their capability published a class, `FinanceRow` declared `dataClass`
// in its props and rendered nothing, and `FeedItemRow` carried a comment saying the feed
// list had no class field three days after it grew one. Each of those is a row that states
// less than the capability knows, and the shape that would be worse — a row stating MORE —
// is what the first two tests below plant an input against.
//
// The kind functions are executed. The two component files are asserted as SOURCE, which
// is the same trade `tools/dashboard-row-disclosure.test.ts` makes and carries the same
// caveat: it holds the parts that regress silently, and whether the chip reads well on the
// page is a human's call.

import { describe, expect, test } from "bun:test";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const LIB = join(import.meta.dir, "../dashboard/src/lib");
const KINDS = join(LIB, "home/kinds");

type Kind = {
  key: string;
  capability: string | null;
  /** A component filename under `home/rows/`, resolved by `registry.ts`. */
  view: string;
  dataClass(row: unknown): string | null;
};

const kinds = await Promise.all(
  readdirSync(KINDS)
    .filter((name) => name.endsWith(".ts"))
    .map(async (name) => {
      const module = (await import(join(KINDS, name))) as { default: Kind };
      return { file: name, kind: module.default };
    }),
);

const rowMeta = readFileSync(join(LIB, "RowMeta.svelte"), "utf8");

/** The label map alone. Scoped, because the doc comment beside it NAMES the retired words
 *  to say they are retired, and a whole-file match reads that sentence as the defect. */
const classLabels = rowMeta.slice(
  rowMeta.indexOf("const CLASS_LABEL"),
  rowMeta.indexOf("};", rowMeta.indexOf("const CLASS_LABEL")),
);

const feedItemRow = readFileSync(join(LIB, "feed/FeedItemRow.svelte"), "utf8");

describe("a kind reports the class its capability published, and never one of its own", () => {
  test("there are kinds to check, so a passing run means something", () => {
    expect(kinds.length).toBeGreaterThanOrEqual(10);
  });

  test("a row that carries no class produces no class", () => {
    // The planted input: an object with nothing on it. A kind that answers anything but
    // null for this is deciding a class the capability never stated, which is the false
    // provenance claim `decisions.ts` forbids on `dataClass`. `null`, exactly — an
    // `undefined` here means the function's own declared return type is not true of it.
    const invented = kinds
      .map(({ file, kind }) => [file, kind.dataClass({} as never)] as const)
      .filter(([, value]) => value !== null);
    expect(invented).toEqual([]);
  });

  test("the class comes back verbatim on every kind whose capability publishes one", () => {
    // Verbatim is the whole property: `libs/content-item` decides what c2 means, and a kind
    // that re-derived, defaulted or clamped the value would be the second place deciding
    // that §6.1 forbids. Each row below is the shape its own capability serves.
    const byKey = new Map(kinds.map(({ kind }) => [kind.key, kind]));
    const cases: [string, unknown, string][] = [
      ["feed", { data_class: "c0" }, "c0"],
      ["mail", { data_class: "c2" }, "c2"],
      ["finance", { data_class: "c1" }, "c1"],
      ["calendar", { data_class: "c1" }, "c1"],
      ["task", { data_class: "c2" }, "c2"],
      ["opportunity", { data_class: "c0" }, "c0"],
    ];
    for (const [key, row, expected] of cases) {
      const kind = byKey.get(key);
      expect([key, kind?.dataClass(row)]).toEqual([key, expected]);
    }
  });

  test("a kind that reports no class is one whose contract states none", () => {
    // The list, so a kind going silent shows up as a failing test rather than as a chip
    // that stopped appearing. `host` and `system` read sjel-status, which serves machine
    // state and no content. The two trips kinds are an OPEN operator ruling: 12 of 13 live
    // plans name third parties, four with full names, so a plan is arguably c2 — and B50
    // says in as many words that inventing that answer is the failure mode, so they stay
    // null until the operator rules.
    //
    // Every other kind's capability publishes a class as of 2026-09-08, so this list is
    // now complete rather than provisional: a kind joining it is a regression.
    const silent = kinds
      .filter(({ kind }) => kind.dataClass({ data_class: "c2" } as never) === null)
      .map(({ kind }) => kind.key)
      .sort();
    expect(silent).toEqual([
      "host",
      "system",
      "trip",
      "trip-retrospective",
    ]);
  });
});

describe("a kind that publishes a class has a row that renders it", () => {
  test("every publishing kind's view hands the class to RowMeta", () => {
    // The defect this is shaped against is `FinanceRow`'s, found by B50 and fixed on
    // 2026-09-08: the row declared `dataClass` in its prop type, never destructured it and
    // rendered nothing, so "honoured on mail and finance" was true of the registry and
    // false of the pixels. A row may satisfy this in one of two ways — hand `{dataClass}`
    // to `RowMeta`, or delegate to a component that reads the class off the row itself,
    // which is what `FeedRow` does through `FeedItemRow`.
    //
    // Derived from the kinds rather than from a list, so a kind that starts publishing
    // fails here until its row catches up, with nobody having to remember this file.
    const renders = /<RowMeta[^>]*\{dataClass\}/s;
    const delegates = /data_class|<FeedItemRow/s;
    const dropped = kinds
      .filter(({ kind }) => kind.dataClass({ data_class: "c2" } as never) !== null)
      .filter(({ kind }) => {
        const view = readFileSync(join(LIB, `home/rows/${kind.view}.svelte`), "utf8");
        return !renders.test(view) && !delegates.test(view);
      })
      .map(({ kind }) => `${kind.key} -> ${kind.view}`);
    expect(dropped).toEqual([]);
  });
});

describe("the chip states the vocabulary the PRD defines", () => {
  test("the four words are the PRD's own, not a fourth dialect", () => {
    // PRD Axon.md §6.1: "C0 Public | C1 Mine | C2 Others | C3 Secret", ruled with the
    // sentence "two vocabularies standing side by side is the one outcome not allowed".
    // `libs/content-item/src/lib.rs` (`DataClass::new`) is the implementation it names.
    // This file printed Money for c1 and Private for c2 until 2026-09-08; "Private" is the
    // name of the RETIRED pre-Q27 class, which is exactly how a dialect gets read as the
    // real thing.
    for (const [value, label] of [
      ["c0", "Public"],
      ["c1", "Mine"],
      ["c2", "Others"],
      ["c3", "Secret"],
    ]) {
      expect([value, new RegExp(`${value}:\\s*"${label}"`).test(classLabels)]).toEqual([
        value,
        true,
      ]);
    }
    for (const retired of ["Money", "Private", "Sensitive"]) {
      expect([retired, classLabels.includes(`"${retired}"`)]).toEqual([retired, false]);
    }
  });

  test("an unreadable literal floors to the strictest word, not to an empty chip", () => {
    // Typed `DataClass`, so svelte-check refuses a bad literal from a caller — proven by
    // planting one — and JSON refuses nothing. The floor is `content-item`'s own arm.
    expect(rowMeta).toContain("?? CLASS_LABEL.c3");
  });
});

describe("the feed row renders the class the feed list publishes", () => {
  test("the claim that the list has no class field is gone", () => {
    // It stopped being true on 2026-09-06 (`capabilities/comms/src/server/contracts.rs`
    // publishes `data_class` on `FeedListItem`) and the comment outlived it by two days.
    expect(feedItemRow).not.toContain("FeedEntry` carries no class field");
    expect(feedItemRow).toContain("entry.data_class");
  });

  test("the meta line survives a row that has a class and no reason to be here", () => {
    // `{:else if whyHere}` alone dropped the whole line — chip included — for an item with
    // no evaluation and no profile match, which is most of a fresh inbox.
    expect(feedItemRow).toContain("whyHere || dataClass");
  });

  test("the class the row derived reaches the chip", () => {
    // Added by the verifier, 2026-09-08. The derived gate above lets `FeedRow` pass by
    // DELEGATING, and it never looks at what the delegate does with the class — so the
    // FinanceRow defect this whole file is shaped against survived one file over: deleting
    // `{dataClass}` from the call below leaves `entry.data_class` derived, leaves
    // `whyHere || dataClass` gating the line, and leaves all 519 assertions in `bun test`
    // green while the feed chip renders nothing. Measured, not assumed.
    expect(/<RowMeta[^>]*\{dataClass\}/s.test(feedItemRow)).toBe(true);
  });
});
