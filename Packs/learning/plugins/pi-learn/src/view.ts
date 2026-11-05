import { Component, ItemView, MarkdownRenderer, Notice, TFile, WorkspaceLeaf, setIcon } from 'obsidian';
import type PiLearnPlugin from './main';
import { askCard, quizCard, reviewCard } from './cards';
import { QUESTIONS_PROTOCOL, THINKING_LEVELS, buildLaunch, splitModel } from './launch';
import { parsePath, pathGraph, progress, sittingPrompt, type Kind } from './path';
import { ModelPicker } from './picker';
import { PiRpc, modelKey, type PiEvent, type PiModel } from './rpc';

export const VIEW_TYPE = 'pi-learn';

/** Built-in pi tools whose `path` argument names a file they change. */
const WRITING_TOOLS = new Set(['write', 'edit']);


/** One assistant message being streamed: its text and the component its Markdown renders under. */
interface Live {
  el: HTMLElement;
  text: string;
  component: Component;
  timer: number | null;
}

const compact = (n: number) => (n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1e3 ? `${(n / 1e3).toFixed(1)}k` : String(n));

export class LearnView extends ItemView {
  private pi: PiRpc | null = null;
  private streaming = false;
  private kind: Kind = 'teach';
  private live: Live | null = null;
  private components: Component[] = [];
  private pathFile: TFile | null = null;
  /** The open note when it is not a path note: the concept a sitting starts from. */
  private focusFile: TFile | null = null;
  private planOpen = false;
  private models: PiModel[] = [];

  /** Not `titleEl`: ItemView owns that name, and a field declaration resets it. */
  private pathTitleEl!: HTMLElement;
  private badgeEl!: HTMLElement;
  private dotsEl!: HTMLElement;
  private planEl!: HTMLElement;
  private planBtn!: HTMLElement;
  private statusEl!: HTMLElement;
  private streamEl!: HTMLElement;
  private input!: HTMLTextAreaElement;
  private modelBtn!: HTMLButtonElement;
  private thinkSel!: HTMLSelectElement;
  private meterEl!: HTMLElement;

  constructor(leaf: WorkspaceLeaf, private readonly plugin: PiLearnPlugin) {
    super(leaf);
  }

  getViewType() {
    return VIEW_TYPE;
  }
  getDisplayText() {
    return 'Learn';
  }
  getIcon() {
    return 'graduation-cap';
  }

  async onOpen() {
    const root = this.contentEl;
    root.empty();
    root.addClass('pi-learn');

    // Head: the open path, the mode, and the plan.
    const head = root.createDiv({ cls: 'pi-learn-head' });
    const top = head.createDiv({ cls: 'pi-learn-head-row' });
    this.pathTitleEl = top.createDiv({ cls: 'pi-learn-title' });
    this.planBtn = top.createEl('button', { cls: 'pi-learn-chip', attr: { 'aria-label': 'Show the path graph' } });
    setIcon(this.planBtn.createSpan({ cls: 'pi-learn-chip-icon' }), 'git-fork');
    this.planBtn.createSpan({ text: 'Plan' });
    this.planBtn.onclick = () => this.togglePlan();
    this.badgeEl = top.createDiv({ cls: 'pi-learn-badge' });
    this.dotsEl = head.createDiv({ cls: 'pi-learn-dots' });
    this.planEl = head.createDiv({ cls: 'pi-learn-plan' });
    this.planEl.hide();
    this.statusEl = root.createDiv({ cls: 'pi-learn-status' });
    this.statusEl.hide();

    this.streamEl = root.createDiv({ cls: 'pi-learn-stream' });
    this.emptyState();

    // Composer and footer.
    const dock = root.createDiv({ cls: 'pi-learn-dock' });
    const box = dock.createDiv({ cls: 'pi-learn-composer' });
    this.input = box.createEl('textarea', {
      cls: 'pi-learn-input',
      attr: { rows: '1', placeholder: 'Answer, ask, or name a topic…' },
    });
    this.input.addEventListener('input', () => this.grow());
    this.input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
        e.preventDefault();
        void this.send();
      }
    });
    const send = box.createEl('button', { cls: 'pi-learn-send', attr: { 'aria-label': 'Send (Enter)' } });
    setIcon(send, 'arrow-up');
    send.onclick = () => (this.streaming && !this.input.value.trim() ? void this.abort() : void this.send());

    const foot = dock.createDiv({ cls: 'pi-learn-foot' });
    this.modelBtn = foot.createEl('button', { cls: 'pi-learn-chip pi-learn-model', text: 'model', attr: { 'aria-label': 'Choose the model' } });
    this.modelBtn.onclick = () =>
      new ModelPicker(this.app, this.models, (key) => void this.pickModel(key), false).open();
    this.thinkSel = foot.createEl('select', { cls: 'pi-learn-select dropdown', attr: { 'aria-label': 'Thinking' } });
    this.thinkSel.createEl('option', { text: 'thinking', value: '' });
    this.thinkSel.onchange = () => void this.pickThinking(this.thinkSel.value);
    this.meterEl = foot.createDiv({ cls: 'pi-learn-meter' });

    this.registerEvent(this.app.workspace.on('file-open', (f) => void this.trackPath(f)));
    this.registerEvent(
      this.app.vault.on('modify', (f) => {
        if (f instanceof TFile && f.path === this.pathFile?.path) void this.paintPath();
      }),
    );
    await this.trackPath(this.app.workspace.getActiveFile());
    await this.paintPath();
    this.paintBadge();
    void this.connect();
  }

  async onClose() {
    this.pi?.stop();
    for (const c of this.components) c.unload();
  }

  /* ───────────── learning path context ───────────── */

  private isPath(file: TFile): boolean {
    const headings = this.app.metadataCache.getFileCache(file)?.headings ?? [];
    return headings.some((h) => h.level === 2 && h.heading === 'Learning Path');
  }

  /**
   * The context follows the open note. A path note is the path. Any other note is the focus, and
   * its path is a learning path that links to it or that it links to; it has none when no path
   * does. Notes outside Markdown leave the context as it is.
   */
  private async trackPath(file: TFile | null) {
    if (!file || file.extension !== 'md') return;
    if (this.isPath(file)) {
      this.pathFile = file;
      this.focusFile = null;
    } else {
      const links = this.app.metadataCache.resolvedLinks;
      const paths = this.app.vault.getMarkdownFiles().filter((f) => f !== file && this.isPath(f));
      this.focusFile = file;
      this.pathFile =
        paths.find((p) => links[p.path]?.[file.path]) ?? paths.find((p) => links[file.path]?.[p.path]) ?? null;
    }
    await this.paintPath();
  }

  private async paintPath() {
    this.pathTitleEl.empty();
    this.dotsEl.empty();
    this.planBtn.toggle(this.pathFile !== null);
    const focus = this.focusFile;
    if (!this.pathFile) {
      const name = this.pathTitleEl.createDiv({
        cls: `pi-learn-title-name${focus ? '' : ' is-none'}`,
        text: focus?.basename ?? 'No learning path',
      });
      if (focus) name.onclick = () => void this.openInMain(focus);
      this.pathTitleEl.createDiv({
        cls: 'pi-learn-title-sub',
        text: focus ? 'Not on a learning path · Teach starts from this note' : 'Open a note or a path, or type a topic',
      });
      return;
    }
    const file = this.pathFile;
    const name = this.pathTitleEl.createDiv({
      cls: 'pi-learn-title-name',
      text: file.basename.replace(/\s+—\s+Learning Path$/, ''),
    });
    name.onclick = () => void this.openInMain(file);
    const markdown = await this.app.vault.cachedRead(file);
    const path = parsePath(markdown);
    if (path) {
      const p = progress(path);
      this.pathTitleEl.createDiv({
        cls: 'pi-learn-title-sub',
        text: `${p.held} of ${path.nodes.length} held${focus ? ` · at ${focus.basename}` : p.current ? ' · resuming' : ''}`,
      });
      for (const n of path.nodes) {
        this.dotsEl.createSpan({ cls: `pi-learn-dot is-${n.state}`, attr: { 'aria-label': n.label } });
      }
    }
    if (this.planOpen) await this.paintPlan(markdown);
  }

  private async togglePlan() {
    this.planOpen = !this.planOpen;
    this.planBtn.toggleClass('is-active', this.planOpen);
    this.planEl.toggle(this.planOpen);
    if (this.planOpen && this.pathFile) await this.paintPlan(await this.app.vault.cachedRead(this.pathFile));
  }

  private async paintPlan(markdown: string) {
    this.planEl.empty();
    const graph = pathGraph(markdown);
    if (!graph) return;
    await MarkdownRenderer.render(this.app, '```mermaid\n' + graph + '```', this.planEl, this.pathFile?.path ?? '', this);
  }

  /* ───────────── pi process, models, meter ───────────── */

  private vaultPath(): string {
    return (this.app.vault.adapter as unknown as { getBasePath(): string }).getBasePath();
  }

  private ensurePi(): PiRpc {
    if (this.pi?.running) return this.pi;
    const launch = buildLaunch(this.plugin.settings, this.vaultPath(), process.platform, process.env);
    this.pi = new PiRpc(
      launch,
      (e) => this.onEvent(e),
      (code, stderr) => {
        this.setStreaming(false);
        if (code !== 0) this.notice(`pi stopped (exit ${code ?? 'error'}). ${stderr.trim().split('\n').pop() ?? ''}`, true);
      },
    );
    this.pi.start();
    return this.pi;
  }

  /** Start pi, list its models, and show the current one. */
  private async connect() {
    const pi = this.ensurePi();
    try {
      const list = await pi.command({ type: 'get_available_models' });
      this.models = (list?.models ?? []).sort((a: PiModel, b: PiModel) => modelKey(a).localeCompare(modelKey(b)));
      await this.applyKind();
    } catch (err) {
      this.notice(`Could not reach pi: ${(err as Error).message}. Check Settings → Learn.`, true);
    }
  }

  /** Apply the model and thinking level remembered for the current kind, then show what pi uses. */
  private async applyKind() {
    const pi = this.ensurePi();
    const s = this.plugin.settings;
    const model = splitModel(this.kind === 'teach' ? s.teachModel : s.practiceModel);
    const level = this.kind === 'teach' ? s.teachThinking : s.practiceThinking;
    if (model) await pi.command({ type: 'set_model', ...model }).catch((e) => this.notice(e.message, true));
    if (level) await pi.command({ type: 'set_thinking_level', level }).catch(() => undefined);
    await this.paintModel();
  }

  private async paintModel() {
    const pi = this.ensurePi();
    const state = await pi.command({ type: 'get_state' }).catch(() => null);
    this.modelBtn.setText(state?.model ? modelKey(state.model) : 'model');
    const levels: string[] = (await pi.command({ type: 'get_available_thinking_levels' }).catch(() => null))?.levels ?? THINKING_LEVELS;
    this.thinkSel.empty();
    for (const l of levels) this.thinkSel.createEl('option', { text: `thinking ${l}`, value: l });
    if (state?.thinkingLevel) this.thinkSel.value = state.thinkingLevel;
    await this.paintMeter();
  }

  private async pickModel(key: string) {
    const model = splitModel(key);
    if (!model) return;
    await this.ensurePi().command({ type: 'set_model', ...model }).catch((e) => this.notice(e.message, true));
    if (this.kind === 'teach') this.plugin.settings.teachModel = key;
    else this.plugin.settings.practiceModel = key;
    await this.plugin.save();
    await this.paintModel();
  }

  private async pickThinking(level: string) {
    await this.ensurePi().command({ type: 'set_thinking_level', level }).catch((e) => this.notice(e.message, true));
    if (this.kind === 'teach') this.plugin.settings.teachThinking = level;
    else this.plugin.settings.practiceThinking = level;
    await this.plugin.save();
  }

  private async paintMeter() {
    const stats = await this.pi?.command({ type: 'get_session_stats' }).catch(() => null);
    this.meterEl.empty();
    if (!stats) return;
    const ctx = stats.contextUsage;
    const used = ctx?.tokens != null && ctx?.contextWindow ? `${compact(ctx.tokens)} / ${compact(ctx.contextWindow)}` : null;
    const cost = typeof stats.cost === 'number' && stats.cost > 0 ? `$${stats.cost.toFixed(4)}` : null;
    this.meterEl.setText([used, cost].filter(Boolean).join(' · '));
    this.meterEl.setAttr('aria-label', 'Context used · estimated API cost of this sitting');
  }

  /* ───────────── sittings and messages ───────────── */

  private async sitting(kind: Kind) {
    const topic = this.input.value.trim();
    const path = this.pathFile ? `[[${this.pathFile.basename}]]` : null;
    const focus = this.focusFile ? `[[${this.focusFile.basename}]]` : null;
    if (!path && !focus && !topic) {
      this.input.focus();
      this.notice('Open a note or a learning path, or type a topic first.');
      return;
    }
    const pi = this.ensurePi();
    try {
      if (this.streaming) await pi.command({ type: 'abort' });
      await pi.command({ type: 'new_session' });
      this.kind = kind;
      await this.applyKind();
    } catch (err) {
      this.notice(`Could not start the sitting: ${(err as Error).message}`, true);
      return;
    }
    this.streamEl.empty();
    this.paintBadge();
    const label = this.focusFile?.basename ?? this.pathFile?.basename.replace(/\s+—\s+Learning Path$/, '') ?? topic;
    this.divider(`${kind === 'teach' ? 'Teaching' : 'Practice'} · ${label}`);
    const extra = (path || focus) && topic ? ` ${topic}` : '';
    this.input.value = '';
    this.grow();
    await this.prompt(sittingPrompt(kind, path, focus, topic) + extra, null);
  }

  private async send() {
    const message = this.input.value.trim();
    if (!message) return;
    this.input.value = '';
    this.grow();
    await this.prompt(message, message);
  }

  /** Send a prompt; while a turn runs it becomes a steer. `shown` is what the user bubble says. */
  private async prompt(message: string, shown: string | null) {
    const pi = this.ensurePi();
    if (shown !== null) this.bubble(shown);
    try {
      await pi.command(
        this.streaming ? { type: 'prompt', message, streamingBehavior: 'steer' } : { type: 'prompt', message },
      );
      this.setStreaming(true);
    } catch (err) {
      this.notice(`pi rejected the message: ${(err as Error).message}`, true);
    }
  }

  private async abort() {
    if (!this.pi?.running) return;
    await this.pi.command({ type: 'abort' }).catch(() => undefined);
  }

  /* ───────────── events ───────────── */

  private onEvent(e: PiEvent) {
    switch (e.type) {
      case 'agent_start':
        this.setStreaming(true);
        break;
      case 'agent_settled':
        this.setStreaming(false);
        void this.paintMeter();
        break;
      case 'message_start':
        if (e.message?.role === 'assistant') this.startAssistant();
        break;
      case 'message_update': {
        const ev = e.assistantMessageEvent;
        if (ev?.type === 'text_delta' && this.live) {
          this.live.text += ev.delta;
          this.scheduleRender();
        } else if (ev?.type === 'thinking_start') {
          this.setStatus('Thinking');
        } else if (ev?.type === 'text_start') {
          this.setStatus('');
        }
        break;
      }
      case 'message_end':
        if (e.message?.role === 'assistant') this.endAssistant(e.message);
        break;
      case 'tool_execution_start':
        this.toolStarted(e.toolName, e.args ?? {});
        break;
      case 'tool_execution_end':
        if (e.isError) this.toolLine('alert-triangle', `${e.toolName} failed`, 'is-error');
        break;
      case 'extension_ui_request':
        void this.uiRequest(e);
        break;
      case 'auto_retry_start':
        this.setStatus('Retrying');
        break;
    }
  }

  private startAssistant() {
    this.setStatus('');
    const el = this.streamEl.createDiv({ cls: 'pi-learn-msg is-assistant markdown-rendered' });
    const component = new Component();
    component.load();
    this.components.push(component);
    this.live = { el, text: '', component, timer: null };
  }

  private scheduleRender() {
    const live = this.live;
    if (!live || live.timer !== null) return;
    live.timer = window.setTimeout(() => {
      live.timer = null;
      void this.renderLive(live);
    }, 120);
  }

  private async renderLive(live: Live) {
    const old = live.component;
    old.unload();
    live.component = new Component();
    live.component.load();
    this.components[this.components.indexOf(old)] = live.component;
    live.el.empty();
    await MarkdownRenderer.render(this.app, live.text, live.el, this.pathFile?.path ?? '', live.component);
    this.scrollDown();
  }

  private endAssistant(message: any) {
    const live = this.live;
    this.live = null;
    if (!live) return;
    if (live.timer !== null) window.clearTimeout(live.timer);
    const final = (message.content ?? [])
      .filter((c: any) => c.type === 'text')
      .map((c: any) => c.text)
      .join('\n\n');
    if (final) live.text = final;
    if (message.stopReason === 'error' && message.errorMessage) live.text += `\n\n> [!error] ${message.errorMessage}`;
    if (!live.text.trim()) {
      live.el.remove();
      return;
    }
    void this.renderLive(live);
  }

  private toolStarted(name: string, args: Record<string, any>) {
    if (name === 'quiz' || name === 'ask') return; // their card arrives as a dialog
    if (WRITING_TOOLS.has(name) && typeof args.path === 'string') {
      const rel = this.vaultRelative(args.path);
      this.toolLine(name === 'write' ? 'file-plus' : 'pencil', rel ?? args.path);
      if (rel && this.plugin.settings.followEdits) void this.follow(rel);
      return;
    }
    if (name === 'read' && typeof args.path === 'string') {
      this.toolLine('file-text', this.vaultRelative(args.path) ?? args.path, 'is-quiet');
      return;
    }
    this.toolLine('wrench', name, 'is-quiet');
  }

  private vaultRelative(path: string): string | null {
    const base = this.vaultPath().replace(/\\/g, '/').replace(/\/$/, '');
    const p = path.replace(/\\/g, '/');
    if (p.startsWith(base + '/')) return p.slice(base.length + 1);
    if (!p.startsWith('/') && !/^[A-Za-z]:\//.test(p)) return p.replace(/^\.\//, '');
    return null;
  }

  /** Show the note pi is writing in the main pane, once the write has landed. */
  private async follow(rel: string) {
    if (!rel.endsWith('.md')) return;
    for (let i = 0; i < 20; i++) {
      const f = this.app.vault.getAbstractFileByPath(rel);
      if (f instanceof TFile) return this.openInMain(f);
      await sleep(250);
    }
  }

  private async openInMain(file: TFile) {
    const existing = this.app.workspace.getLeavesOfType('markdown').find((l) => (l.view as any).file?.path === file.path);
    if (existing) {
      this.app.workspace.revealLeaf(existing);
      return;
    }
    const leaf = this.app.workspace.getMostRecentLeaf(this.app.workspace.rootSplit) ?? this.app.workspace.getLeaf('tab');
    await leaf.openFile(file, { active: false });
  }

  /* ───────────── extension dialogs ───────────── */

  private async uiRequest(e: PiEvent) {
    const pi = this.pi;
    if (!pi) return;
    const md = (el: HTMLElement, text: string) => {
      const component = new Component();
      component.load();
      this.components.push(component);
      void MarkdownRenderer.render(this.app, text, el, this.pathFile?.path ?? '', component);
    };

    if (e.method === 'editor' && typeof e.title === 'string' && e.title.startsWith(`${QUESTIONS_PROTOCOL} `)) {
      const widget = e.title.slice(QUESTIONS_PROTOCOL.length + 1);
      let params: any;
      try {
        params = JSON.parse(e.prefill ?? '');
      } catch {
        pi.respond(e.id, { cancelled: true });
        return;
      }
      const slot = this.streamEl.createDiv({ cls: 'pi-learn-slot' });
      this.setStatus('');
      this.scrollDown();
      const result =
        widget === 'quiz'
          ? await quizCard(slot, params, md)
          : widget === 'quiz-review'
            ? await reviewCard(slot, params, md)
            : widget === 'ask'
              ? await askCard(slot, params, md)
              : null;
      this.scrollDown();
      if (result === null) pi.respond(e.id, { cancelled: true });
      else pi.respond(e.id, { value: JSON.stringify(result) });
      return;
    }

    switch (e.method) {
      case 'select':
      case 'confirm':
      case 'input':
      case 'editor':
        this.genericDialog(e, pi);
        break;
      case 'notify':
        this.notice(e.message ?? '', e.notifyType === 'error');
        break;
      case 'setStatus':
        this.setStatus(e.statusText ?? '');
        break;
    }
  }

  /** A plain inline dialog for every other extension request. */
  private genericDialog(e: PiEvent, pi: PiRpc) {
    const card = this.streamEl.createDiv({ cls: 'pi-learn-card' });
    card.createDiv({ cls: 'pi-learn-card-label', text: e.title ?? '' });
    if (e.message) card.createDiv({ cls: 'pi-learn-context', text: e.message });
    const body = card.createDiv({ cls: 'pi-learn-card-body' });
    const done = (answer: { value?: string; confirmed?: boolean; cancelled?: true }, label: string) => {
      pi.respond(e.id, answer);
      card.addClass('is-done');
      body.empty();
      card.createDiv({ cls: 'pi-learn-card-summary', text: label });
    };
    if (e.method === 'select') {
      const list = body.createDiv({ cls: 'pi-learn-options' });
      for (const option of e.options ?? []) {
        list.createEl('button', { cls: 'pi-learn-option', text: option }).onclick = () => done({ value: option }, option);
      }
    } else if (e.method === 'confirm') {
      const actions = body.createDiv({ cls: 'pi-learn-actions' });
      actions.createEl('button', { cls: 'pi-learn-btn', text: 'No' }).onclick = () => done({ confirmed: false }, 'No');
      actions.createEl('button', { cls: 'pi-learn-btn mod-cta', text: 'Yes' }).onclick = () => done({ confirmed: true }, 'Yes');
    } else {
      const field = body.createEl('textarea', { cls: 'pi-learn-field', attr: { placeholder: e.placeholder ?? '' } });
      field.value = e.prefill ?? '';
      const actions = body.createDiv({ cls: 'pi-learn-actions' });
      actions.createEl('button', { cls: 'pi-learn-btn', text: 'Cancel' }).onclick = () => done({ cancelled: true }, 'Cancelled');
      actions.createEl('button', { cls: 'pi-learn-btn mod-cta', text: 'Send' }).onclick = () =>
        done({ value: field.value }, field.value.split('\n')[0] || '(empty)');
    }
    this.scrollDown();
  }

  /* ───────────── small pieces ───────────── */

  private emptyState() {
    const empty = this.streamEl.createDiv({ cls: 'pi-learn-empty' });
    const start = (kind: Kind, icon: string, title: string, sub: string) => {
      const b = empty.createEl('button', { cls: 'pi-learn-start' });
      setIcon(b.createSpan({ cls: 'pi-learn-start-icon' }), icon);
      const words = b.createSpan();
      words.createDiv({ cls: 'pi-learn-start-title', text: title });
      words.createDiv({ cls: 'pi-learn-start-sub', text: sub });
      b.onclick = () => void this.sitting(kind);
    };
    start('teach', 'book-open', 'Teach', 'Probe, plan, and learn the next step');
    start('practice', 'repeat', 'Practice', 'Problems on what you already hold');
    empty.createDiv({ cls: 'pi-learn-empty-note', text: 'Each sitting starts fresh. The notes carry what you learned.' });
  }

  private paintBadge() {
    this.badgeEl.empty();
    this.badgeEl.toggleClass('is-busy', this.streaming);
    this.badgeEl.createSpan({ cls: 'pi-learn-spinner' });
    this.badgeEl.createSpan({ text: this.kind === 'teach' ? 'Teaching' : 'Practice' });
  }

  private bubble(text: string) {
    this.streamEl.querySelector('.pi-learn-empty')?.remove();
    this.streamEl.createDiv({ cls: 'pi-learn-msg is-user', text });
    this.scrollDown();
  }

  private divider(label: string) {
    this.streamEl.createDiv({ cls: 'pi-learn-divider', text: label });
  }

  private toolLine(icon: string, label: string, cls = '') {
    const line = this.streamEl.createDiv({ cls: `pi-learn-tool ${cls}`.trim() });
    setIcon(line.createSpan({ cls: 'pi-learn-tool-icon' }), icon);
    line.createSpan({ text: label });
    this.scrollDown();
  }

  private grow() {
    this.input.style.height = 'auto';
    this.input.style.height = `${Math.min(this.input.scrollHeight, 240)}px`;
  }

  private setStreaming(on: boolean) {
    this.streaming = on;
    this.contentEl.toggleClass('is-streaming', on);
    this.badgeEl?.toggleClass('is-busy', on);
    if (!on) this.setStatus('');
  }

  private setStatus(text: string) {
    this.statusEl.setText(text);
    this.statusEl.toggle(Boolean(text));
  }

  private notice(text: string, error = false) {
    new Notice(error ? `Learn: ${text}` : text, error ? 8000 : 4000);
  }

  private scrollDown() {
    this.streamEl.scrollTop = this.streamEl.scrollHeight;
  }
}
