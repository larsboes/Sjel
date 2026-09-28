"use strict";
/*
 * Load an Obsidian plugin's main.js the way Obsidian loads it.
 *
 * Obsidian reads exactly `main.js` and evaluates it as
 * `(function anonymous(require, module, exports) { ...source... })`, calling it with a
 * `require` that answers for `obsidian` and for node packages and for nothing relative,
 * then takes `module.exports.default || module.exports` as the plugin class. This
 * reproduces that shape, so the test and the probe exercise the file Obsidian will
 * actually run rather than a bundled or transpiled cousin of it — and a main.js Obsidian
 * could not load fails here first.
 *
 * The `obsidian` module is a stub the caller supplies, and every other id is refused by
 * name. That refusal is deliberate rather than restrictive: a plugin declaring
 * `isDesktopOnly: false` may require nothing but `obsidian`, because node packages do not
 * exist on mobile. A harness that quietly handed out the real `node:fs` would let that
 * mistake load here and fail on a phone.
 */

const fs = require("node:fs");
const path = require("node:path");

/**
 * @param {string} pluginDir directory holding manifest.json and main.js
 * @param {object} obsidianStub what `require("obsidian")` answers inside the plugin
 * @returns {{exports: any, manifest: any, source: string}}
 */
function loadPlugin(pluginDir, obsidianStub) {
  const source = fs.readFileSync(path.join(pluginDir, "main.js"), "utf8");
  const manifest = JSON.parse(fs.readFileSync(path.join(pluginDir, "manifest.json"), "utf8"));
  const shim = (id) => {
    if (id === "obsidian") return obsidianStub;
    throw new Error(`main.js required "${id}"; a plugin with isDesktopOnly: false may require only "obsidian"`);
  };
  const module = { exports: {} };
  // eslint-disable-next-line no-new-func -- this is the loader's whole job, and it is the
  // same construction Obsidian performs on the same bytes.
  const factory = new Function("require", "module", "exports", source);
  factory(shim, module, module.exports);
  const entry = module.exports.default || module.exports;
  if (typeof entry !== "function") {
    throw new Error(`${pluginDir}/main.js exports no plugin class — Obsidian would refuse it`);
  }
  return { exports: module.exports, manifest, source };
}

/**
 * The smallest `obsidian` a plugin that only renders and reads can be loaded against.
 *
 * The base classes are `Object` rather than fakes with behaviour: the view and the setting
 * tab are not exercised here (they need a real workspace), and a fake that pretends
 * otherwise would report a pass for code nobody ran.
 */
function minimalObsidian(overrides) {
  return Object.assign(
    {
      ItemView: class {},
      Notice: class {},
      Plugin: class {},
      PluginSettingTab: class {},
      Setting: class {},
      requestUrl: () => {
        throw new Error("requestUrl was called but this stub does not provide one");
      },
      setIcon: () => {},
    },
    overrides || {}
  );
}

module.exports = { loadPlugin, minimalObsidian };
