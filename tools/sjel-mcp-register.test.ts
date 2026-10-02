// tools/sjel-mcp-register.test.ts — what the registration writes, and that it stays inside the
// managed policy's allowlist (ISA ISC-40). Run: bun test tools/sjel-mcp-register.test.ts

import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

import {
  claudeAddArgs,
  claudeRemoveArgs,
  piConfigWith,
  piConfigWithout,
  piEntry,
  SERVER_NAME,
  serverCommand,
} from "./sjel-mcp-register.ts";

const HOME = process.env.HOME ?? "";

describe("pi's entry", () => {
  test("carries the timeout pi's own CLI cannot set", () => {
    // `pi mcp add` rejects --timeout, and the default 60 s would cut off an ask-mode write that
    // waits up to 120 s for the owner's Allow. This field is why pi is written, not delegated.
    const entry = piEntry();
    expect(entry.timeout).toBe(180);
    expect(entry.exposure).toBe("codemode");
  });

  test("names the same command the server is started by", () => {
    const entry = piEntry();
    expect([entry.command as string, ...(entry.args as string[])]).toEqual(serverCommand());
  });
});

describe("pi's config keeps what it does not own", () => {
  test("sets our entry and leaves other servers and keys alone", () => {
    const before = {
      autoEnableCodemode: false,
      mcpServers: { other: { command: "other", args: [] } },
    };
    const after = piConfigWith(before, piEntry());
    expect(after.autoEnableCodemode).toBe(false);
    expect(Object.keys(after.mcpServers as object).sort()).toEqual(["other", SERVER_NAME]);
    expect((before.mcpServers as Record<string, unknown>)[SERVER_NAME]).toBeUndefined();
  });

  test("registering twice is the same file, not a duplicate", () => {
    const once = piConfigWith(null, piEntry());
    const twice = piConfigWith(once, piEntry());
    expect(JSON.stringify(twice)).toBe(JSON.stringify(once));
  });

  test("a missing, unparseable-shaped or array config still yields a usable one", () => {
    for (const before of [null, undefined, [], "nonsense", 42, { mcpServers: "not an object" }]) {
      const after = piConfigWith(before, piEntry());
      expect((after.mcpServers as Record<string, unknown>)[SERVER_NAME]).toEqual(piEntry());
    }
  });

  test("unregister removes only our entry, and drops the map when it empties", () => {
    const two = piConfigWith({ mcpServers: { other: { command: "other" } } }, piEntry());
    expect(Object.keys(piConfigWithout(two).mcpServers as object)).toEqual(["other"]);
    expect("mcpServers" in piConfigWithout(piConfigWith(null, piEntry()))).toBe(false);
  });
});

describe("claude's registration is the command the policy allows", () => {
  // The managed policy allowlists a stdio server by `serverCommand`, and commands match
  // exactly — every argument, in order. So the registered command and the allowlisted one are
  // one value with two readers, and this is the test that fails if they drift apart.
  const policy = JSON.parse(
    readFileSync(join(import.meta.dir, "templates", "claude-code", "managed-settings.json"), "utf8"),
  ) as { allowedMcpServers: { serverCommand?: string[] }[]; allowManagedMcpServersOnly: boolean };

  test("the policy pins exactly one command, by serverCommand and not by name", () => {
    expect(policy.allowManagedMcpServersOnly).toBe(true);
    expect(policy.allowedMcpServers).toHaveLength(1);
    expect(policy.allowedMcpServers[0].serverCommand).toBeDefined();
    // serverName would stop matching the moment any serverCommand entry exists, so pinning the
    // command is what makes the entry authoritative rather than decorative.
    expect(policy.allowedMcpServers[0]).not.toHaveProperty("serverName");
  });

  test("the allowlisted command is the one registration writes, argument for argument", () => {
    const allowed = policy.allowedMcpServers[0].serverCommand!;
    expect(allowed.map((part) => part.replace(/^\$\{HOME\}/, HOME))).toEqual(serverCommand());
  });

  test("the policy names no real home directory, so it stays public", () => {
    expect(JSON.stringify(policy)).not.toContain(HOME);
  });

  test("add and remove name the server and the scope, and pass the command after --", () => {
    const args = claudeAddArgs();
    expect(args.slice(0, 3)).toEqual(["mcp", "add", SERVER_NAME]);
    expect(args).toContain("--scope");
    expect(args.indexOf("--")).toBeLessThan(args.indexOf(serverCommand()[0]));
    expect(args.slice(args.indexOf("--") + 1)).toEqual(serverCommand());
    expect(claudeRemoveArgs()).toEqual(["mcp", "remove", SERVER_NAME, "--scope", "user"]);
  });
});
