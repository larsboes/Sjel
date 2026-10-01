import { describe, expect, test } from "bun:test";

import {
  authorizedLoopbackRequest,
  dropped,
  historyOf,
  legacyObservations,
  lowestSeen,
  portInManifest,
  railWatchesOf,
  stageWatchesOf,
  stillPlanned,
  watchKey,
  withObservation,
} from "./sparpreis-watch.ts";

describe("authorizedLoopbackRequest", () => {
  test("adds the deployment credential to a loopback request", () => {
    const request = authorizedLoopbackRequest(
      "http://127.0.0.1:8090/api/plans",
      { method: "POST", headers: { "content-type": "application/json" }, body: "{}" },
      "Bearer test-token",
    );
    expect(new Headers(request.init.headers).get("authorization")).toBe("Bearer test-token");
    expect(request.init.method).toBe("POST");
  });

  test("refuses to put the deployment credential on a non-loopback request", () => {
    expect(() =>
      authorizedLoopbackRequest(
        "https://provider.example/api",
        {},
        "Bearer test-token",
      ),
    ).toThrow("not loopback");
  });
});

describe("railWatchesOf", () => {
  const railItem = {
    item_type: "option_set",
    external_id: "split:8000207:8000105",
    payload: { query: { from: "8000207", to: "8000105", time: "2026-09-01T08:00:00", bc: 25 } },
  };

  test("a rail option_set becomes a watch with its fare context", () => {
    const watches = railWatchesOf("p1", [railItem]);
    expect(watches).toEqual([
      {
        planId: "p1",
        from: "8000207",
        to: "8000105",
        time: "2026-09-01T08:00:00",
        bc: 25,
        dTicket: false,
        firstClass: false,
      },
    ]);
  });

  test("accommodation queries and the watch's own observations are not watches", () => {
    const accommodation = {
      item_type: "option_set",
      external_id: "booking.com:berlin",
      payload: { query: { from: "coordinate-anchor", to: "52.52,13.40", check_in: "2026-10-07" } },
    };
    const ownObservation = {
      item_type: "option_set",
      external_id: "sparpreis-watch:8000207:8000105:2026-09-01T08:00:00:2026-08-11",
      payload: { query: { from: "8000207", to: "8000105", time: "2026-09-01T08:00:00" } },
    };
    const stay = { item_type: "stay", external_id: "booking.com:1", payload: {} };
    expect(railWatchesOf("p1", [accommodation, ownObservation, stay])).toEqual([]);
  });
});

describe("stageWatchesOf", () => {
  const stage = (over: Record<string, unknown>) => ({
    id: "stage:a",
    date: "2026-10-07",
    status: "planning",
    transport_modes: ["train"],
    origin: { name: "Bonn" },
    destination: { name: "Stuttgart" },
    ...over,
  });

  test("an unbooked upcoming train stage is watched by place name on its date", () => {
    expect(stageWatchesOf("p1", [stage({})], "2026-09-25")).toEqual([
      { planId: "p1", from: "Bonn", to: "Stuttgart", time: "2026-10-07T07:00:00", stageId: "stage:a" },
    ]);
  });

  test("booked, completed, past, undated and non-train stages are not watched", () => {
    const stages = [
      stage({ status: "booked" }),
      stage({ status: "completed" }),
      stage({ date: "2026-09-01" }),
      stage({ date: null }),
      stage({ transport_modes: ["flight"] }),
      stage({ origin: {} }),
    ];
    expect(stageWatchesOf("p1", stages, "2026-09-25")).toEqual([]);
  });
});

describe("stillPlanned", () => {
  const watch = { planId: "p", from: "8000044", to: "8011160", time: "2026-10-07T08:00:00" };

  // The Berlin plan, 2026-09-25: its option_set searched Bonn -> Berlin while its
  // stages had become Bonn -> Stuttgart -> Berlin.
  test("an option_set whose route no stage resolved to is not watched", () => {
    const legs = new Set(["8000044:8000096:2026-10-07"]);
    expect(stillPlanned(watch, true, legs)).toBe(false);
    expect(stillPlanned(watch, true, new Set(["8000044:8011160:2026-10-07"]))).toBe(true);
  });

  test("a plan with no train stage keeps every option_set watch", () => {
    expect(stillPlanned(watch, false, new Set())).toBe(true);
  });
});

describe("history", () => {
  const key = "8000207:8000105:2026-09-01T08:00:00:bc25";
  const legacy = (day: string, prices: Array<number | null>) => ({
    id: `i-${day}`,
    item_type: "option_set",
    external_id: `sparpreis-watch:${key}:${day}`,
    payload: { options: prices.map((total_price) => ({ total_price })) },
  });

  test("per-day items group under their watch key", () => {
    const groups = legacyObservations([legacy("2026-08-10", [29.99]), legacy("2026-08-11", [35.99])]);
    expect([...groups.keys()]).toEqual([key]);
    expect(groups.get(key)?.map((o) => o.day)).toEqual(["2026-08-10", "2026-08-11"]);
  });

  test("the single item and legacy items merge, one entry per day", () => {
    const single = {
      item_type: "option_set",
      external_id: `sparpreis-watch:${key}`,
      payload: { history: [{ day: "2026-08-11", prices: [33.0] }, { day: "2026-08-12", prices: [40.0] }] },
    };
    const history = historyOf([legacy("2026-08-10", [29.99, null]), legacy("2026-08-11", [35.99]), single], key);
    expect(history).toEqual([
      { day: "2026-08-10", prices: [29.99] },
      { day: "2026-08-11", prices: [33.0] },
      { day: "2026-08-12", prices: [40.0] },
    ]);
  });

  test("a different watch key does not bleed in", () => {
    const other = legacy("2026-08-11", [9.99]);
    other.external_id = "sparpreis-watch:8000000:8000001:2026-09-01T08:00:00:2026-08-11";
    expect(historyOf([other], key)).toEqual([]);
  });

  // €67.99 -> €47.99 was reported as a drop on 2026-09-23 although €39.99 had been
  // seen on 2026-08-15. The comparison is against the lowest, not the latest.
  test("a new low is measured against every earlier observation", () => {
    const history = [
      { day: "2026-08-15", prices: [39.99] },
      { day: "2026-09-22", prices: [67.99] },
    ];
    expect(lowestSeen(history)).toBe(39.99);
    expect(dropped(lowestSeen(history), 47.99)).toBe(false);
    expect(dropped(lowestSeen(history), 34.99)).toBe(true);
    expect(lowestSeen([])).toBeNull();
  });

  test("today's observation replaces an earlier one from the same day", () => {
    const history = [{ day: "2026-09-25", prices: [50] }];
    expect(withObservation(history, { day: "2026-09-25", prices: [45] })).toEqual([
      { day: "2026-09-25", prices: [45] },
    ]);
  });
});

describe("dropped", () => {
  test("a real drop counts, float noise and first observations do not", () => {
    expect(dropped(35.99, 29.99)).toBe(true);
    expect(dropped(29.99, 29.985)).toBe(false);
    expect(dropped(null, 29.99)).toBe(false);
    expect(dropped(29.99, 35.99)).toBe(false);
  });
});

describe("watchKey", () => {
  test("fare context is part of the identity, absent context is absent", () => {
    expect(
      watchKey({ planId: "p", from: "1", to: "2", time: "2026-09-01T08:00:00", bc: 25 }),
    ).toBe("1:2:2026-09-01T08:00:00:bc25");
    expect(watchKey({ planId: "p", from: "1", to: "2", time: "2026-09-01T08:00:00" })).toBe(
      "1:2:2026-09-01T08:00:00",
    );
  });
});

// The port read out of a service.toml is interpolated straight into
// `http://127.0.0.1:${port}` by this tool and by tools/feed-sweep.ts, and feed-sweep puts
// the comms bearer token on that request. So the one thing this reader must not return is
// something that is not a port.
describe("portInManifest", () => {
  test("reads the port a manifest declares", () => {
    expect(portInManifest('name = "comms"\nport = "8099"\n')).toBe("8099");
  });

  test("refuses a manifest with no port line", () => {
    expect(() => portInManifest('name = "comms"\n')).toThrow("declares no port");
  });

  // Measured with bun: new URL("http://127.0.0.1:1@evil.example/x").host === "evil.example".
  // The last `@` before the path ends the userinfo, so this value moves the host off
  // loopback and takes the Authorization header with it.
  test("refuses a port that would move the host off loopback", () => {
    expect(() => portInManifest('port = "1@evil.example"\n')).toThrow("not a TCP port");
    expect(new URL(`http://127.0.0.1:1@evil.example/feed`).host).toBe("evil.example");
  });

  test("refuses a port carrying a path, a space or a scheme", () => {
    for (const bad of ["8099/../x", "80 99", "https://evil.example", "-1", "80990"]) {
      expect(() => portInManifest(`port = "${bad}"\n`)).toThrow("not a TCP port");
    }
  });
});
