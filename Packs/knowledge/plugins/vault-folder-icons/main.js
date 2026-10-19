"use strict";
/*
 * Vault Folder Icons — assigns a Lucide icon to a folder in the file explorer.
 *
 * Own code, no upstream, no build step: Obsidian loads CommonJS and provides
 * `obsidian` at runtime, so this file IS the plugin. Nothing to compile and no
 * dependency that can go unmaintained.
 *
 * Icon ids come from Obsidian's own registry via getIconIds(), so the picker
 * lists exactly what this Obsidian version can render — no bundled icon pack
 * to fall out of date.
 *
 * Replaces obsidian-icon-folder (florianwoelki/obsidian-iconize). That plugin's
 * assignments used its own `LiPascalCase` naming; migrate() converts them to
 * Obsidian's `lucide-kebab-case` ids on first load.
 */

const { Plugin, PluginSettingTab, Setting, FuzzySuggestModal, setIcon, getIconIds, TFolder, Notice } = require("obsidian");

const DEFAULTS = { icons: {} };

/** `LiBrainCircuit` (iconize) → `lucide-brain-circuit` (Obsidian). */
function fromIconize(name) {
  if (typeof name !== "string" || !name.startsWith("Li")) return null;
  const kebab = name
    .slice(2)
    .replace(/([a-z0-9])([A-Z])/g, "$1-$2")
    .replace(/([A-Z])([A-Z][a-z])/g, "$1-$2")
    .toLowerCase();
  return `lucide-${kebab}`;
}

class IconPicker extends FuzzySuggestModal {
  constructor(app, onPick) {
    super(app);
    this.onPick = onPick;
    this.setPlaceholder("Pick an icon — type to filter");
  }
  getItems() {
    return getIconIds();
  }
  getItemText(id) {
    return id.replace(/^lucide-/, "");
  }
  // Render the icon next to its name; a name-only list is unusable for picking icons.
  renderSuggestion(match, el) {
    el.addClass("mod-complex");
    const wrap = el.createDiv({ cls: "vfi-suggest" });
    const icon = wrap.createSpan();
    setIcon(icon, match.item);
    wrap.createSpan({ text: this.getItemText(match.item) });
    wrap.style.display = "flex";
    wrap.style.alignItems = "center";
    wrap.style.gap = "8px";
  }
  onChooseItem(id) {
    this.onPick(id);
  }
}

module.exports = class VaultFolderIcons extends Plugin {
  async onload() {
    this.settings = Object.assign({}, DEFAULTS, await this.loadData());
    await this.migrate();

    this.addSettingTab(new VaultFolderIconsSettings(this.app, this));

    // Right-click a folder → assign or clear. This is the whole interaction.
    this.registerEvent(
      this.app.workspace.on("file-menu", (menu, file) => {
        if (!(file instanceof TFolder)) return;
        menu.addItem((item) =>
          item
            .setTitle("Change folder icon")
            .setIcon("lucide-image")
            .onClick(() => {
              new IconPicker(this.app, async (id) => {
                this.settings.icons[file.path] = id;
                await this.saveSettings();
              }).open();
            })
        );
        if (this.settings.icons[file.path]) {
          menu.addItem((item) =>
            item
              .setTitle("Remove folder icon")
              .setIcon("lucide-x")
              .onClick(async () => {
                delete this.settings.icons[file.path];
                await this.saveSettings();
              })
          );
        }
      })
    );

    // A folder rename must carry its icon, or the assignment silently orphans.
    this.registerEvent(
      this.app.vault.on("rename", async (file, oldPath) => {
        const moved = {};
        let changed = false;
        for (const [path, icon] of Object.entries(this.settings.icons)) {
          if (path === oldPath) {
            moved[file.path] = icon;
            changed = true;
          } else if (path.startsWith(oldPath + "/")) {
            moved[file.path + path.slice(oldPath.length)] = icon;
            changed = true;
          } else {
            moved[path] = icon;
          }
        }
        if (changed) {
          this.settings.icons = moved;
          await this.saveSettings();
        }
      })
    );

    this.registerEvent(
      this.app.vault.on("delete", async (file) => {
        if (this.settings.icons[file.path]) {
          delete this.settings.icons[file.path];
          await this.saveSettings();
        }
      })
    );

    this.app.workspace.onLayoutReady(() => {
      this.paint();
      this.watch();
    });
  }

  onunload() {
    this.observer?.disconnect();
    for (const el of document.querySelectorAll(".vfi-icon")) el.remove();
  }

  /** One-time import of obsidian-icon-folder's assignments. */
  async migrate() {
    if (this.settings.migratedFromIconize) return;
    let imported = 0;
    for (const [path, value] of Object.entries(this.settings.icons)) {
      const converted = fromIconize(value);
      if (converted) {
        this.settings.icons[path] = converted;
        imported++;
      }
    }
    this.settings.migratedFromIconize = true;
    await this.saveData(this.settings);
    if (imported) new Notice(`Vault Folder Icons: migrated ${imported} icons from Iconize`);
  }

  async saveSettings() {
    await this.saveData(this.settings);
    this.paint();
  }

  /** The explorer virtualises rows, so repaint whenever its subtree changes. */
  watch() {
    const explorer = document.querySelector(".nav-files-container");
    if (!explorer) return;
    this.observer = new MutationObserver(() => this.paint());
    this.observer.observe(explorer, { childList: true, subtree: true });
    this.register(() => this.observer.disconnect());
  }

  paint() {
    for (const title of document.querySelectorAll(".nav-folder-title")) {
      const path = title.getAttribute("data-path");
      const want = path ? this.settings.icons[path] : null;
      const existing = title.querySelector(":scope > .vfi-icon");
      if (!want) {
        existing?.remove();
        continue;
      }
      if (existing?.dataset.icon === want) continue;
      existing?.remove();
      const span = createSpan({ cls: "vfi-icon" });
      span.dataset.icon = want;
      setIcon(span, want);
      title.prepend(span);
    }
  }
};

class VaultFolderIconsSettings extends PluginSettingTab {
  constructor(app, plugin) {
    super(app, plugin);
    this.plugin = plugin;
  }
  display() {
    const { containerEl } = this;
    containerEl.empty();
    const entries = Object.entries(this.plugin.settings.icons).sort();
    if (!entries.length) {
      containerEl.createEl("p", {
        text: "No folder icons yet. Right-click any folder in the file explorer and choose “Change folder icon”.",
      });
      return;
    }
    for (const [path, icon] of entries) {
      const row = new Setting(containerEl).setName(path).setDesc(icon);
      const preview = row.nameEl.createSpan({ cls: "vfi-icon" });
      setIcon(preview, icon);
      row.nameEl.prepend(preview);
      row.addButton((b) =>
        b.setIcon("lucide-pencil").setTooltip("Change").onClick(() => {
          new IconPicker(this.app, async (id) => {
            this.plugin.settings.icons[path] = id;
            await this.plugin.saveSettings();
            this.display();
          }).open();
        })
      );
      row.addButton((b) =>
        b.setIcon("lucide-trash-2").setTooltip("Remove").setWarning().onClick(async () => {
          delete this.plugin.settings.icons[path];
          await this.plugin.saveSettings();
          this.display();
        })
      );
    }
  }
}
