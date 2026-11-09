import { Component, MarkdownRenderChild, MarkdownRenderer, TFile } from 'obsidian';
import type PiLearnPlugin from './main';
import { learningPaths } from './catalog';
import { parsePath, progress, resumePoint, type Kind } from './path';

/** Live vault projection; no progress or planning state is copied into the home note. */
export class LearningHub extends MarkdownRenderChild {
  private timer: number | null = null;
  private revision = 0;
  private disposed = false;
  private history: Component | null = null;

  constructor(el: HTMLElement, private readonly plugin: PiLearnPlugin, private readonly sourcePath: string) {
    super(el);
  }

  onload() {
    const refresh = () => {
      if (this.timer !== null) window.clearTimeout(this.timer);
      this.timer = window.setTimeout(() => { this.timer = null; void this.render(); }, 150);
    };
    const { vault, metadataCache } = this.plugin.app;
    this.registerEvent(vault.on('create', refresh));
    this.registerEvent(vault.on('delete', refresh));
    this.registerEvent(vault.on('rename', refresh));
    this.registerEvent(vault.on('modify', refresh));
    this.registerEvent(metadataCache.on('changed', refresh));
    this.registerEvent(metadataCache.on('resolved', refresh));
    void this.render();
  }

  onunload() {
    this.disposed = true;
    this.revision++;
    if (this.timer !== null) window.clearTimeout(this.timer);
  }

  private action(el: HTMLElement, label: string, file: TFile | null, kind: Kind) {
    const button = el.createEl('button', { text: label });
    button.onclick = () => void this.plugin.prepare(file, kind);
    return button;
  }

  private link(el: HTMLElement, file: TFile, label = file.basename) {
    const link = el.createEl('a', { text: label, cls: 'internal-link', attr: { href: file.path, 'data-href': file.path } });
    link.onclick = (event) => { event.preventDefault(); void this.plugin.app.workspace.openLinkText(file.path, this.sourcePath); };
  }

  private async render() {
    const revision = ++this.revision;
    const { app } = this.plugin;
    try {
      const paths = await Promise.all(learningPaths(app).map(async (file) => {
        try { return { file, path: parsePath(await app.vault.cachedRead(file)), error: '' }; }
        catch (error) { return { file, path: null, error: (error as Error).message }; }
      }));
      const notes = app.vault.getMarkdownFiles().filter((f) => f.path.startsWith('Knowledge/'));
      const claims = notes.filter((f) => app.metadataCache.getFileCache(f)?.frontmatter?.type === 'claim').sort((a, b) => a.path.localeCompare(b.path));
      const log = app.vault.getAbstractFileByPath('Resources/Learning Log.md');
      const history = log instanceof TFile ? (await app.vault.cachedRead(log)).split('\n').filter((line) => /^- \d{4}-\d{2}-\d{2} ·/.test(line)).slice(-5).reverse().join('\n') : '';
      if (this.disposed || revision !== this.revision) return;
      if (this.history) { this.removeChild(this.history); this.history = null; }
      const el = this.containerEl;
      el.empty();
      el.addClass('learn-hub');
      const toolbar = el.createDiv({ cls: 'learn-hub-toolbar' });
      toolbar.createEl('p', { text: `${paths.length} paths · ${notes.filter((f) => app.metadataCache.getFileCache(f)?.frontmatter?.type === 'note').length} concepts · ${claims.length} claims` });
      this.action(toolbar, 'Plan a new topic', null, 'teach');
      el.createEl('h2', { text: 'Choose a learning path' });
      if (!paths.length) el.createEl('p', { text: 'No learning paths yet. Plan a new topic to probe the foundations and build a path.' });
      for (const { file, path, error } of paths) {
        const row = el.createDiv({ cls: 'learn-hub-path' });
        const title = row.createDiv({ cls: 'learn-hub-path-title' });
        this.link(title, file, file.basename.replace(/\s+—\s+Learning Path$/, ''));
        const summary = app.metadataCache.getFileCache(file)?.frontmatter?.summary;
        if (typeof summary === 'string') row.createEl('p', { cls: 'learn-hub-summary', text: summary });
        if (!path) {
          row.createEl('p', { text: error ? `Could not read path: ${error}` : 'Repair the Learning Path graph before resuming.' });
          continue;
        }
        const counts = progress(path), next = resumePoint(path);
        row.createEl('p', { cls: 'learn-hub-progress', text: `${counts.held} / ${path.nodes.length} held · ${next.node ? `${next.reason === 'current' ? 'Resume' : 'Next'}: ${next.node.label}` : next.reason === 'complete' ? 'Path complete' : next.reason === 'blocked' ? 'No ready step; check prerequisite cycle or states' : 'Check graph syntax or current nodes'}` });
        const actions = row.createDiv({ cls: 'learn-hub-actions' });
        const teach = this.action(actions, 'Plan teaching', file, 'teach');
        teach.disabled = !next.node;
        teach.title = next.node ? `Plan ${next.node.label}` : 'Open the path to inspect its graph; no next step can be selected';
        const practice = this.action(actions, 'Plan practice', file, 'practice');
        practice.disabled = !counts.held || !claims.length;
        practice.title = practice.disabled ? 'Teaching must produce held nodes and claim notes first' : 'Practice existing claims from held nodes';
      }
      const practice = el.createEl('details', { cls: 'learn-hub-practice' });
      practice.createEl('summary', { text: `Practice material · ${claims.length} claims` });
      if (!claims.length) practice.createEl('p', { text: 'Teach a concept first. Claim notes will appear here automatically.' });
      const list = practice.createEl('ul');
      for (const file of claims.slice(0, 10)) {
        const item = list.createEl('li');
        this.link(item, file);
        this.action(item, 'Plan practice', file, 'practice');
      }
      if (claims.length > 10) practice.createEl('p', { text: 'Open a concept or claim in the vault to practice beyond this preview.' });
      el.createEl('h2', { text: 'Recent sittings' });
      if (history) {
        this.history = this.addChild(new Component());
        await MarkdownRenderer.render(app, history, el.createDiv(), this.sourcePath, this.history);
      } else el.createEl('p', { text: 'No recorded sittings yet. Finish a teaching or practice sitting to save its handoff here.' });
    } catch (error) {
      if (this.disposed || revision !== this.revision) return;
      this.containerEl.empty();
      this.containerEl.createEl('p', { text: `Could not refresh learning hub: ${(error as Error).message}` });
      this.containerEl.createEl('button', { text: 'Retry' }).onclick = () => void this.render();
    }
  }
}
