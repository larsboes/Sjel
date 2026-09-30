import { describe, expect, test } from "bun:test";

import { skillsLineWith, syncTargets } from "./harnesses.ts";

// `promote` takes the skill name off the command line and it names a directory under
// the harness destination, so the alphabet is "whatever a filename may contain" and
// not "whatever a TOML identifier may contain". Three of those characters used to
// change what got written; the cases below are the ones that used to.
describe("skillsLineWith", () => {
  test("appends the skill inside the array", () => {
    expect(skillsLineWith('skills = ["trim"]', "suggest-skills")).toBe('skills = ["trim", "suggest-skills"]');
  });

  test("appends past trailing whitespace, which is what the `]` anchor is for", () => {
    expect(skillsLineWith('skills = ["trim"]  ', "cv")).toBe('skills = ["trim", "cv"]');
  });

  test("refuses a skill the line already declares", () => {
    expect(() => skillsLineWith('skills = ["trim"]', "trim")).toThrow("already in the skills line");
  });

  // The regex-injection case (CodeQL alert 71). `new RegExp('"a.b"')` matched `"axb"`,
  // so promoting `a.b` into a Pack that carries `axb` was refused for a skill that is
  // not there. `.` and `|` are both legal in a macOS filename.
  test("a metacharacter in the name is a character, not a pattern", () => {
    expect(skillsLineWith('skills = ["axb"]', "a.b")).toBe('skills = ["axb", "a.b"]');
    expect(skillsLineWith('skills = ["a"]', "a|b")).toBe('skills = ["a", "a|b"]');
  });

  // The replacement-pattern case. `line.replace(/\]\s*$/, `, "${skill}"]`)` expanded
  // `$&` to the matched `]` and `$'` to the text after it.
  test("a replacement pattern in the name is inserted verbatim", () => {
    expect(skillsLineWith('skills = ["trim"]', "a$&b")).toBe('skills = ["trim", "a$&b"]');
    expect(skillsLineWith('skills = ["trim"]', "a$'b")).toBe(`skills = ["trim", "a$'b"]`);
  });

  // A Pack with no skills yet is the state `promote` exists to end, and the comma
  // splice wrote `skills = [, "trim"]` into the manifest for it — measured against
  // this function before this case was added, and against the code it replaced.
  test("fills an empty array without leaving a leading comma", () => {
    expect(skillsLineWith("skills = []", "trim")).toBe('skills = ["trim"]');
    expect(skillsLineWith("skills = [ ]", "trim")).toBe('skills = ["trim"]');
  });

  // Was a silent no-op: the body came back unchanged and promote still reported
  // success, so the skill was copied into the Pack and never declared by it.
  test("refuses a skills array that does not close on this line", () => {
    expect(() => skillsLineWith('skills = [', "trim")).toThrow("multi-line array");
  });
});

// `sync --all` was documented from the day the verb was written and never worked. The argv
// filter that builds `positional` drops anything starting with `--`, so `positional[1]` was
// always undefined for that form and `sync()` threw its usage error before reaching the
// `target === "--all"` branch — dead code that nothing exercised. Measured 2026-09-30.
describe("syncTargets", () => {
  test("a named pack is the only target", () => {
    expect(syncTargets("coding", false, ["writing", "coding"])).toEqual(["coding"]);
  });

  test("--all takes every pack the harness knows, sorted", () => {
    expect(syncTargets(undefined, true, ["writing", "coding", "harness"])).toEqual([
      "coding",
      "harness",
      "writing",
    ]);
  });

  test("--all with nothing deployed is an empty list, not a crash", () => {
    expect(syncTargets(undefined, true, [])).toEqual([]);
  });

  // The caller turns this into the usage error; the point of the case is that the
  // neither-given path is not silently an empty list.
  test("neither a pack nor --all selects nothing", () => {
    expect(syncTargets(undefined, false, ["coding"])).toEqual([]);
  });
});
