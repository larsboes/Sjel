"use strict";
/*
 * Sjel Lens — a right-sidebar pane that says what Sjel knows about the open note.
 *
 * Own code, no upstream, no build step. Obsidian's plugin loader reads exactly `main.js`
 * and evaluates it as `(function anonymous(require, module, exports) { ... })` with a
 * `require` that resolves `obsidian` and node packages and nothing relative — so a second
 * source file would not be loaded at all. This file IS the plugin, and the tests load it
 * the same way the loader does.
 *
 * ## Read-only, and mechanically so
 *
 * Every request below is a GET. The plugin never writes a note, never writes frontmatter,
 * and registers no command that mutates anything. A pane that wrote what a capability
 * computes would be a writer nobody else can see. The people adapter left on 2026-10-20 with
 * the `Atlas/People` notes; `capabilities/entities` holds people now.
 *
 * ## Why requestUrl and never fetch
 *
 * `requestUrl` is not a browser request. Obsidian's renderer hands the call to the main
 * process over the `request-url` IPC channel, and the main process issues it with
 * Electron's `net.request`, setting only `Content-Type` and the headers the caller passed.
 * No Origin header is produced, and `origin_allowed_by(None, _)` returns true — the same
 * path curl, the service runner's health probes and every server-to-server caller take.
 *
 * A `fetch()` from this file would run in the renderer and carry `Origin:
 * app://obsidian.md`. The origin guard admits that exact origin
 * (`libs/sjel-server/src/origin.rs`, `origin_allowed_by`), so vault and trips would answer.
 * Discovery would still fail: sjel-status carries no CORS layer
 * (`capabilities/sjel-status/src/main.rs`, `build_router`), so the renderer withholds the
 * registry's reply from a cross-origin `fetch`. `requestUrl` depends on neither the origin
 * allowance nor a CORS header, which is why `tools/check-obsidian-plugins.sh` refuses this
 * file if a browser-context request appears in its code.
 *
 * ## Why there is one address in here and not eleven
 *
 * A capability's port is a machine fact: `[capability.<name>] port` in the overlay
 * overrides the manifest. Eleven port literals in a plugin are eleven things that go stale
 * silently. So this reads the registry `sjel-status` already serves
 * (`capabilities/sjel-status/src/main.rs`, `/api/sjel-status/capabilities`), which is the
 * same shape `tools/capability.sh registry` prints, and takes each capability's address
 * from the `health_url` the registry computed. One bootstrap address is left, it is a
 * setting, and it is the only one.
 */

const { ItemView, Notice, Plugin, PluginSettingTab, Setting, requestUrl, setIcon } = require("obsidian");

const VIEW_TYPE = "sjel-lens";

const DEFAULTS = {
  // The one address that cannot be discovered, because discovery starts here.
  registryUrl: "http://127.0.0.1:8082/api/sjel-status/capabilities",
  // Measured 2026-09-09, /api/plans answers in 0.01 s. Four seconds
  // is far past both, and past it the pane says so rather than hanging.
  timeoutMs: 4000,
  // The registry moves when a service starts or stops, not per keystroke.
  registryTtlMs: 60000,
};

/* ------------------------------------------------------------------ discovery */

/**
 * The origin a capability answers on, taken from its registry row.
 *
 * `health_url` is the registry's own composition of host, port and health path, so
 * removing the health path leaves the base with no port literal anywhere in this file.
 * Returns null for a row that serves nothing — `kind = "data"` capabilities such as
 * knowledge-base and store carry no port and no health_url at all.
 */
function capabilityOrigin(entry) {
  if (!entry || typeof entry.health_url !== "string" || entry.health_url === "") return null;
  const healthPath = typeof entry.health_path === "string" ? entry.health_path : "";
  let base = entry.health_url;
  if (healthPath !== "" && base.endsWith(healthPath)) {
    base = base.slice(0, base.length - healthPath.length);
  }
  base = base.replace(/\/+$/, "");
  return base === "" ? null : base;
}

/**
 * Where one named capability is, or why it cannot be reached.
 *
 * The three refusals are distinct on purpose: absent from the registry (the capability is
 * not enabled on this machine), no HTTP surface (it is a data or scheduled capability),
 * and not running. The pane prints whichever it got.
 */
function resolveCapability(registry, name) {
  const rows = Array.isArray(registry) ? registry : [];
  const entry = rows.find((row) => row && row.name === name);
  if (!entry) return { ok: false, reason: `${name} is not enabled on this machine` };
  const origin = capabilityOrigin(entry);
  if (!origin) return { ok: false, reason: `${name} serves no HTTP surface` };
  if (entry.up === false) return { ok: false, reason: `${name} is not running` };
  return { ok: true, origin };
}

/* --------------------------------------------------------- note kind → adapter */

/**
 * One entry per note kind this plugin can say anything about. The folder is the whole
 * routing rule: a note outside every folder here gets a pane that says so, not an error.
 */
const ADAPTERS = [
  { id: "trips", label: "Trip projection", folder: "Resources/Sjel/Trips/", capability: "trips" },
];

function adapterFor(path) {
  if (typeof path !== "string" || !path.endsWith(".md")) return null;
  return ADAPTERS.find((adapter) => path.startsWith(adapter.folder)) || null;
}

/* ------------------------------------------------------------------- shared */

/** A value from either side of the comparison, as one printable string or null. */
function printable(value) {
  if (value === null || value === undefined || value === "") return null;
  return String(value);
}

/* --------------------------------------------------------------- trips adapter */

/** Unix seconds from the string form both sides of the comparison use, or null. */
function toEpochSeconds(value) {
  if (typeof value === "number" && Number.isFinite(value)) return Math.trunc(value);
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  if (!/^\d+$/.test(trimmed)) return null;
  return Number.parseInt(trimmed, 10);
}

/**
 * Is the projection in front of you current?
 *
 * `capabilities/trips/src/projection.rs` writes `axon_revision` as the plan's `updated_at`
 * — deliberately the same token the store already uses for optimistic concurrency — so the
 * comparison is exact and needs no tolerance.
 *
 * `ahead` should be impossible and is reported rather than hidden: a note whose revision is
 * newer than the row it came from means somebody edited the projection, which the export
 * overwrites whole on its next run.
 */
function tripStaleness(frontmatter, plans) {
  const matter = frontmatter && typeof frontmatter === "object" ? frontmatter : {};
  const tripId = printable(matter.axon_trip_id);
  if (!tripId) return { state: "not-a-projection" };
  const rows = Array.isArray(plans) ? plans : [];
  const plan = rows.find((row) => row && row.id === tripId);
  if (!plan) return { state: "orphaned", tripId };
  const noteRevision = toEpochSeconds(matter.axon_revision);
  const planRevision = toEpochSeconds(plan.updated_at);
  if (noteRevision === null || planRevision === null) {
    return { state: "unreadable", tripId, title: printable(plan.title) };
  }
  const common = { tripId, title: printable(plan.title), revision: planRevision };
  if (noteRevision === planRevision) return { state: "current", ...common };
  if (noteRevision < planRevision) {
    return { state: "stale", behindSeconds: planRevision - noteRevision, ...common };
  }
  return { state: "ahead", aheadSeconds: noteRevision - planRevision, ...common };
}

/** A duration a person reads, from whole seconds. Coarse on purpose. */
function humanDuration(seconds) {
  const total = Math.max(0, Math.trunc(seconds));
  if (total < 60) return `${total} second${total === 1 ? "" : "s"}`;
  const minutes = Math.floor(total / 60);
  if (minutes < 60) return `${minutes} minute${minutes === 1 ? "" : "s"}`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) return `${hours} hour${hours === 1 ? "" : "s"}`;
  const days = Math.floor(hours / 24);
  return `${days} day${days === 1 ? "" : "s"}`;
}

/** The badge: one word of verdict and one sentence of consequence. */
function stalenessLabel(verdict) {
  switch (verdict && verdict.state) {
    case "current":
      return { badge: "current", detail: "This note matches the plan it was exported from." };
    case "stale":
      return {
        badge: "stale",
        detail: `The plan changed ${humanDuration(verdict.behindSeconds)} after this note was written. Re-export to catch up.`,
      };
    case "ahead":
      return {
        badge: "ahead",
        detail: `This note is ${humanDuration(verdict.aheadSeconds)} newer than the plan. An edit here is lost on the next export.`,
      };
    case "orphaned":
      return { badge: "orphaned", detail: "No plan carries this trip id. The plan was deleted or the id was edited." };
    case "unreadable":
      return { badge: "unreadable", detail: "axon_revision is not a revision this can compare." };
    default:
      return { badge: "not a projection", detail: "This note carries no axon_trip_id, so Sjel did not write it." };
  }
}

/* ------------------------------------------------------------------- transport */

/**
 * One GET, as JSON, with a deadline.
 *
 * `requestUrl` has no timeout of its own and Obsidian keeps the underlying request going
 * after the race is lost; what the deadline buys is a pane that degrades on schedule
 * instead of an empty pane that never resolves.
 *
 * `throw: false` is asked for explicitly so a 4xx arrives as a status to report rather than
 * an exception with the status buried in its message.
 */
function readJson(url, timeoutMs) {
  let timer = null;
  const request = requestUrl({ url, method: "GET", throw: false }).then((response) => {
    if (response.status < 200 || response.status >= 300) {
      throw new Error(`${url} answered ${response.status}`);
    }
    return response.json;
  });
  const deadline = new Promise((_resolve, reject) => {
    timer = setTimeout(() => reject(new Error(`${url} did not answer within ${timeoutMs} ms`)), timeoutMs);
  });
  // Cleared on both paths. A timer left armed per render is a leak in Obsidian and keeps
  // the probe's node process alive after it has printed its answer.
  return Promise.race([request, deadline]).finally(() => {
    if (timer !== null) clearTimeout(timer);
  });
}

/* ------------------------------------------------------------------------ view */

class SjelLensView extends ItemView {
  constructor(leaf, plugin) {
    super(leaf);
    this.plugin = plugin;
    // Every render bumps this. A slow answer for the note you just left must not paint
    // over the note you are now on.
    this.generation = 0;
  }

  getViewType() {
    return VIEW_TYPE;
  }

  getDisplayText() {
    return "Sjel Lens";
  }

  getIcon() {
    return "lucide-scan-eye";
  }

  async onOpen() {
    this.contentEl.addClass("sjel-lens");
    this.registerEvent(this.app.workspace.on("active-leaf-change", () => this.render()));
    this.registerEvent(this.app.workspace.on("file-open", () => this.render()));
    // Frontmatter is what the trips adapter compares, so a save that changes it must
    // change the badge.
    this.registerEvent(
      this.app.metadataCache.on("changed", (file) => {
        if (file && this.app.workspace.getActiveFile() === file) this.render();
      })
    );
    await this.render();
  }

  async render() {
    const generation = ++this.generation;
    const root = this.contentEl;
    root.empty();

    const file = this.app.workspace.getActiveFile();
    if (!file) {
      root.createEl("p", { cls: "sjel-lens-quiet", text: "No note is open." });
      return;
    }

    root.createEl("div", { cls: "sjel-lens-path", text: file.path });

    const adapter = adapterFor(file.path);
    if (!adapter) {
      const folders = ADAPTERS.map((entry) => entry.folder).join(" and ");
      root.createEl("p", {
        cls: "sjel-lens-quiet",
        text: `Sjel knows nothing about this note. It reads ${folders}.`,
      });
      return;
    }

    const section = root.createDiv({ cls: "sjel-lens-section" });
    section.createEl("h4", { text: adapter.label });
    const body = section.createDiv();
    body.createEl("p", { cls: "sjel-lens-quiet", text: `Asking ${adapter.capability}…` });

    let outcome;
    try {
      outcome = await this.plugin.readAdapter(adapter, file);
    } catch (error) {
      outcome = { ok: false, reason: String((error && error.message) || error) };
    }
    if (generation !== this.generation) return;

    body.empty();
    if (!outcome.ok) {
      // Per route, never globally: one dead capability costs one line, and any other
      // adapter on the same note still renders.
      body.createEl("p", { cls: "sjel-lens-down", text: outcome.reason });
      return;
    }
    this.renderTrip(body, outcome.verdict);
  }

  renderTrip(parent, verdict) {
    const label = stalenessLabel(verdict);
    const badge = parent.createEl("div", { cls: `sjel-lens-badge sjel-lens-${verdict.state}` });
    badge.createSpan({ text: label.badge });
    parent.createEl("p", { text: label.detail });
    if (verdict.title) parent.createEl("p", { cls: "sjel-lens-quiet", text: verdict.title });
    if (verdict.tripId) parent.createEl("code", { cls: "sjel-lens-quiet", text: verdict.tripId });
  }
}

/* -------------------------------------------------------------------- settings */

class SjelLensSettingTab extends PluginSettingTab {
  constructor(app, plugin) {
    super(app, plugin);
    this.plugin = plugin;
  }

  display() {
    const { containerEl } = this;
    containerEl.empty();
    new Setting(containerEl)
      .setName("Registry address")
      .setDesc(
        "The one address that is not discovered. Every capability's port comes from the registry this serves, so nothing else here needs changing when a port moves."
      )
      .addText((text) =>
        text
          .setPlaceholder(DEFAULTS.registryUrl)
          .setValue(this.plugin.settings.registryUrl)
          .onChange(async (value) => {
            this.plugin.settings.registryUrl = value.trim() || DEFAULTS.registryUrl;
            this.plugin.registry = null;
            await this.plugin.saveData(this.plugin.settings);
          })
      );
  }
}

/* ---------------------------------------------------------------------- plugin */

class SjelLens extends Plugin {
  async onload() {
    this.settings = Object.assign({}, DEFAULTS, await this.loadData());
    this.registry = null;
    this.registryReadAt = 0;

    this.registerView(VIEW_TYPE, (leaf) => new SjelLensView(leaf, this));
    this.addSettingTab(new SjelLensSettingTab(this.app, this));
    this.addRibbonIcon("lucide-scan-eye", "Sjel Lens", () => this.reveal());
    this.addCommand({
      id: "open",
      name: "Open the pane",
      callback: () => this.reveal(),
    });
    this.addCommand({
      id: "refresh",
      name: "Ask the capabilities again",
      callback: () => {
        this.registry = null;
        const views = this.app.workspace.getLeavesOfType(VIEW_TYPE);
        if (views.length === 0) new Notice("Sjel Lens is not open.");
        for (const leaf of views) leaf.view.render();
      },
    });
  }

  onunload() {
    // Obsidian detaches the leaves this plugin registered; the view's own registerEvent
    // handlers come off with it.
  }

  async reveal() {
    const existing = this.app.workspace.getLeavesOfType(VIEW_TYPE);
    const leaf = existing.length > 0 ? existing[0] : this.app.workspace.getRightLeaf(false);
    if (!leaf) {
      new Notice("Sjel Lens: no right sidebar to open into.");
      return;
    }
    if (existing.length === 0) await leaf.setViewState({ type: VIEW_TYPE, active: true });
    this.app.workspace.revealLeaf(leaf);
  }

  /** The registry, cached for as long as a service's up/down answer is worth reusing. */
  async capabilities() {
    const now = Date.now();
    if (this.registry && now - this.registryReadAt < this.settings.registryTtlMs) {
      return this.registry;
    }
    const rows = await readJson(this.settings.registryUrl, this.settings.timeoutMs);
    this.registry = Array.isArray(rows) ? rows : [];
    this.registryReadAt = now;
    return this.registry;
  }

  /**
   * One adapter's answer for one file, or the reason there is none.
   *
   * Discovery failing is reported as itself. Reporting it as "vault is down" would be a
   * guess, and the two have different fixes.
   */
  async readAdapter(adapter, file) {
    let registry;
    try {
      registry = await this.capabilities();
    } catch (error) {
      return {
        ok: false,
        reason: `sjel-status did not answer, so nothing could be discovered: ${(error && error.message) || error}`,
      };
    }

    const found = resolveCapability(registry, adapter.capability);
    if (!found.ok) return { ok: false, reason: found.reason };

    try {
      const plans = await readJson(`${found.origin}/api/plans`, this.settings.timeoutMs);
      const frontmatter = (this.app.metadataCache.getFileCache(file) || {}).frontmatter;
      return { ok: true, verdict: tripStaleness(frontmatter, plans) };
    } catch (error) {
      return { ok: false, reason: `${adapter.capability}: ${(error && error.message) || error}` };
    }
  }
}

module.exports = SjelLens;

/*
 * The testable half, hung off the exported class.
 *
 * Obsidian's loader takes `module.exports.default || module.exports` as the plugin class
 * and ignores everything else on it, so this costs the plugin nothing and saves it a build
 * step: `sjel-lens.test.js` and `sjel-lens.probe.js` load this same file through the same
 * loader shape and reach the pure functions here.
 */
module.exports.lens = {
  ADAPTERS,
  DEFAULTS,
  VIEW_TYPE,
  adapterFor,
  capabilityOrigin,
  humanDuration,
  readJson,
  resolveCapability,
  stalenessLabel,
  toEpochSeconds,
  tripStaleness,
};
