import { Component, ItemView, MarkdownRenderer, Modal, Notice, TFile, WorkspaceLeaf, setIcon } from 'obsidian';
import type PiLearnPlugin from './main';
import { askCard, quizCard, reviewCard } from './cards';
import { QUESTIONS_PROTOCOL, THINKING_LEVELS, buildLaunch, splitModel } from './launch';
import { parsePath, pathGraph, progress, resumePoint, sittingPrompt, type Kind, type SittingBrief } from './path';
import { curationNeighbors, isCuratableNote, isLearningNote, isLearningPath, learningPaths } from './catalog';
import { organizationIssues } from './organization';
import { ModelPicker, SessionPicker } from './picker';
import { PiRpc, modelKey, type LearnSession, type PiEvent, type PiModel } from './rpc';

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
const CLOSED_MARKER = '<!-- pi-learn:sitting-closed -->';
type DialogAnswer = { value?: string; confirmed?: boolean; cancelled?: true };
type PendingDialog = { controller: AbortController; pi: PiRpc; response?: Promise<void> };

export class LearnView extends ItemView {
  private pi: PiRpc | null = null;
  private streaming = false;
  private kind: Kind = 'teach';
  private live: Live | null = null;
  private components: Component[] = [];
  private pathFile: TFile | null = null;
  /** The open note when it is not a path note: the concept a sitting starts from. */
  private focusFile: TFile | null = null;
  private curateFile: TFile | null = null;
  private browsingFile: TFile | null = null;
  private plannedFocus: TFile | null = null;
  private planOpen = false;
  private paintSeq = 0;
  private models: PiModel[] = [];
  private sittingActive = false;
  private lastTopic = '';
  private transitioning = false;
  private closing = false;
  private closeConfirmed = false;
  private closeFailed = false;
  private stopping = false;
  private disposed = false;
  private stickToBottom = true;
  private dialogs = new Map<string, PendingDialog>();
  private resetModal: Modal | null = null;
  private activity: HTMLDetailsElement | null = null;
  private replyFailed = false;
  private sessions: LearnSession[] = [];
  private sessionFile: string | null = null;
  private sessionUnavailable = false;
  private sessionPicker: SessionPicker | null = null;

  /** Not `titleEl`: ItemView owns that name, and a field declaration resets it. */
  private pathTitleEl!: HTMLElement;
  private badgeEl!: HTMLElement;
  private dotsEl!: HTMLElement;
  private planEl!: HTMLElement;
  private scopeEl!: HTMLElement;
  private planBtn!: HTMLElement;
  private statusEl!: HTMLElement;
  private streamEl!: HTMLElement;
  private input!: HTMLTextAreaElement;
  private modelBtn!: HTMLButtonElement;
  private thinkSel!: HTMLSelectElement;
  private meterEl!: HTMLElement;
  private sendBtn!: HTMLButtonElement;
  private continueBtn!: HTMLButtonElement;
  private newBtn!: HTMLButtonElement;
  private historyBtn!: HTMLButtonElement;
  private teachBtn!: HTMLButtonElement;
  private practiceBtn!: HTMLButtonElement;
  private curateBtn!: HTMLButtonElement;
  private finishBtn!: HTMLButtonElement;
  private planner!: HTMLDetailsElement;
  private targetSel!: HTMLSelectElement;
  private outcomeInput!: HTMLInputElement;
  private minutesInput!: HTMLInputElement;
  private modeSel!: HTMLSelectElement;
  private startBtn!: HTMLButtonElement;

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
    this.disposed = false;
    this.sittingActive = false;
    this.closing = false;
    this.streaming = false;
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
    const controls = head.createDiv({ cls: 'pi-learn-sittings', attr: { 'aria-label': 'Sitting controls' } });
    this.teachBtn = controls.createEl('button', { cls: 'pi-learn-chip', text: 'Teach' });
    this.teachBtn.onclick = () => void this.startOrContinue('teach');
    this.practiceBtn = controls.createEl('button', { cls: 'pi-learn-chip', text: 'Practice' });
    this.practiceBtn.onclick = () => void this.startOrContinue('practice');
    this.curateBtn = controls.createEl('button', { cls: 'pi-learn-chip', text: 'Curate' });
    this.curateBtn.onclick = () => void this.startOrContinue('curate');
    this.finishBtn = controls.createEl('button', { cls: 'pi-learn-chip', text: 'Finish sitting' });
    this.finishBtn.onclick = () => void this.finishSitting();
    const sessions = head.createDiv({ cls: 'pi-learn-sittings', attr: { 'aria-label': 'Saved sessions' } });
    this.continueBtn = sessions.createEl('button', { cls: 'pi-learn-btn mod-cta', text: 'Continue' });
    this.continueBtn.onclick = () => void this.startOrContinue(this.kind);
    this.newBtn = sessions.createEl('button', { cls: 'pi-learn-btn', text: 'New session' });
    this.newBtn.onclick = () => void this.sitting(this.kind, true);
    this.historyBtn = sessions.createEl('button', { cls: 'pi-learn-btn', text: 'History' });
    this.historyBtn.onclick = () => void this.showHistory();
    this.scopeEl = head.createDiv({ cls: 'pi-learn-title-sub' });
    this.scopeEl.hide();
    this.planner = head.createEl('details', { cls: 'pi-learn-planner' });
    this.planner.createEl('summary', { text: 'Session planner' });
    const field = (label: string) => {
      const wrapper = this.planner.createEl('label');
      wrapper.createSpan({ text: label });
      return wrapper;
    };
    this.targetSel = field('Note, path or topic').createEl('select', { attr: { 'aria-label': 'Note, path or topic' } });
    this.refreshTargets();
    this.targetSel.onchange = () => void this.trackPath(this.app.workspace.getActiveFile());
    this.outcomeInput = field('Desired outcome').createEl('input', { attr: { type: 'text', placeholder: 'What should become possible?', 'aria-label': 'Desired outcome' } });
    this.minutesInput = field('Available minutes').createEl('input', { attr: { type: 'number', min: '1', step: '1', placeholder: 'Optional', 'aria-label': 'Available minutes' } });
    this.modeSel = field('Mode').createEl('select', { attr: { 'aria-label': 'Sitting mode' } });
    this.modeSel.createEl('option', { text: 'Teach', attr: { value: 'teach' } });
    this.modeSel.createEl('option', { text: 'Practice', attr: { value: 'practice' } });
    this.modeSel.createEl('option', { text: 'Curate', attr: { value: 'curate' } });
    this.modeSel.onchange = () => { this.kind = this.modeSel.value as Kind; void this.trackPath(this.app.workspace.getActiveFile()); this.paintControls(); };
    this.startBtn = this.planner.createEl('button', { text: 'Start planned sitting', cls: 'mod-cta' });
    this.startBtn.onclick = () => void this.sitting(this.modeSel.value as Kind);
    this.statusEl = root.createDiv({ cls: 'pi-learn-status', attr: { role: 'status' } });
    this.statusEl.hide();

    this.streamEl = root.createDiv({ cls: 'pi-learn-stream' });
    this.streamEl.addEventListener('scroll', () => {
      this.stickToBottom = this.streamEl.scrollHeight - this.streamEl.clientHeight - this.streamEl.scrollTop < 48;
    });
    this.emptyState();

    // Composer and footer.
    const dock = root.createDiv({ cls: 'pi-learn-dock' });
    const box = dock.createDiv({ cls: 'pi-learn-composer' });
    this.input = box.createEl('textarea', {
      cls: 'pi-learn-input',
      attr: { rows: '1', placeholder: 'Name a topic…', 'aria-label': 'Topic or message' },
    });
    this.input.addEventListener('input', () => { this.grow(); this.paintControls(); });
    this.input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
        e.preventDefault();
        void this.send();
      }
    });
    this.sendBtn = box.createEl('button', { cls: 'pi-learn-send' });
    this.sendBtn.onclick = () => (this.streaming && !this.input.value.trim() ? void this.abort() : void this.send());

    const foot = dock.createDiv({ cls: 'pi-learn-foot' });
    this.modelBtn = foot.createEl('button', { cls: 'pi-learn-chip pi-learn-model', text: 'model', attr: { 'aria-label': 'Choose the model' } });
    this.modelBtn.onclick = () => this.chooseModel();
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
    this.registerEvent(this.app.metadataCache.on('changed', () => {
      this.refreshTargets();
      if (!this.sittingActive) void this.trackPath(this.app.workspace.getActiveFile());
    }));
    this.registerEvent(this.app.vault.on('rename', (file, oldPath) => {
      if (this.targetSel.value === oldPath) {
        this.refreshTargets();
        this.targetSel.value = file.path;
      }
    }));
    await this.trackPath(this.app.workspace.getActiveFile());
    await this.paintPath();
    this.paintBadge();
    this.paintControls();
    void this.connect();
  }

  async onClose() {
    this.disposed = true;
    this.resetModal?.close();
    this.sessionPicker?.close();
    await this.cancelDialogs();
    this.pi?.stop();
    this.pi = null;
    this.clearTranscript();
    if (this.sittingActive) this.notice('Sitting stopped without finishing. Notes already written remain; resume from the learning path.');
  }

  private clearTranscript() {
    if (this.live?.timer !== null && this.live?.timer !== undefined) window.clearTimeout(this.live.timer);
    this.live = null;
    this.activity = null;
    this.replyFailed = false;
    for (const c of this.components) c.unload();
    this.components = [];
    this.streamEl.empty();
    this.stickToBottom = true;
  }

  /* ───────────── learning path context ───────────── */

  private refreshTargets() {
    const selected = this.targetSel.value;
    this.targetSel.empty();
    this.targetSel.createEl('option', { text: 'Follow the open note or path', attr: { value: '' } });
    this.targetSel.createEl('option', { text: 'New topic', attr: { value: '__new__' } });
    for (const file of learningPaths(this.app)) this.targetSel.createEl('option', { text: file.basename, attr: { value: file.path } });
    if (this.plannedFocus) this.targetSel.createEl('option', { text: this.plannedFocus.basename, attr: { value: this.plannedFocus.path } });
    this.targetSel.value = selected || '';
  }

  /** Stage a target without starting a model-backed sitting. */
  async prepare(file: TFile | null, kind: Kind) {
    if (this.sittingActive) {
      this.notice('Finish the current sitting before planning another.');
      return;
    }
    this.plannedFocus = file && (kind === 'curate' ? isCuratableNote(this.app, file) : isLearningNote(this.app, file)) ? file : null;
    this.refreshTargets();
    this.targetSel.value = file ? file.path : '__new__';
    this.lastTopic = '';
    this.kind = kind;
    this.modeSel.value = kind;
    this.input.value = '';
    await this.trackPath(file);
    this.planner.open = true;
    this.paintControls();
    (file ? this.outcomeInput : this.input).focus();
  }

  private async trackPath(file: TFile | null) {
    this.browsingFile = file;
    if (this.sittingActive || this.transitioning) { this.paintScope(); return; }
    if (this.targetSel.value === '__new__') file = null;
    else if (this.targetSel.value) {
      const target = this.app.vault.getAbstractFileByPath(this.targetSel.value);
      file = target instanceof TFile ? target : null;
    }
    this.pathFile = null;
    this.focusFile = null;
    this.curateFile = file && isCuratableNote(this.app, file) ? file : null;
    if (file?.extension === 'md' && isLearningPath(this.app, file)) this.pathFile = file;
    else if (file?.extension === 'md' && isLearningNote(this.app, file)) {
      const links = this.app.metadataCache.resolvedLinks;
      this.focusFile = file;
      this.pathFile = learningPaths(this.app).find((p) => links[p.path]?.[file!.path] || links[file!.path]?.[p.path]) ?? null;
    }
    await this.paintPath();
  }

  private paintScope() {
    if (!this.scopeEl) return;
    const curation = this.kind === 'curate';
    this.scopeEl.toggle(curation);
    if (!curation) return;
    const anchor = this.curateFile;
    this.scopeEl.setText(anchor
      ? `${this.sittingActive ? 'Fixed curation target' : 'Curation target'}: ${anchor.path}${this.sittingActive && this.browsingFile && this.browsingFile.path !== anchor.path ? ` · Viewing ${this.browsingFile.basename} does not change this target. Finish this sitting before curating another note.` : ''}`
      : 'Select an existing note to curate; a typed topic must resolve to one anchor.');
  }

  private async paintPath() {
    this.paintScope();
    const seq = ++this.paintSeq;
    this.pathTitleEl.empty();
    this.dotsEl.empty();
    this.planBtn.toggle(this.kind !== 'curate' && this.pathFile !== null);
    this.planEl.toggle(this.kind !== 'curate' && this.planOpen);
    if (this.kind === 'curate') {
      this.pathTitleEl.createDiv({ cls: 'pi-learn-title-name', text: this.curateFile?.basename ?? 'Choose a note to curate' });
      this.pathTitleEl.createDiv({ cls: 'pi-learn-title-sub', text: 'Slim, connect and organize · one note and its direct links' });
      return;
    }
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
    if (seq !== this.paintSeq || this.disposed) return;
    const path = parsePath(markdown);
    if (path) {
      const p = progress(path);
      const next = resumePoint(path);
      this.pathTitleEl.createDiv({
        cls: 'pi-learn-title-sub',
        text: `${p.held} of ${path.nodes.length} held${focus ? ` · at ${focus.basename}` : next.node ? ` · ${next.reason === 'current' ? 'resume' : 'next'}: ${next.node.label}` : ` · ${next.reason}`}`,
      });
      for (const n of path.nodes) {
        this.dotsEl.createSpan({ cls: `pi-learn-dot is-${n.state}`, attr: { 'aria-label': `${n.label}: ${n.state}`, title: `${n.label} · ${n.state}` } });
      }
    }
    if (this.planOpen) await this.paintPlan(markdown, seq);
  }

  private async togglePlan() {
    this.planOpen = !this.planOpen;
    this.planBtn.toggleClass('is-active', this.planOpen);
    this.planEl.toggle(this.planOpen);
    if (this.planOpen && this.pathFile) {
      const seq = this.paintSeq;
      await this.paintPlan(await this.app.vault.cachedRead(this.pathFile), seq);
    }
  }

  private async paintPlan(markdown: string, seq: number) {
    const graph = pathGraph(markdown);
    if (!graph || seq !== this.paintSeq || this.disposed) return;
    this.planEl.empty();
    await MarkdownRenderer.render(this.app, '```mermaid\n' + graph + '```', this.planEl, this.pathFile?.path ?? '', this);
  }

  /* ───────────── pi process, models, meter ───────────── */

  private vaultPath(): string {
    return (this.app.vault.adapter as unknown as { getBasePath(): string }).getBasePath();
  }

  private ensurePi(): PiRpc {
    if (this.disposed) throw new Error('Learn is closed');
    if (this.pi?.running) return this.pi;
    const launch = buildLaunch(this.plugin.settings, this.vaultPath(), process.platform, process.env);
    const pi = new PiRpc(
      launch,
      (e) => { if (this.pi === pi) this.onEvent(e); },
      (code, stderr) => {
        if (this.pi !== pi || this.disposed) return;
        this.cancelDialogs();
        this.closing = false;
        this.setStreaming(false);
        if (code !== 0) this.notice(`pi stopped (exit ${code ?? 'error'}). ${stderr.trim().split('\n').pop() ?? ''}`, true);
      },
    );
    this.pi = pi;
    pi.start();
    return pi;
  }

  /** Start pi, list its models, and show the current one. */
  private async connect() {
    const pi = this.ensurePi();
    try {
      const list = await pi.command({ type: 'get_available_models' });
      this.models = (list?.models ?? []).sort((a: PiModel, b: PiModel) => modelKey(a).localeCompare(modelKey(b)));
      await this.paintModel();
    } catch (err) {
      this.notice(`Could not reach pi: ${(err as Error).message}. Check Settings → Learn.`, true);
    }
  }

  /** Apply the model and thinking level remembered for the current kind, then show what pi uses. */
  private async applyKind() {
    const pi = this.ensurePi();
    const s = this.plugin.settings;
    const model = splitModel(s[`${this.kind}Model`] ?? '');
    const level = s[`${this.kind}Thinking`];
    if (model) await pi.command({ type: 'set_model', ...model }).catch((e) => this.notice(e.message, true));
    if (level) await pi.command({ type: 'set_thinking_level', level }).catch(() => undefined);
    await this.paintModel();
  }

  private async paintModel() {
    const pi = this.ensurePi();
    const state = await pi.command({ type: 'get_state' }).catch(() => null);
    this.modelBtn.setText(state?.model?.id ?? 'Choose model');
    this.modelBtn.setAttr('title', state?.model ? modelKey(state.model) : 'Choose the model');
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
    this.plugin.settings[`${this.kind}Model`] = key;
    await this.plugin.save();
    await this.paintModel();
  }

  private chooseModel() {
    new ModelPicker(this.app, this.models, (key) => void this.pickModel(key), false).open();
  }

  private async pickThinking(level: string) {
    await this.ensurePi().command({ type: 'set_thinking_level', level }).catch((e) => this.notice(e.message, true));
    this.plugin.settings[`${this.kind}Thinking`] = level;
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

  private target(kind: Kind): string {
    return (kind === 'curate' ? this.curateFile?.path : this.pathFile?.path ?? this.focusFile?.path)
      ?? ((!this.sittingActive ? this.input.value.trim() : '') || this.lastTopic);
  }

  private async savedSessions(): Promise<LearnSession[]> {
    const pi = this.ensurePi();
    const commands = await pi.command({ type: 'get_commands' });
    if (!commands?.commands?.some((command: { name: string }) => command.name === 'learn-sessions')) {
      throw new Error('Session discovery is not loaded. Reload Learn to load its extension');
    }
    this.sessions = [];
    await pi.command({ type: 'prompt', message: '/learn-sessions' });
    return this.sessions;
  }

  private async startOrContinue(kind: Kind) {
    if (this.transitioning || this.disposed || this.stopping) return;
    if (this.sittingActive && this.kind === kind) { this.input.focus(); return; }
    this.transitioning = true;
    this.paintControls();
    let latest: LearnSession | undefined;
    try {
      const target = this.target(kind);
      latest = (await this.savedSessions()).find((session) => session.kind === kind && session.target === target);
    } catch (err) {
      this.notice(`Could not read saved sessions: ${(err as Error).message}. New session is still available.`, true);
      return;
    } finally {
      this.transitioning = false;
      this.paintControls();
    }
    if (latest) await this.resumeSession(latest, this.sittingActive ? '' : this.input.value);
    else await this.sitting(kind);
  }

  private async showHistory() {
    if (this.transitioning || this.disposed || this.stopping || this.streaming) return;
    this.transitioning = true;
    this.paintControls();
    try {
      const sessions = await this.savedSessions();
      if (!sessions.length) { this.notice('No saved learning sessions yet. Choose New session.'); return; }
      this.sessionPicker = new SessionPicker(this.app, sessions, (session) => void this.resumeSession(session));
      this.sessionPicker.open();
    } catch (err) {
      this.notice(`Could not read saved sessions: ${(err as Error).message}`, true);
    } finally {
      this.transitioning = false;
      this.paintControls();
    }
  }

  private async resumeSession(session: LearnSession, draft = '') {
    if (this.transitioning || this.disposed || this.stopping) return;
    if (this.sittingActive && this.sessionFile === session.path) { this.input.focus(); return; }
    this.transitioning = true;
    this.paintControls();
    try {
      if ((this.streaming || this.dialogs.size) && !(await this.abort())) return;
      const pi = this.ensurePi();
      const result = await pi.command({ type: 'switch_session', sessionPath: session.path });
      if (result?.cancelled || this.disposed) return;
      this.sessionUnavailable = true;
      this.sittingActive = false;
      this.clearTranscript();
      this.sessionFile = session.path;
      this.kind = session.kind;
      this.modeSel.value = session.kind;
      this.lastTopic = session.target;
      this.pathFile = null;
      this.focusFile = null;
      this.curateFile = null;
      const file = this.app.vault.getAbstractFileByPath(session.target);
      if (file instanceof TFile) {
        this.curateFile = isCuratableNote(this.app, file) ? file : null;
        if (isLearningPath(this.app, file)) this.pathFile = file;
        else if (isLearningNote(this.app, file)) this.focusFile = file;
      }
      this.input.value = draft;
      await this.paintPath();
      const history = await pi.command({ type: 'get_messages' });
      if (this.disposed || this.pi !== pi) return;
      this.sittingActive = true;
      this.sessionUnavailable = false;
      this.closing = false;
      this.replyFailed = false;
      this.divider(`Continued · ${session.target} · ${session.kind}`);
      for (const message of history?.messages ?? []) this.restoreMessage(message);
      await this.paintModel();
      this.grow();
      this.setStreaming(false);
      this.input.focus();
    } catch (err) {
      if (this.sessionUnavailable && !this.disposed) {
        this.replyFailed = true;
        this.divider(`Could not load ${session.target}. Choose Continue or History to retry, or New session.`);
      }
      this.notice(`Could not continue the session: ${(err as Error).message}`, true);
    } finally {
      this.transitioning = false;
      this.paintControls();
    }
  }

  private restoreMessage(message: any) {
    const content = typeof message.content === 'string' ? message.content : (message.content ?? [])
      .filter((part: any) => part.type === 'text').map((part: any) => part.text).join('\n\n');
    if (message.role === 'user') {
      const shown = content.replace(/<skill\b[^>]*>[\s\S]*?<\/skill>\s*/g, '').trim();
      if (shown) this.bubble(shown);
    } else if (message.role === 'assistant') {
      this.startAssistant();
      this.endAssistant(message);
    } else if (message.role === 'toolResult' && (message.toolName === 'ask' || message.toolName === 'quiz')) {
      const receipt = this.streamEl.createEl('details', { cls: 'pi-learn-card-archive' });
      receipt.createEl('summary', { text: `Saved ${message.toolName} result · read-only` });
      const component = new Component();
      component.load();
      this.components.push(component);
      void MarkdownRenderer.render(this.app, content, receipt.createDiv(), this.pathFile?.path ?? '', component);
    }
  }

  private async sitting(kind: Kind, explicitNew = false) {
    if (this.transitioning || this.disposed || this.stopping) return;
    const topic = this.input.value.trim() || (!this.pathFile && !this.focusFile ? this.lastTopic : '');
    const path = kind !== 'curate' && this.pathFile ? `[[${this.pathFile.path.replace(/\.md$/, '')}]]` : null;
    const focusFile = kind === 'curate' ? this.curateFile : this.focusFile;
    const focus = focusFile ? `[[${focusFile.path.replace(/\.md$/, '')}]]` : null;
    if (!path && !focus && !topic) {
      this.input.focus();
      this.notice('Open a note or a learning path, or type a topic first.');
      return;
    }
    const minutes = this.minutesInput.value.trim();
    if (minutes && (!Number.isInteger(Number(minutes)) || Number(minutes) < 1)) {
      this.notice('Enter a positive whole number of minutes.');
      this.minutesInput.focus();
      return;
    }
    const brief: SittingBrief = { outcome: this.outcomeInput.value, minutes: minutes ? Number(minutes) : undefined };
    this.transitioning = true;
    this.paintControls();
    try {
      if (kind === 'curate' && focusFile) {
        const neighbors = curationNeighbors(this.app, focusFile);
        const scope = new Set([focusFile.path, ...neighbors.map((file) => file.path)]);
        brief.related = neighbors.map((file) => `[[${file.path.replace(/\.md$/, '')}]]`);
        brief.structure = organizationIssues(this.app).filter((issue) => scope.has(issue.file.path)).map((issue) => `${issue.file.path}: ${issue.message}`);
      }
      if (kind === 'teach' && this.pathFile && !this.focusFile) {
        const graph = parsePath(await this.app.vault.cachedRead(this.pathFile));
        const next = graph && resumePoint(graph);
        if (!next?.node) {
          this.notice(next?.reason === 'complete' ? 'This path is complete. Choose Practice or a new topic.' : 'No ready step. Open the path and check its prerequisites, syntax and current nodes.');
          return;
        }
        brief.next = `${next.node.id}: ${next.node.label}`;
      }
      if (this.sittingActive && !explicitNew && !(await this.confirmReset())) return;
      if (this.disposed) return;
      const pi = this.ensurePi();
      if ((this.streaming || this.dialogs.size) && !(await this.abort())) return;
      const result = await pi.command({ type: 'new_session' });
      if (result?.cancelled) {
        this.notice('pi kept the current sitting.');
        return;
      }
      this.cancelDialogs();
      this.sessionUnavailable = true;
      this.sittingActive = false;
      this.clearTranscript();
      this.sessionFile = null;
      this.kind = kind;
      this.modeSel.value = kind;
      const target = (kind === 'curate' ? focusFile?.path : this.pathFile?.path ?? focusFile?.path) ?? topic;
      await pi.command({ type: 'set_session_name', name: `Learn · ${kind} · ${target}` });
      await this.applyKind();
      await this.paintPath();
    } catch (err) {
      this.notice(`Could not start the sitting: ${(err as Error).message}`, true);
      return;
    } finally {
      this.transitioning = false;
      this.paintControls();
    }
    if (this.disposed) return;
    this.clearTranscript();
    this.sittingActive = true;
    this.sessionUnavailable = false;
    this.paintScope();
    this.planner.open = false;
    this.lastTopic = topic;
    this.closing = false;
    this.paintBadge();
    this.paintControls();
    const label = focusFile?.basename ?? this.pathFile?.basename.replace(/\s+—\s+Learning Path$/, '') ?? topic;
    this.divider(`${kind === 'teach' ? 'Teaching' : kind === 'curate' ? 'Curation' : 'Practice'} · ${label}`);
    this.input.value = '';
    this.grow();
    await this.prompt(sittingPrompt(kind, path, focus, topic, brief), null);
  }

  private async send() {
    if (this.transitioning || this.stopping || this.disposed || this.sessionUnavailable) return;
    if (!this.sittingActive) return this.startOrContinue(this.kind);
    const message = this.input.value.trim();
    if (!message) return;
    this.input.value = '';
    this.grow();
    await this.prompt(message, message);
  }

  /** Send a prompt; while a turn runs it becomes a steer. `shown` is what the user bubble says. */
  private async prompt(message: string, shown: string | null) {
    if (this.sessionUnavailable) return false;
    const pi = this.ensurePi();
    if (shown !== null) this.bubble(shown);
    const wasStreaming = this.streaming;
    this.replyFailed = false;
    this.activity = null;
    this.setStreaming(true);
    try {
      const result = await pi.command(
        wasStreaming ? { type: 'prompt', message, streamingBehavior: 'steer' } : { type: 'prompt', message },
      );
      if (result?.disposition === 'handled' && !wasStreaming) {
        this.closing = false;
        this.setStreaming(false);
      }
      return true;
    } catch (err) {
      this.closing = false;
      this.setStreaming(false);
      this.notice(`pi rejected the message: ${(err as Error).message}`, true);
      return false;
    }
  }

  private confirmReset(): Promise<boolean> {
    return new Promise((resolve) => {
      const modal = new Modal(this.app);
      let reset = false;
      modal.titleEl.setText('Start a new sitting?');
      modal.contentEl.createEl('p', { text: 'This sitting has not finished. Keep it and use Finish sitting to close it, or start fresh without completing its closing procedure. Notes already written remain.' });
      const actions = modal.contentEl.createDiv({ cls: 'pi-learn-actions' });
      const keep = actions.createEl('button', { text: 'Keep sitting', cls: 'mod-cta' });
      keep.onclick = () => modal.close();
      actions.createEl('button', { text: 'Start fresh' }).onclick = () => { reset = true; modal.close(); };
      modal.onClose = () => {
        if (this.resetModal === modal) this.resetModal = null;
        resolve(reset);
      };
      this.resetModal = modal;
      modal.open();
      keep.focus();
    });
  }

  private async finishSitting() {
    if (!this.sittingActive || this.closing || this.transitioning || this.stopping) return;
    this.transitioning = true;
    this.paintControls();
    try {
      if ((this.streaming || this.dialogs.size) && !(await this.abort())) return;
      if (this.disposed) return;
      this.closing = true;
      this.closeConfirmed = false;
      this.closeFailed = false;
      this.paintControls();
      await this.prompt(
        this.kind === 'curate'
          ? `Finish this curation sitting using the curate skill's Close procedure. Stop editing now, verify changes already made, and summarize changed files, preserved sources and unresolved proposals. Do not apply unapproved merges, deletions or restructuring. Do not change learning-path states, invent claims or append a teaching/practice session to the Learning Log. End with ${CLOSED_MARKER} only after the required checks and writes succeed; report failures without the marker.`
          : `Finish this ${this.kind} sitting using the active skill's Close procedure. Treat Finish as a request to stop teaching or posing problems now. Save an honest partial handoff when a gate, contest, problem or retest remains; list each unresolved item rather than requiring another question. Complete the required claim review, note and learning-path updates, and Learning Log entry that apply to this sitting. Do not invent mastery or record a completed session when the closing checks or writes fail. Summarize what was saved and the next step. Only after the closing procedure and required writes succeed, end the final reply with ${CLOSED_MARKER}. Unfinished learning may be recorded as pending and still close the sitting. A failed required write or a claim saved without required review is a closing failure: report it and do not emit that marker. Save already reviewed material and defer unreviewed claims when stopping.`,
        null,
      );
    } catch (err) {
      this.closing = false;
      this.notice(`Could not finish the sitting: ${(err as Error).message}`, true);
    } finally {
      this.transitioning = false;
      this.paintControls();
    }
  }

  private async abort(): Promise<boolean> {
    if (this.stopping) return false;
    let stopped = false;
    this.stopping = true;
    this.closing = false;
    this.closeConfirmed = false;
    this.paintControls();
    await this.cancelDialogs();
    try {
      if (this.pi?.running) {
        await this.pi.command({ type: 'clear_queue' });
        await this.pi.command({ type: 'abort' });
      }
      stopped = true;
    } catch (err) {
      this.notice(`Could not stop pi: ${(err as Error).message}`, true);
    } finally {
      this.stopping = false;
      if (stopped || !this.pi?.running) this.setStreaming(false);
      else this.paintControls();
    }
    return stopped;
  }

  /* ───────────── events ───────────── */

  private onEvent(e: PiEvent) {
    if (this.disposed) return;
    switch (e.type) {
      case 'agent_start':
        this.setStreaming(true);
        break;
      case 'agent_settled':
        if (this.closing) {
          if (this.closeConfirmed && !this.closeFailed && this.dialogs.size === 0) {
            this.sittingActive = false;
            this.divider('Sitting closed · choose Teach or Practice to start fresh');
          }
          this.closing = false;
        }
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
        if (e.isError) {
          if (this.closing) this.closeFailed = true;
          this.toolLine('alert-triangle', `${e.toolName} failed`, 'is-error');
        }
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
    const component = live.component;
    await MarkdownRenderer.render(this.app, live.text, live.el, this.pathFile?.path ?? '', component);
    if (this.disposed || !this.components.includes(component)) component.unload();
    else this.scrollDown();
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
    if (this.closing) {
      if (message.stopReason === 'error' || message.stopReason === 'aborted') this.closeFailed = true;
      if (message.stopReason === 'stop' && final.includes(CLOSED_MARKER)) this.closeConfirmed = true;
    }
    if (final) live.text = final.replaceAll(CLOSED_MARKER, '').trimEnd();
    if (message.stopReason === 'error') {
      this.replyFailed = true;
      this.showReplyError(message.errorMessage || 'The provider returned no reply.');
    }
    if (!live.text.trim()) {
      live.el.remove();
      return;
    }
    this.activity = null;
    void this.renderLive(live);
  }

  private showReplyError(raw: string) {
    this.activity = null;
    const error = this.streamEl.createDiv({ cls: 'pi-learn-error', attr: { role: 'alert' } });
    error.createDiv({ cls: 'pi-learn-error-title', text: this.closing ? 'Could not finish the sitting' : 'The model could not respond' });
    let reason = raw;
    const jsonStart = raw.indexOf('{');
    if (jsonStart !== -1) {
      try {
        const body = JSON.parse(raw.slice(jsonStart));
        reason = body.error?.message ?? body.message ?? raw;
      } catch { /* Non-JSON provider errors remain readable as plain text. */ }
    }
    error.createEl('p', { text: reason });
    error.createEl('p', { cls: 'pi-learn-error-help', text: 'Choose an available model, then send your message again or retry Finish sitting.' });
    const actions = error.createDiv({ cls: 'pi-learn-error-actions' });
    actions.createEl('button', { text: 'Choose model' }).onclick = () => this.chooseModel();
    const details = actions.createEl('details');
    details.createEl('summary', { text: 'Technical details' });
    details.createEl('pre', { text: raw });
    this.scrollDown();
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
      if (this.disposed) return;
      const f = this.app.vault.getAbstractFileByPath(rel);
      if (f instanceof TFile) return this.disposed ? undefined : this.openInMain(f);
      await sleep(250);
    }
  }

  private async openInMain(file: TFile) {
    if (this.disposed) return;
    const existing = this.app.workspace.getLeavesOfType('markdown').find((l) => (l.view as any).file?.path === file.path);
    if (existing) {
      this.app.workspace.revealLeaf(existing);
      return;
    }
    const leaf = this.app.workspace.getMostRecentLeaf(this.app.workspace.rootSplit) ?? this.app.workspace.getLeaf('tab');
    await leaf.openFile(file, { active: false });
  }

  /* ───────────── extension dialogs ───────────── */

  private async cancelDialogs() {
    const responses: Promise<void>[] = [];
    for (const [id, dialog] of this.dialogs) {
      if (dialog.response) {
        dialog.controller.abort();
        responses.push(dialog.response);
      } else {
        this.dialogs.delete(id);
        if (dialog.pi.running) dialog.pi.respond(id, { cancelled: true });
        dialog.controller.abort();
      }
    }
    await Promise.all(responses);
  }

  private replyDialog(id: string, dialog: PendingDialog, answer: DialogAnswer) {
    if (this.dialogs.get(id) !== dialog) return;
    this.dialogs.delete(id);
    if (dialog.pi.running) dialog.pi.respond(id, answer);
  }

  private async uiRequest(e: PiEvent) {
    const pi = this.pi;
    if (!pi || this.disposed) return;
    const isDialog = ['select', 'confirm', 'input', 'editor'].includes(e.method);
    if (isDialog && (this.stopping || this.dialogs.has(e.id))) {
      if (!this.dialogs.has(e.id)) pi.respond(e.id, { cancelled: true });
      return;
    }
    const dialog: PendingDialog | null = isDialog ? { controller: new AbortController(), pi } : null;
    if (dialog) this.dialogs.set(e.id, dialog);
    const md = (el: HTMLElement, text: string) => {
      const component = new Component();
      component.load();
      this.components.push(component);
      void MarkdownRenderer.render(this.app, text, el, this.pathFile?.path ?? '', component);
    };

    if ((e.method === 'editor' || e.method === 'input') && typeof e.title === 'string' && e.title.startsWith(`${QUESTIONS_PROTOCOL} `)) {
      const widget = e.title.slice(QUESTIONS_PROTOCOL.length + 1);
      let params: any;
      try {
        params = JSON.parse((e.method === 'input' ? e.placeholder : e.prefill) ?? '');
      } catch {
        this.replyDialog(e.id, dialog!, { cancelled: true });
        return;
      }
      const slot = this.streamEl.createDiv({ cls: 'pi-learn-slot' });
      this.setStatus('');
      this.revealPending(slot);
      try {
        const signal = dialog!.controller.signal;
        const response =
          widget === 'quiz'
            ? quizCard(slot, params, md, signal)
            : widget === 'quiz-review'
              ? reviewCard(slot, params, md, signal)
              : widget === 'ask'
                ? askCard(slot, params, md, signal)
                : Promise.resolve(null);
        const completion = response.then((result) => {
          this.replyDialog(e.id, dialog!, result === null ? { cancelled: true } : { value: JSON.stringify(result) });

        });
        if (widget === 'ask') dialog!.response = completion;
        await completion;
      } catch (err) {
        dialog!.controller.abort();
        this.replyDialog(e.id, dialog!, { cancelled: true });
        this.notice(`Could not display the question: ${(err as Error).message}`, true);
      }
      this.scrollDown();
      return;
    }

    switch (e.method) {
      case 'select':
      case 'confirm':
      case 'input':
      case 'editor':
        this.genericDialog(e, dialog!);
        break;
      case 'notify':
        this.notice(e.message ?? '', e.notifyType === 'error');
        break;
      case 'setWidget':
        if (e.widgetKey === 'learn-sessions') {
          try {
            const data = JSON.parse(e.widgetLines?.[0] ?? e.widgetText?.[0] ?? '');
            if (Array.isArray(data.sessions)) this.sessions = data.sessions;
          } catch { this.notice('Could not read the saved-session list.', true); }
        }
        break;
      case 'setStatus':
        this.setStatus(e.statusText ?? '');
        break;
    }
  }

  /** A plain inline dialog for every other extension request. */
  private genericDialog(e: PiEvent, dialog: PendingDialog) {
    const card = this.streamEl.createEl('details', { cls: 'pi-learn-card' });
    card.open = true;
    this.revealPending(card);
    const summary = card.createEl('summary', { cls: 'pi-learn-card-label', text: e.title?.split('\n')[0] ?? 'Question' });
    const body = card.createDiv({ cls: 'pi-learn-card-body' });
    if (e.title?.includes('\n')) body.createEl('pre', { cls: 'pi-learn-context', text: e.title });
    if (e.message) body.createDiv({ cls: 'pi-learn-context', text: e.message });
    let expiry: number | null = null;
    const clearExpiry = () => { if (expiry !== null) window.clearTimeout(expiry); expiry = null; };
    const disable = () => {
      card.querySelectorAll<HTMLButtonElement | HTMLInputElement | HTMLTextAreaElement>('button,input,textarea').forEach((el) => { el.disabled = true; });
    };
    const cancel = () => {
      clearExpiry();
      disable();
      summary.setText('Request cancelled · nothing approved');
      card.open = false;
    };
    dialog.controller.signal.addEventListener('abort', cancel, { once: true });
    const done = (answer: DialogAnswer, label: string) => {
      if (this.dialogs.get(e.id) !== dialog) return;
      clearExpiry();
      dialog.controller.signal.removeEventListener('abort', cancel);
      disable();
      this.replyDialog(e.id, dialog, answer);
      summary.setText(`Answered: ${label} · sent to pi`);
      card.open = false;
    };
    if (typeof e.timeout === 'number' && Number.isFinite(e.timeout) && e.timeout > 0) {
      expiry = window.setTimeout(() => {
        if (this.dialogs.get(e.id) !== dialog) return;
        dialog.controller.abort();
        this.replyDialog(e.id, dialog, { cancelled: true });
        summary.setText('Request expired · nothing approved');
      }, e.timeout);
    }
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
    empty.createDiv({ cls: 'pi-learn-empty-title', text: 'Teach, practice or curate' });
    empty.createDiv({ cls: 'pi-learn-empty-note', text: 'Continue the selected track, start a New session, or open History.' });
    empty.createDiv({ cls: 'pi-learn-empty-note', text: 'Conversations are saved in pi. Notes carry learning progress. Time and depth are optional.' });
  }

  private paintBadge() {
    this.badgeEl.empty();
    this.badgeEl.toggleClass('is-busy', this.streaming);
    this.badgeEl.createSpan({ cls: 'pi-learn-spinner' });
    this.badgeEl.createSpan({ text: this.replyFailed ? 'Reply failed' : this.closing ? 'Finishing' : this.streaming ? 'Working' : this.sittingActive ? 'Your turn' : 'Ready' });
  }

  private bubble(text: string) {
    this.activity = null;
    this.streamEl.querySelector('.pi-learn-empty')?.remove();
    this.streamEl.createDiv({ cls: 'pi-learn-msg is-user', text });
    this.scrollDown();
  }

  private divider(label: string) {
    this.activity = null;
    this.streamEl.createDiv({ cls: 'pi-learn-divider', text: label });
  }

  private toolLine(icon: string, label: string, cls = '') {
    if (!this.activity) {
      this.activity = this.streamEl.createEl('details', { cls: 'pi-learn-activity' });
      this.activity.createEl('summary');
    }
    const line = this.activity.createDiv({ cls: `pi-learn-tool ${cls}`.trim(), attr: { title: label } });
    setIcon(line.createSpan({ cls: 'pi-learn-tool-icon' }), icon);
    line.createSpan({ text: label });
    const count = this.activity.querySelectorAll('.pi-learn-tool').length;
    const failed = this.activity.querySelectorAll('.is-error').length;
    this.activity.querySelector('summary')!.setText(`Activity · ${count} ${count === 1 ? 'step' : 'steps'}${failed ? ` · ${failed} failed` : ''}`);
    this.activity.toggleClass('has-error', failed > 0);
    this.scrollDown();
  }

  private grow() {
    this.input.style.height = 'auto';
    this.input.style.height = `${Math.min(this.input.scrollHeight, 240)}px`;
    this.paintControls();
  }

  private paintControls() {
    this.paintScope();
    if (!this.sendBtn) return;
    const locked = this.transitioning || this.stopping || this.disposed;
    const planningLocked = locked || this.sittingActive;
    for (const field of [this.targetSel, this.outcomeInput, this.minutesInput, this.modeSel, this.startBtn]) {
      field.disabled = planningLocked;
      field.title = this.sittingActive ? 'Finish the current sitting before planning another' : '';
    }
    this.continueBtn.disabled = locked || this.closing || this.streaming;
    this.newBtn.disabled = locked || this.closing;
    this.historyBtn.disabled = locked || this.closing || this.streaming;
    const sessionLock = locked ? 'A session change is in progress' : this.closing ? 'Finish is saving learning records' : this.streaming ? 'Stop the current reply first' : '';
    this.continueBtn.title = sessionLock || 'Restore the selected track’s last conversation without generating a reply';
    this.newBtn.title = locked || this.closing ? sessionLock : 'Start fresh; the previous conversation stays saved';
    this.historyBtn.title = sessionLock || 'Find a saved learning conversation';
    this.teachBtn.disabled = locked || this.closing;
    this.practiceBtn.disabled = locked || this.closing;
    this.curateBtn.disabled = locked || this.closing;
    this.teachBtn.setAttr('aria-pressed', String(this.kind === 'teach'));
    this.practiceBtn.setAttr('aria-pressed', String(this.kind === 'practice'));
    this.curateBtn.setAttr('aria-pressed', String(this.kind === 'curate'));
    this.finishBtn.disabled = locked || !this.sittingActive || this.closing;
    this.finishBtn.setAttr('title', !this.sittingActive ? 'Start a sitting first' : this.closing ? 'Saving learning records and the next step' : 'Complete the skill’s closing procedure and save its learning records');
    this.finishBtn.setText(this.closing ? 'Finishing…' : 'Finish sitting');
    this.sendBtn.disabled = locked || this.sessionUnavailable;
    this.input.disabled = this.sessionUnavailable;
    const stop = this.streaming && !this.input.value.trim();
    const label = stop ? 'Stop generating' : this.streaming ? 'Send steering message (Enter)' : this.sittingActive ? 'Send message (Enter)' : `Start ${this.kind === 'teach' ? 'teaching' : this.kind === 'curate' ? 'curation' : 'practice'} (Enter)`;
    setIcon(this.sendBtn, stop ? 'square' : 'arrow-up');
    this.sendBtn.setAttr('aria-label', label);
    this.sendBtn.setAttr('title', label);
    this.input.placeholder = this.sessionUnavailable ? 'Reopen the conversation or start a New session' : this.sittingActive ? 'Answer or ask…' : 'Name a topic, or use the open note…';
  }

  private setStreaming(on: boolean) {
    this.streaming = on;
    this.contentEl.toggleClass('is-streaming', on);
    this.paintBadge();
    if (!on) this.setStatus('');
    this.paintControls();
  }

  private setStatus(text: string) {
    this.statusEl.setText(text);
    this.statusEl.toggle(Boolean(text));
  }

  private notice(text: string, error = false) {
    new Notice(error ? `Learn: ${text}` : text, error ? 8000 : 4000);
  }

  /** A blocking question takes the reader to it, even when an earlier passage was being read. */
  private revealPending(el: HTMLElement) {
    this.stickToBottom = true;
    el.scrollIntoView?.({ block: 'nearest' });
    this.scrollDown();
  }

  private scrollDown() {
    if (this.stickToBottom) this.streamEl.scrollTop = this.streamEl.scrollHeight;
  }
}
