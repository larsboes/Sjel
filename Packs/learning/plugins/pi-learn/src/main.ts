import { App, Platform, Plugin, PluginSettingTab, Setting } from 'obsidian';
import { THINKING_LEVELS, buildLaunch, type Launcher } from './launch';
import { ModelPicker } from './picker';
import { listModels, type PiModel } from './rpc';
import { LearnView, VIEW_TYPE } from './view';
import { LearningHub } from './hub';
import type { Kind } from './path';
import type { TFile } from 'obsidian';

export interface PiLearnSettings {
  piCommand: string;
  launcher: Launcher;
  wslDistro: string;
  /** `provider/model` for teach sittings; empty keeps pi's default model. */
  teachModel: string;
  /** `provider/model` for practice sittings; empty keeps pi's default model. */
  practiceModel: string;
  /** Thinking level for teach sittings; empty keeps pi's default. */
  teachThinking: string;
  /** Thinking level for practice sittings; empty keeps pi's default. */
  practiceThinking: string;
  /** Open the note pi writes in the main pane. */
  followEdits: boolean;
}

const DEFAULTS: PiLearnSettings = {
  piCommand: 'pi',
  launcher: 'native',
  wslDistro: '',
  teachModel: '',
  practiceModel: '',
  teachThinking: '',
  practiceThinking: '',
  followEdits: true,
};

export default class PiLearnPlugin extends Plugin {
  settings: PiLearnSettings = { ...DEFAULTS };

  async onload() {
    this.settings = { ...DEFAULTS, ...(await this.loadData()) };
    this.registerView(VIEW_TYPE, (leaf) => new LearnView(leaf, this));
    this.addRibbonIcon('graduation-cap', 'Open Learn', () => void this.open());
    this.addCommand({ id: 'open', name: 'Open Learn', callback: () => void this.open() });
    this.registerMarkdownCodeBlockProcessor('learning-hub', (_source, el, ctx) => {
      ctx.addChild(new LearningHub(el, this, ctx.sourcePath));
    });
    this.addSettingTab(new PiLearnSettingTab(this.app, this));
  }

  async prepare(file: TFile | null, kind: Kind) {
    const view = await this.open();
    if (view) await view.prepare(file, kind);
  }

  async open(): Promise<LearnView | null> {
    const existing = this.app.workspace.getLeavesOfType(VIEW_TYPE)[0];
    if (existing) {
      this.app.workspace.revealLeaf(existing);
      return existing.view as LearnView;
    }
    const leaf = this.app.workspace.getRightLeaf(false);
    if (!leaf) return null;
    await leaf.setViewState({ type: VIEW_TYPE, active: true });
    this.app.workspace.revealLeaf(leaf);
    return leaf.view as LearnView;
  }

  async save() {
    await this.saveData(this.settings);
  }
}

class PiLearnSettingTab extends PluginSettingTab {
  constructor(app: App, private readonly plugin: PiLearnPlugin) {
    super(app, plugin);
  }

  display() {
    const { containerEl } = this;
    const s = this.plugin.settings;
    containerEl.empty();

    new Setting(containerEl)
      .setName('pi command')
      .setDesc('The pi executable: a name on PATH or an absolute path. Takes effect on the next sitting.')
      .addText((t) =>
        t.setValue(s.piCommand).onChange(async (v) => {
          s.piCommand = v;
          await this.plugin.save();
        }),
      );

    if (Platform.isWin) {
      new Setting(containerEl)
        .setName('Run pi in')
        .setDesc('Windows: start pi natively, or inside WSL with the vault path translated by wslpath.')
        .addDropdown((d) =>
          d
            .addOption('native', 'Windows')
            .addOption('wsl', 'WSL')
            .setValue(s.launcher)
            .onChange(async (v) => {
              s.launcher = v as Launcher;
              await this.plugin.save();
            }),
        );
      new Setting(containerEl)
        .setName('WSL distribution')
        .setDesc('Empty uses the default distribution.')
        .addText((t) =>
          t.setValue(s.wslDistro).onChange(async (v) => {
            s.wslDistro = v;
            await this.plugin.save();
          }),
        );
    }

    void this.models(containerEl.createDiv());
    new Setting(containerEl)
      .setName('Follow edits')
      .setDesc('Open the note pi is writing in the main pane.')
      .addToggle((t) =>
        t.setValue(s.followEdits).onChange(async (v) => {
          s.followEdits = v;
          await this.plugin.save();
        }),
      );
  }

  /** Model and thinking pickers per sitting kind, filled from a short-lived pi. */
  private async models(el: HTMLElement) {
    const s = this.plugin.settings;
    el.createEl('h3', { text: 'Models' });
    const status = el.createDiv({ cls: 'setting-item-description', text: 'Loading models from pi…' });
    let models: PiModel[] = [];
    let current: string | null = null;
    try {
      const vault = (this.app.vault.adapter as unknown as { getBasePath(): string }).getBasePath();
      ({ models, current } = await listModels(buildLaunch(s, vault, process.platform, process.env)));
      status.setText(`${models.length} models from pi. pi's default is ${current ?? 'not set'}.`);
    } catch (err) {
      status.setText(`Could not list models: ${(err as Error).message}. Check the pi command above.`);
    }
    const kinds = [
      { label: 'Teach', model: 'teachModel', thinking: 'teachThinking' },
      { label: 'Practice', model: 'practiceModel', thinking: 'practiceThinking' },
    ] as const;
    for (const k of kinds) {
      new Setting(el)
        .setName(`${k.label} model`)
        .setDesc(`Used when a ${k.label.toLowerCase()} sitting starts. Also switchable in the Learn footer.`)
        .addButton((b) => {
          const label = () => b.setButtonText(s[k.model] || `pi default${current ? ` (${current})` : ''}`);
          label();
          b.setDisabled(models.length === 0).onClick(() =>
            new ModelPicker(this.app, models, async (key) => {
              s[k.model] = key;
              await this.plugin.save();
              label();
            }, true).open(),
          );
        })
        .addDropdown((d) => {
          d.addOption('', 'thinking: pi default');
          for (const level of THINKING_LEVELS) d.addOption(level, `thinking: ${level}`);
          d.setValue(s[k.thinking]).onChange(async (v) => {
            s[k.thinking] = v;
            await this.plugin.save();
          });
        });
    }
  }
}
