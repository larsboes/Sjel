import { expect, test } from "bun:test";
import { env } from "./lib/env.ts";

test("a setting is read under its own name", () => {
  process.env.SJEL_ENV_TS_PRESENT = "new";
  expect(env("SJEL_ENV_TS_PRESENT")).toBe("new");
  expect(env("SJEL_ENV_TS_NEITHER")).toBeUndefined();
  expect(env("HOME")).toBe(process.env.HOME);
});

test("the pre-rename Axon name is no longer read", () => {
  // The compatibility layer was retired on 2026-09-30. A setting present only under the old
  // name must not answer for its SJEL_ one, so a half-migrated shell fails loudly.
  process.env.AXON_ENV_TS_RETIRED = "old";
  expect(env("SJEL_ENV_TS_RETIRED")).toBeUndefined();
});
