"use strict";
/*
 * Sjel Lens' pure half, and the loader contract under it.
 *
 * Every fixture below is a trimmed copy of a real answer, taken 2026-09-09 from the live
 * loopback services: the registry rows from `/api/sjel-status/capabilities` (then
 * served under its old name) and the plan rows from trips' `/api/plans`.
 * Shapes invented at a desk are how a test passes against an API that changed.
 *
 * The view itself is not here. It needs a workspace, and a fake workspace would report a
 * pass for code nobody ran.
 */

const { describe, expect, test } = require("bun:test");
const path = require("node:path");
const { loadPlugin, minimalObsidian } = require("./load-plugin.js");

const PLUGIN_DIR = path.join(__dirname, "sjel-lens");

/** Load once for the pure functions; the transport tests load their own with a stub. */
const loaded = loadPlugin(PLUGIN_DIR, minimalObsidian());
const lens = loaded.exports.lens;

/* ------------------------------------------------------------ the loader contract */

describe("what Obsidian will do with this file", () => {
  test("main.js exports a plugin class and requires only obsidian", () => {
    // loadPlugin refuses any other require id and throws when the export is not a class,
    // so reaching this line is the assertion. Restating it keeps the failure readable.
    expect(typeof loaded.exports).toBe("function");
  });

  test("the manifest claims nothing that needs a desktop", () => {
    expect(loaded.manifest.isDesktopOnly).toBe(false);
    expect(loaded.manifest.id).toBe("sjel-lens");
  });

  test("the pure half is reachable without constructing anything", () => {
    expect(Object.keys(lens).length).toBeGreaterThan(0);
  });
});

/* ----------------------------------------------------------------- discovery */

// Verbatim rows from the registry, 2026-09-09; the route is now /api/sjel-status/capabilities.
const REGISTRY = [
  {
    name: "vault",
    port: "8094",
    health_path: "/health",
    health_url: "http://127.0.0.1:8094/health",
    up: true,
  },
  {
    name: "trips",
    port: "8086",
    health_path: "/health",
    health_url: "http://127.0.0.1:8086/health",
    up: true,
  },
  {
    name: "punctuality",
    port: "8085",
    health_path: "/health",
    health_url: "http://127.0.0.1:8085/health",
    up: false,
  },
  {
    name: "knowledge-graph",
    port: "4244",
    health_path: "/api/graph/stats",
    health_url: "http://127.0.0.1:4244/api/graph/stats",
    up: false,
  },
  { name: "dashboard", port: "47117", health_path: "/", health_url: "http://127.0.0.1:47117/", up: false },
  { name: "knowledge-base", port: "", health_path: "", health_url: null, up: null },
];

describe("capabilityOrigin", () => {
  test("strips a single-segment health path", () => {
    expect(lens.capabilityOrigin(REGISTRY[0])).toBe("http://127.0.0.1:8094");
  });

  test("strips a multi-segment health path", () => {
    // knowledge-graph's health path is /api/graph/stats, not /health. A base built by
    // cutting at the third slash would be wrong here and right everywhere else.
    expect(lens.capabilityOrigin(REGISTRY[3])).toBe("http://127.0.0.1:4244");
  });

  test("a health path of / leaves no trailing slash to double up", () => {
    expect(lens.capabilityOrigin(REGISTRY[4])).toBe("http://127.0.0.1:47117");
  });

  test("a capability that serves nothing has no origin", () => {
    expect(lens.capabilityOrigin(REGISTRY[5])).toBeNull();
    expect(lens.capabilityOrigin(undefined)).toBeNull();
  });

  test("an external endpoint keeps its host", () => {
    // vaultwarden's row points off this machine. Deriving the origin from health_url
    // rather than from the bootstrap host is what makes that come out right.
    const row = { name: "vaultwarden", health_path: "/alive", health_url: "https://host.example/alive" };
    expect(lens.capabilityOrigin(row)).toBe("https://host.example");
  });
});

describe("resolveCapability", () => {
  test("finds a running capability", () => {
    expect(lens.resolveCapability(REGISTRY, "vault")).toEqual({
      ok: true,
      origin: "http://127.0.0.1:8094",
    });
  });

  test("names the three refusals apart", () => {
    // Each has a different fix, so each says a different thing.
    expect(lens.resolveCapability(REGISTRY, "punctuality").reason).toBe("punctuality is not running");
    expect(lens.resolveCapability(REGISTRY, "knowledge-base").reason).toBe("knowledge-base serves no HTTP surface");
    expect(lens.resolveCapability(REGISTRY, "soundscape").reason).toBe("soundscape is not enabled on this machine");
  });

  test("a registry that is not a list refuses rather than throws", () => {
    expect(lens.resolveCapability(null, "vault").ok).toBe(false);
  });
});

/* ------------------------------------------------------------------- routing */

describe("adapterFor", () => {
  test("routes the folder it knows", () => {
    expect(lens.adapterFor("Resources/Sjel/Trips/Berlin.md").id).toBe("trips");
  });

  test("everything else is nobody's note", () => {
    expect(lens.adapterFor("Journal/2026-09-09.md")).toBeNull();
    expect(lens.adapterFor("Atlas/People/Erika Mustermann.md")).toBeNull();
    expect(lens.adapterFor("Resources/Sjel/Trips/Berlin.png")).toBeNull();
    expect(lens.adapterFor(null)).toBeNull();
  });

  test("a lookalike folder does not match", () => {
    expect(lens.adapterFor("Resources/Sjel/Trips-Archive/Berlin.md")).toBeNull();
  });
});

/* --------------------------------------------------------------------- trips */

// Trimmed from GET :8086/api/plans, 2026-09-09, beside the frontmatter the projection
// wrote into Resources/Sjel/Trips/Berlin.md.
const PLANS = [
  { id: "trip:plan:18c72d1ebb4aac680000", title: "Berlin", updated_at: "1788965111" },
  { id: "trip:plan:18c72d21c85f5a900001", title: "DevFest Hamburg 2026", updated_at: "1787911108" },
];

const BERLIN_NOTE = {
  axon_trip_id: "trip:plan:18c72d1ebb4aac680000",
  axon_schema: "schemas/trip-plan.schema.json",
  axon_projection_version: "2",
  axon_revision: "1788965111",
  title: "Berlin",
};

describe("tripStaleness", () => {
  test("a projection at the plan's revision is current", () => {
    const verdict = lens.tripStaleness(BERLIN_NOTE, PLANS);
    expect(verdict.state).toBe("current");
    expect(verdict.title).toBe("Berlin");
  });

  test("a projection behind its plan is stale, by how much", () => {
    const note = { ...BERLIN_NOTE, axon_revision: "1788878711" };
    const verdict = lens.tripStaleness(note, PLANS);
    expect(verdict.state).toBe("stale");
    expect(verdict.behindSeconds).toBe(86400);
    expect(lens.stalenessLabel(verdict).detail).toContain("24 hours");
  });

  test("a projection ahead of its plan is reported, not hidden", () => {
    // Should be impossible: the export overwrites the file whole. Saying "current" here
    // would be the pane lying about the one thing it exists to answer.
    const note = { ...BERLIN_NOTE, axon_revision: "1788965711" };
    const verdict = lens.tripStaleness(note, PLANS);
    expect(verdict.state).toBe("ahead");
    expect(verdict.aheadSeconds).toBe(600);
  });

  test("a trip id no plan carries is orphaned", () => {
    const note = { ...BERLIN_NOTE, axon_trip_id: "trip:plan:deleted" };
    expect(lens.tripStaleness(note, PLANS).state).toBe("orphaned");
  });

  test("a hand-written note in the folder is not a projection", () => {
    expect(lens.tripStaleness({ title: "My own Berlin notes" }, PLANS).state).toBe("not-a-projection");
    expect(lens.tripStaleness(undefined, PLANS).state).toBe("not-a-projection");
  });

  test("a revision that is not a revision says so", () => {
    const note = { ...BERLIN_NOTE, axon_revision: "2026-10-07" };
    expect(lens.tripStaleness(note, PLANS).state).toBe("unreadable");
  });

  test("plans that did not arrive leave every note orphaned rather than current", () => {
    expect(lens.tripStaleness(BERLIN_NOTE, null).state).toBe("orphaned");
  });
});

describe("toEpochSeconds", () => {
  test("takes the string form the projection writes and the store returns", () => {
    expect(lens.toEpochSeconds("1788965111")).toBe(1788965111);
    expect(lens.toEpochSeconds(1788965111)).toBe(1788965111);
  });

  test("refuses anything that is not whole seconds", () => {
    expect(lens.toEpochSeconds("2026-10-07")).toBeNull();
    expect(lens.toEpochSeconds("")).toBeNull();
    expect(lens.toEpochSeconds(null)).toBeNull();
    expect(lens.toEpochSeconds("12abc")).toBeNull();
  });
});

describe("humanDuration", () => {
  test("changes unit at the boundary and never below zero", () => {
    expect(lens.humanDuration(1)).toBe("1 second");
    expect(lens.humanDuration(59)).toBe("59 seconds");
    expect(lens.humanDuration(60)).toBe("1 minute");
    expect(lens.humanDuration(3600)).toBe("1 hour");
    expect(lens.humanDuration(47 * 3600)).toBe("47 hours");
    expect(lens.humanDuration(48 * 3600)).toBe("2 days");
    expect(lens.humanDuration(-5)).toBe("0 seconds");
  });
});

describe("stalenessLabel", () => {
  test("every state gets a badge and a consequence", () => {
    for (const state of ["current", "stale", "ahead", "orphaned", "unreadable", "not-a-projection"]) {
      const label = lens.stalenessLabel({ state, behindSeconds: 60, aheadSeconds: 60 });
      expect(label.badge.length).toBeGreaterThan(0);
      expect(label.detail.length).toBeGreaterThan(0);
    }
  });
});

/* ----------------------------------------------------------------- transport */

describe("readJson", () => {
  /** Load a second copy whose requestUrl is ours. */
  function withTransport(requestUrl) {
    return loadPlugin(PLUGIN_DIR, minimalObsidian({ requestUrl })).exports.lens;
  }

  test("a 200 answers with the parsed body", async () => {
    const stub = withTransport(async () => ({ status: 200, json: { plans: 2 } }));
    await expect(stub.readJson("http://127.0.0.1:8086/api/plans", 1000)).resolves.toEqual({ plans: 2 });
  });

  test("a refusal carries its status into the message", async () => {
    // The origin guard's refusal, as `libs/sjel-server/src/origin.rs` words it. The inbound
    // gate's 401 takes the same path. The pane must print the status, not an empty table.
    const stub = withTransport(async () => ({ status: 403, json: { error: "cross-origin access to vault is not allowed" } }));
    await expect(stub.readJson("http://127.0.0.1:8086/api/plans", 1000)).rejects.toThrow("answered 403");
  });

  test("a capability that never answers loses to the deadline", async () => {
    const stub = withTransport(() => new Promise(() => {}));
    await expect(stub.readJson("http://127.0.0.1:8086/api/plans", 20)).rejects.toThrow("did not answer within 20 ms");
  });

  test("it asks for the status rather than an exception", async () => {
    let seen = null;
    const stub = withTransport(async (options) => {
      seen = options;
      return { status: 200, json: {} };
    });
    await stub.readJson("http://127.0.0.1:8086/api/plans", 1000);
    expect(seen.method).toBe("GET");
    expect(seen.throw).toBe(false);
  });
});
