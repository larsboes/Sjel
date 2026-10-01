// tools/sjel-mcp.test.ts — the MCP server's tool list follows the owner's mode (ISA ISC-40).
// Run: bun test tools/sjel-mcp.test.ts

import { describe, expect, test } from "bun:test";

import { callUrl, toolName, toolsFor, type Gate, type Route } from "./sjel-mcp.ts";

const ROUTES: Route[] = [
  { method: "GET", path: "/health", summary: "Liveness." },
  { method: "GET", path: "/routes", summary: "This manifest." },
  { method: "GET", path: "/triage", summary: "Mail proposals." },
  { method: "GET", path: "/discover", summary: "Crawl and rank." },
  { method: "POST", path: "/triage/{id}/status", summary: "Set a status." },
  { method: "POST", path: "/triage/{id}/gmail", summary: "Gmail action." },
];
const GATE: Gate = { capability: "comms", get_writes: ["GET /discover"], confirm: ["POST /triage/{id}/gmail"] };
const names = (mode: Parameters<typeof toolsFor>[1]) => toolsFor("comms", mode, ROUTES, GATE).map((t) => t.name);

describe("the tool list follows the mode", () => {
  test("off offers nothing", () => {
    expect(names("off")).toEqual([]);
  });

  test("read-only offers reads, and not a GET that writes", () => {
    expect(names("read-only")).toEqual(["comms__get_triage"]);
  });

  test("auto offers writes, and a confirm route says it waits", () => {
    const tools = toolsFor("comms", "auto", ROUTES, GATE);
    expect(tools.map((t) => t.name)).toEqual([
      "comms__get_triage",
      "comms__get_discover",
      "comms__post_triage_id_status",
      "comms__post_triage_id_gmail",
    ]);
    expect(tools[3].description).toContain("Waits for the owner");
    expect(tools[2].description).not.toContain("Waits for the owner");
    expect(tools[2].inputSchema.required).toEqual(["id"]);
  });

  test("ask marks every write as waiting", () => {
    const tools = toolsFor("comms", "ask", ROUTES, GATE);
    expect(tools.find((t) => t.name === "comms__post_triage_id_status")?.description).toContain("Waits for the owner");
  });
});

describe("names and URLs", () => {
  test("a name is MCP-legal and at most 64 characters", () => {
    const long = toolName("entities-google-sync", "POST", "/api/a-very/long/{path}/that/goes/on/and/on/and/on/forever");
    expect(long.length).toBeLessThanOrEqual(64);
    expect(long).toMatch(/^[a-zA-Z0-9_-]+$/);
    expect(toolName("trips", "GET", "/api/plans")).toBe("trips__get_api_plans");
  });

  test("path parameters are filled and encoded, and the query is appended", () => {
    expect(callUrl("http://127.0.0.1:8083", "/triage/{id}/status", { id: "a/b", query: { limit: 5 } })).toBe(
      "http://127.0.0.1:8083/triage/a%2Fb/status?limit=5",
    );
    expect(() => callUrl("http://x", "/triage/{id}", {})).toThrow("missing path parameter: id");
  });
});
