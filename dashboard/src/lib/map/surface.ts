// dashboard/src/lib/map/surface.ts — one MapLibre lifecycle for the whole shell.
//
// What this replaces. `MapView.svelte` and `TripMap.svelte` were 679 lines implementing the
// same six things twice: the lazy `import("maplibre-gl")`, the IntersectionObserver that
// gates it, the resize dance for a host that measures 0x0 on its first frame, the fit-bounds
// key, and the deferred / loading / failed states. MapView's own header said so — TripMap's
// logic was "carried over unchanged". Two copies of a lifecycle is one copy of a bug.
//
// What it adds. A Map instance outlives the component that used it. Leaving /map for /travel
// used to call `map.remove()` and pay the entire cold chain again: 980 KB of already-parsed
// library thrown away, the style re-parsed, the sprite and glyphs re-decoded, every visible
// tile re-uploaded to the GPU. Here the instance is detached and parked, and the next view
// re-parents its container and swaps sources and layers. Route change costs a `setData`.
//
// It is a pool, not a singleton, because two maps can legitimately be on screen at once:
// `/travel` renders a trip map in `{#if viewingPast}` and another in `{#if mapOpen}` inside
// the same plan-detail branch. A singleton would have silently stolen the canvas from the
// first. `lease()` hands out a parked instance if one is free and builds a second if not.
//
// The modularity this preserves: a view still owns its own layer specs, its own paint
// expressions and its own data. What it stops owning is WebGL.

// The base path comes through $lib/nav's `link`, not from `$app/paths` directly. That alias
// does not resolve under plain `bun test`, so importing it here would put it into the import
// graph of every module that touches the map — the same failure nav.ts's own header
// describes, and `tools/dashboard-nav-links.test.ts` fails the build on it. Exactly one
// module reads the alias, and it is the root layout.
import { link } from "../nav";
import type {
  AddLayerObject,
  GeoJSONSource,
  LngLatBoundsLike,
  LngLatLike,
  Map as MapLibreMap,
  MapGeoJSONFeature,
  MapLayerMouseEvent,
  RequestParameters,
  StyleSpecification,
} from "maplibre-gl";

/** MapLibre reports an `ErrorLike`, which is not necessarily a real `Error`. */
type MapLibreErrorEvent = { error?: { message?: string } };

/** Structural GeoJSON, so callers pass wire-typed collections without casts. */
export interface MapFeature {
  type: "Feature";
  geometry: {
    type: "Point" | "LineString";
    coordinates: [number, number] | [number, number][];
  };
  properties: unknown;
}

export interface MapFeatureCollection {
  type: "FeatureCollection";
  features: MapFeature[];
}

export interface MapLayerSpec {
  id: string;
  type: "circle" | "line" | "symbol";
  source: string;
  filter?: unknown[];
  layout?: Record<string, unknown>;
  paint?: Record<string, unknown>;
}

export type MapLibrary = typeof import("maplibre-gl");

/** The vendored basemap, served from this origin. `tools/fetch-basemap` writes it. */
const STYLE_PATH = "/basemap/style.json";
const MANIFEST_PATH = "/basemap/manifest.json";
/** Where a glyph range this repository did not vendor is fetched from instead. */
const UPSTREAM_FONTS = "https://tiles.openfreemap.org/fonts";

const DEFAULT_CENTER: [number, number] = [10.45, 51.16];
const DEFAULT_ZOOM = 4.2;

interface Manifest {
  fetched_at: string;
  planet_snapshot: string;
  glyphs: string[];
}

let library: MapLibrary | undefined;
let vendoredGlyphs: Set<string> | undefined;
let booting: Promise<Booted> | undefined;

interface Booted {
  maplibregl: MapLibrary;
  /** The vendored style, already resolved to absolute URLs. Reused by every Map. */
  style: StyleSpecification;
}

/**
 * Absolute, because MapLibre v6 refuses a root-relative sprite outright:
 *
 *   Invalid sprite URL "/basemap/sprite/ofm", must be absolute. Modify style specification
 *   directly or use TransformStyleFunction to correct the issue dynamically
 *
 * It is a silent failure in the shape that matters — the Map constructs, its controls render,
 * and the style never finishes loading, so the frame sits on "Loading map…" forever with no
 * request in the network log to explain it. Vendoring the style is what turned a URL MapLibre
 * had always resolved for us into one we own, so resolving it is ours too.
 *
 * Concatenation rather than `new URL(path, location.href)`, which looks more correct and is
 * wrong here: the glyph value is a TEMPLATE, and `new URL` percent-encodes its braces into
 * `%7Bfontstack%7D`. MapLibre substitutes `{fontstack}` literally, so the encoded form matches
 * nothing and every glyph request 404s — a map with no labels and no error to say why.
 */
const absolute = (path: string): string => `${location.origin}${link(path)}`;

/**
 * The library, the manifest and the style, once per session.
 *
 * All three together rather than in sequence. The manifest is ~200 bytes and the style ~100 KB
 * off loopback; neither can be the slow one beside a 980 KB chunk, so waiting for them costs
 * nothing and having them first is what lets the Map be constructed from a style OBJECT.
 * That matters twice: MapLibre does not re-fetch it, and the sprite and glyph URLs are made
 * absolute here instead of being baked into a static file that cannot know its own origin.
 */
export function boot(): Promise<Booted> {
  booting ??= Promise.all([
    import("maplibre-gl"),
    import("maplibre-gl/dist/maplibre-gl.css"),
    fetch(link(MANIFEST_PATH))
      .then((response) => (response.ok ? (response.json() as Promise<Manifest>) : null))
      .catch(() => null),
    fetch(link(STYLE_PATH)).then((response) => response.json() as Promise<StyleSpecification>),
    // MapLibre's own worker, emitted and hashed by Vite, handed back as a URL.
    //
    // It has to be given to MapLibre explicitly, and the reason is a build-analysis gap with a
    // nasty failure mode. MapLibre resolves its worker itself, from a TEMPLATE literal:
    //
    //   let t = import.meta.url.endsWith("-dev.mjs") ? "…-worker-dev.mjs" : "maplibre-gl-worker.mjs";
    //   return new URL(`./${t}`, import.meta.url);
    //
    // Rollup only follows `new URL("literal", import.meta.url)`, so it emits no asset at all and
    // the URL MapLibre computes points at a file that was never built. On this shell that path
    // falls through sjel-status' SPA fallback and answers 200 with the app shell, so `new Worker`
    // is handed HTML, dies, and MapLibre reports nothing: a blank canvas and "Loading map…"
    // forever, with no error and no failed request to find. Measured 2026-09-06 — no build in
    // this repository had ever emitted that file, so every map on the served bundle was dead.
    //
    // Dynamic rather than a static top-level import, for the same reason the library itself is:
    // `vite.config.ts`'s bundleGuard matches on the module id, and a static import of anything
    // under maplibre-gl/ makes a route node's chunk statically reachable. One rule, no exception.
    import("maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url"),
  ]).then(([maplibregl, , manifest, style, worker]) => {
    library = maplibregl;
    // Before any Map exists, because the pool constructs one immediately after this resolves.
    maplibregl.setWorkerUrl(worker.default);
    // A missing manifest is not a failure: every glyph then misses the vendored set and goes
    // upstream, which is exactly the behaviour before this module existed.
    vendoredGlyphs = new Set(manifest?.glyphs ?? []);
    if (typeof style.sprite === "string") style.sprite = absolute(style.sprite);
    if (style.glyphs) style.glyphs = absolute(style.glyphs);
    return { maplibregl, style };
  });
  return booting;
}

/**
 * Start the download before anything needs it. Safe to call repeatedly and from anywhere —
 * the nav calls it on hover over a map route, so by the time the route module has resolved
 * the library is parsed and the style is already an object.
 */
export function warm(): void {
  void boot().catch(() => {});
}

const EMPTY_GLYPHS: ReadonlySet<string> = new Set();

/**
 * The one thing left for MapLibre to ask us at request time: is this glyph range vendored?
 *
 * Four Latin ranges are (`static/basemap/LICENSE.md`), and `tools/fetch-basemap` also strips the
 * `name:nonlatin` half of every label, so in practice nothing should miss. This stays because
 * "should" is not "does": a style refresh that reintroduced the two-script label form would
 * otherwise draw a labelless map, and a slow label is a much better failure than a missing one.
 * Every other URL in the style is already absolute and correct by the time it gets here.
 *
 * Pure and exported so `tools/dashboard-basemap.test.ts` can assert the fallback without a
 * browser, a Map, or a network.
 */
export function basemapUrl(url: string, vendoredGlyphKeys: ReadonlySet<string>): string {
  const glyph = /\/basemap\/fonts\/([^/]+)\/(\d+-\d+)\.pbf$/.exec(url);
  if (!glyph) return url;
  const [, encodedStack, range] = glyph;
  if (vendoredGlyphKeys.has(`${decodeURIComponent(encodedStack)}/${range}`)) return url;
  return `${UPSTREAM_FONTS}/${encodedStack}/${range}.pbf`;
}

function transformRequest(url: string): RequestParameters | undefined {
  const rewritten = basemapUrl(url, vendoredGlyphs ?? EMPTY_GLYPHS);
  return rewritten === url ? undefined : { url: rewritten };
}

/** Instances that are alive, styled and parked off-DOM, waiting for the next view. */
const parked: MapLibreMap[] = [];

export interface LeaseOptions {
  center?: [number, number];
  zoom?: number;
}

/** What a view attaches to the map, in one call so a swap is atomic. */
export interface MapContent {
  sources: Record<string, MapFeatureCollection>;
  layers: MapLayerSpec[];
  /** Layer ids that take a pointer cursor and report clicks, topmost first. */
  interactive?: string[];
  onFeatureClick?: (layerId: string, feature: MapGeoJSONFeature) => void;
  /** Pre-escaped HTML for a click popup; null or undefined opens none. */
  popupHtml?: (layerId: string, properties: Record<string, unknown>) => string | null;
}

export interface MapLease {
  readonly map: MapLibreMap;
  /** Replace what is drawn. Adds and removes layers as the spec list changes. */
  apply(content: MapContent): void;
  /** Push new feature data into sources that already exist. The hot path. */
  refresh(sources: Record<string, MapFeatureCollection>): void;
  fit(sources: Record<string, MapFeatureCollection>, options?: { maxZoom?: number }): boolean;
  flyTo(target: [number, number], zoom?: number): void;
  resize(): void;
  /** Strip this view's layers and park the instance for the next one. */
  release(): void;
}

/**
 * Wait for a Map to be ready for `addSource`, whether it is fresh or parked.
 *
 * It rejects on a MapLibre `error` rather than only resolving on `load`, and that is not
 * defensive padding — a style MapLibre refuses never fires `load`, so without this the
 * promise simply never settles and the frame reads "Loading map…" for the rest of the
 * session with nothing in the network log to explain it. That is exactly how the v6 absolute
 * sprite rule was found. An error the caller can render beats a wait nobody can end.
 */
function styleReady(map: MapLibreMap): Promise<void> {
  if (map.isStyleLoaded()) return Promise.resolve();
  return new Promise((resolve, reject) => {
    const onError = (event: MapLibreErrorEvent) =>
      reject(new Error(event.error?.message ?? "the map failed to load"));
    map.once("load", () => {
      map.off("error", onError);
      resolve();
    });
    map.on("error", onError);
  });
}

/**
 * Take a map for this host. Resolves once the style is loaded and the lease can be applied.
 *
 * The container element travels with the instance: parking detaches it from the DOM without
 * destroying the WebGL context, and leasing re-parents it under the new host. That is the
 * whole trick, and it is why `resize()` follows immediately — the new host's box is almost
 * never the old one's.
 */
export async function lease(host: HTMLElement, options: LeaseOptions = {}): Promise<MapLease> {
  const { maplibregl, style } = await boot();

  let map = parked.pop();
  if (map) {
    host.appendChild(map.getContainer());
    map.jumpTo({ center: options.center ?? DEFAULT_CENTER, zoom: options.zoom ?? DEFAULT_ZOOM });
  } else {
    // MapLibre uses the element it is handed as its container -- `getContainer()` returns
    // that exact node. So the surface owns an inner div rather than the caller's host:
    // parking detaches this one, and the component's own element is never touched.
    const canvasHost = document.createElement("div");
    canvasHost.className = "map-canvas-host";
    // Sized here, not in a stylesheet. This element is created by this module and re-parented
    // into a different component instance over its life, so depending on a scoped rule that
    // happens to match its current parent is a bug waiting for the next caller: a host without
    // that rule gives MapLibre a 0x0 box, and a 0x0 map renders nothing while reporting itself
    // perfectly healthy -- style loaded, tiles loaded, no error.
    canvasHost.style.cssText = "position:absolute;inset:0";
    host.appendChild(canvasHost);
    map = new maplibregl.Map({
      container: canvasHost,
      // A fresh copy per Map. MapLibre normalises and annotates the style spec it is handed,
      // and this module keeps ONE parsed style for the whole session -- so handing the same
      // object to a second Map would hand it one the first had already rewritten. The clone
      // costs a fraction of a millisecond against a 980 KB library load.
      style: structuredClone(style),
      center: options.center ?? DEFAULT_CENTER,
      zoom: options.zoom ?? DEFAULT_ZOOM,
      cooperativeGestures: true,
      attributionControl: { compact: true },
      transformRequest,
    });
    map.addControl(new maplibregl.NavigationControl({ showCompass: false }), "top-right");
  }

  try {
    await styleReady(map);
  } catch (cause) {
    // The map is unusable and nothing else holds it. Without this it stays alive off-screen,
    // holding a WebGL context a browser only grants ~16 of, for the rest of the session.
    map.remove();
    throw cause;
  }

  // Everything this lease added, so `release` can undo exactly that and leave the 111
  // basemap layers alone.
  let ownedLayers: string[] = [];
  let ownedSources: string[] = [];
  // Detach closures rather than (event, layer, handler) tuples: MapLibre's `off` overloads
  // key on a literal event name, so storing the name as a string loses the type that makes
  // the call legal. A closure keeps it.
  let detach: Array<() => void> = [];
  let popup: import("maplibre-gl").Popup | undefined;
  // One click can land on features in several interactive layers; the first registered
  // handler (the topmost layer, by `interactive` order) wins.
  let handledClick: Event | undefined;
  let released = false;

  function bind(content: MapContent): void {
    const { interactive = [], onFeatureClick, popupHtml } = content;
    for (const layerId of interactive) {
      const click = (event: MapLayerMouseEvent) => {
        const feature = event.features?.[0];
        if (!feature || event.originalEvent === handledClick) return;
        handledClick = event.originalEvent;
        onFeatureClick?.(layerId, feature);
        const html = popupHtml?.(layerId, feature.properties ?? {});
        if (html && library && map) {
          popup?.remove();
          popup = new library.Popup({ maxWidth: "300px", offset: 12 })
            .setLngLat(event.lngLat)
            .setHTML(html)
            .addTo(map);
        }
      };
      const enter = () => {
        if (map) map.getCanvas().style.cursor = "pointer";
      };
      const leave = () => {
        if (map) map.getCanvas().style.cursor = "";
      };
      map?.on("click", layerId, click);
      map?.on("mouseenter", layerId, enter);
      map?.on("mouseleave", layerId, leave);
      detach.push(
        () => map?.off("click", layerId, click),
        () => map?.off("mouseenter", layerId, enter),
        () => map?.off("mouseleave", layerId, leave),
      );
    }
  }

  function strip(): void {
    for (const off of detach) off();
    detach = [];
    popup?.remove();
    popup = undefined;
    for (const id of ownedLayers) if (map?.getLayer(id)) map.removeLayer(id);
    for (const id of ownedSources) if (map?.getSource(id)) map.removeSource(id);
    ownedLayers = [];
    ownedSources = [];
  }

  const leaseHandle: MapLease = {
    get map() {
      return map as MapLibreMap;
    },

    apply(content) {
      if (released || !map) return;
      strip();
      for (const [id, data] of Object.entries(content.sources)) {
        map.addSource(id, { type: "geojson", data: data as GeoJSONSourceData });
        ownedSources.push(id);
      }
      for (const layer of content.layers) {
        map.addLayer(layer as AddLayerObject);
        ownedLayers.push(layer.id);
      }
      bind(content);
    },

    refresh(sources) {
      if (released || !map) return;
      for (const [id, data] of Object.entries(sources)) {
        (map.getSource(id) as GeoJSONSource | undefined)?.setData(data as GeoJSONSourceData);
      }
    },

    fit(sources, { maxZoom = 10 } = {}) {
      if (released || !map || !library) return false;
      const bounds = new library.LngLatBounds();
      let seen = 0;
      for (const collection of Object.values(sources)) {
        for (const feature of collection.features) {
          const { type, coordinates } = feature.geometry;
          const positions =
            type === "Point" ? [coordinates as [number, number]] : (coordinates as [number, number][]);
          for (const position of positions) {
            bounds.extend(position);
            seen += 1;
          }
        }
      }
      if (seen === 0) return false;
      map.fitBounds(bounds as LngLatBoundsLike, { padding: 64, maxZoom, duration: 500 });
      return true;
    },

    flyTo(target, zoom = 9) {
      if (!released) map?.flyTo({ center: target as LngLatLike, zoom, duration: 900 });
    },

    resize() {
      if (!released) map?.resize();
    },

    release() {
      if (released || !map) return;
      released = true;
      strip();
      // Detached, not destroyed. The WebGL context, the parsed style, the decoded sprite and
      // the uploaded tiles all survive for whoever leases next.
      map.getContainer().remove();
      parked.push(map);
      map = undefined;
    },
  };

  return leaseHandle;
}

/** GeoJSON as MapLibre's own setData accepts it, without importing the whole geojson type. */
type GeoJSONSourceData = Parameters<GeoJSONSource["setData"]>[0];

/**
 * How many instances are parked. Nothing in the shell reads it; it exists so the pool's one
 * real claim -- that a route change reuses an instance instead of building one -- can be
 * checked from a console against the running app, which is the only place it is observable.
 */
export function parkedCount(): number {
  return parked.length;
}
