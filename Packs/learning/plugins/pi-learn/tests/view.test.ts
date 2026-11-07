import { afterEach, describe, expect, mock, test } from 'bun:test';
import { button, document, flush, key, markdown, root } from './dom';

const notices: string[] = [];
class Component { load() {} unload() {} }
class TFile { constructor(public path: string, public basename = path, public extension = 'md') {} }
class ItemView {
  app: any;
  contentEl = root();
  constructor(leaf: any) { this.app = leaf.app; }
  registerEvent() {}
}
class Modal {
  titleEl = root();
  contentEl = root();
  constructor(_app: unknown) {}
  open() {}
  close() { this.titleEl.remove(); this.contentEl.remove(); this.onClose(); }
  onClose() {}
}
mock.module('obsidian', () => ({
  Component, ItemView, TFile, Modal,
  Notice: class { constructor(text: string) { notices.push(text); } },
  FuzzySuggestModal: class {},
  MarkdownRenderer: { async render(_app: unknown, text: string, el: HTMLElement) { markdown(el, text); } },
  setIcon(el: HTMLElement, icon: string) { el.setAttribute('data-icon', icon); },
}));
const { LearnView } = await import('../src/view');
const views: any[] = [];

async function fixture() {
  const events: Array<{ target: any; event: string; callback: Function }> = [];
  const app = {
    workspace: { on(event: string, callback: Function) { events.push({ target: this, event, callback }); }, getActiveFile: () => null },
    vault: { on(event: string, callback: Function) { events.push({ target: this, event, callback }); }, adapter: { getBasePath: () => '/vault' } },
  };
  const plugin = { settings: { piCommand: 'pi', launcher: 'native', followEdits: false, teachModel: '', practiceModel: '', teachThinking: '', practiceThinking: '' }, save: async () => {} };
  const calls: any[] = [];
  const replies: any[] = [];
  const pi = {
    running: true,
    async command(cmd: any) {
      calls.push(cmd);
      if (cmd.type === 'get_available_models') return { models: [] };
      if (cmd.type === 'get_state') return {};
      if (cmd.type === 'get_available_thinking_levels') return { levels: ['off'] };
      if (cmd.type === 'prompt') return { disposition: 'started' };
      return {};
    },
    respond(id: string, reply: unknown) { replies.push({ id, ...reply as object }); },
    stop() { this.running = false; },
  };
  const view: any = new LearnView({ app } as any, plugin as any);
  view.pi = pi;
  views.push(view);
  await view.onOpen();
  await flush();
  calls.length = 0;
  return { view, pi, calls, replies };
}

async function start(f: Awaited<ReturnType<typeof fixture>>) {
  f.view.input.value = 'Locality';
  await f.view.send();
  f.view.onEvent({ type: 'agent_settled' });
  await flush();
}

function message(view: any, text: string, stopReason = 'stop') {
  view.onEvent({ type: 'message_start', message: { role: 'assistant' } });
  view.onEvent({ type: 'message_end', message: { role: 'assistant', stopReason, content: [{ type: 'text', text }] } });
}

function quizRequest(id = 'quiz-1') {
  return { type: 'extension_ui_request', id, method: 'editor', title: 'pi-questions/v1 quiz', prefill: JSON.stringify({ topic: 'Locality', questions: [{ id: 'q', prompt: 'What changes?', options: ['Latency', 'Nothing'], correctIndex: 0 }] }) };
}

afterEach(async () => {
  for (const view of views.splice(0)) if (!view.disposed) await view.onClose();
  notices.length = 0;
  document.body.replaceChildren();
});

describe('sitting controls', () => {
  test('Enter on an initial topic invokes Teach and persistent controls survive the first turn', async () => {
    const f = await fixture();
    f.view.input.value = 'Locality';
    key(f.view.input, 'Enter');
    await flush();
    expect(f.calls.some((c) => c.type === 'new_session')).toBe(true);
    expect(f.calls.find((c) => c.type === 'prompt').message).toBe('/skill:teach Locality');
    expect(button(f.view.contentEl, 'Teach')).toBeDefined();
    expect(button(f.view.contentEl, 'Practice')).toBeDefined();
    expect(button(f.view.contentEl, 'Finish sitting').disabled).toBe(false);
    expect(f.view.sendBtn.getAttribute('aria-label')).toBe('Stop generating');
    expect(f.view.sendBtn.getAttribute('data-icon')).toBe('square');
    f.view.input.value = 'Another question';
    f.view.input.dispatchEvent(new Event('input'));
    expect(f.view.sendBtn.getAttribute('aria-label')).toContain('steering');
  });

  test('starting another sitting requires a decision and Keep preserves the session', async () => {
    const f = await fixture();
    await start(f);
    f.calls.length = 0;
    const keep = f.view.sitting('practice');
    button(f.view.resetModal.contentEl, 'Keep sitting').click();
    await keep;
    expect(f.calls.some((c) => c.type === 'new_session')).toBe(false);
    const fresh = f.view.sitting('practice');
    button(f.view.resetModal.contentEl, 'Start fresh').click();
    await fresh;
    expect(f.calls.some((c) => c.type === 'new_session')).toBe(true);
    expect(f.calls.find((c) => c.type === 'prompt').message).toBe('/skill:practice Locality');
    expect(f.view.kind).toBe('practice');
  });

  test('an extension-cancelled reset does not clear the transcript or change the sitting kind', async () => {
    const f = await fixture();
    await start(f);
    const original = f.pi.command.bind(f.pi);
    f.pi.command = async (cmd) => cmd.type === 'new_session' ? { cancelled: true } : original(cmd);
    f.view.input.value = 'New topic';
    const before = f.view.streamEl.textContent;
    const pending = f.view.sitting('practice');
    button(f.view.resetModal.contentEl, 'Start fresh').click();
    await pending;
    expect(f.view.streamEl.textContent).toBe(before);
    expect(f.view.kind).toBe('teach');
  });

  test('Finish requests closing records and only a successful final marker closes the sitting', async () => {
    const f = await fixture();
    await start(f);
    await f.view.finishSitting();
    const prompt = f.calls.filter((c) => c.type === 'prompt').at(-1).message;
    expect(prompt).toContain('Learning Log');
    expect(prompt).toContain('Do not invent mastery');
    message(f.view, 'One more question is needed.');
    f.view.onEvent({ type: 'agent_settled' });
    expect(f.view.sittingActive).toBe(true);
    await f.view.finishSitting();
    message(f.view, 'Saved the learning records. <!-- pi-learn:sitting-closed -->');
    f.view.onEvent({ type: 'agent_settled' });
    expect(f.view.sittingActive).toBe(false);
    expect(f.view.finishBtn.disabled).toBe(true);
    expect(f.view.streamEl.textContent).toContain('Sitting closed');
    expect(f.view.streamEl.textContent).not.toContain('<!-- pi-learn:sitting-closed -->');
    expect(f.view.sendBtn.getAttribute('aria-label')).toBe('Start teaching (Enter)');
  });

  test('a failed closing tool or aborted reply cannot claim the sitting closed', async () => {
    const f = await fixture();
    await start(f);
    await f.view.finishSitting();
    f.view.onEvent({ type: 'tool_execution_end', toolName: 'write', isError: true });
    message(f.view, '<!-- pi-learn:sitting-closed -->');
    f.view.onEvent({ type: 'agent_settled' });
    expect(f.view.sittingActive).toBe(true);
    await f.view.finishSitting();
    message(f.view, '<!-- pi-learn:sitting-closed -->', 'aborted');
    f.view.onEvent({ type: 'agent_settled' });
    expect(f.view.sittingActive).toBe(true);
  });

  test('fast completion before prompt acknowledgement does not leave a false busy state', async () => {
    const f = await fixture();
    const original = f.pi.command.bind(f.pi);
    f.pi.command = async (cmd) => {
      if (cmd.type === 'prompt') f.view.onEvent({ type: 'agent_settled' });
      return original(cmd);
    };
    f.view.input.value = 'Locality';
    await f.view.send();
    expect(f.view.streaming).toBe(false);
  });
});

describe('pending dialog lifecycle', () => {
  test('Stop cancels a pending quiz once, clears the queue and rejects stale submission', async () => {
    const f = await fixture();
    await start(f);
    const pending = f.view.uiRequest(quizRequest());
    const choice = f.view.streamEl.querySelector('.pi-learn-option');
    await f.view.abort();
    await pending;
    choice.click();
    expect(f.replies).toEqual([{ id: 'quiz-1', cancelled: true }]);
    expect(f.calls.slice(-2).map((c) => c.type)).toEqual(['clear_queue', 'abort']);
    expect(f.view.dialogs.size).toBe(0);
    expect(choice.disabled).toBe(true);
  });

  test('sidebar close cancels both widget and generic dialogs before stopping pi', async () => {
    const f = await fixture();
    await start(f);
    const pending = f.view.uiRequest(quizRequest());
    await f.view.uiRequest({ type: 'extension_ui_request', id: 'confirm-1', method: 'confirm', title: 'Continue?' });
    const yes = button(f.view.streamEl, 'Yes');
    const stale = yes.onclick;
    await f.view.onClose();
    await pending;
    stale.call(yes, new Event('click'));
    expect(f.replies).toEqual([{ id: 'quiz-1', cancelled: true }, { id: 'confirm-1', cancelled: true }]);
    expect(f.pi.running).toBe(false);
    expect(f.view.dialogs.size).toBe(0);
  });

  test('Finish cancels the outstanding question before sending the closing request', async () => {
    const f = await fixture();
    await start(f);
    f.view.setStreaming(true);
    const pending = f.view.uiRequest(quizRequest());
    await f.view.finishSitting();
    await pending;
    expect(f.replies).toEqual([{ id: 'quiz-1', cancelled: true }]);
    expect(f.calls.slice(-3).map((c) => c.type)).toEqual(['clear_queue', 'abort', 'prompt']);
    expect(f.view.closing).toBe(true);
  });

  test('a failed Stop prevents Finish from issuing another prompt', async () => {
    const f = await fixture();
    await start(f);
    f.view.setStreaming(true);
    f.calls.length = 0;
    f.pi.command = async (cmd) => { f.calls.push(cmd); throw new Error('transport unavailable'); };
    await f.view.finishSitting();
    expect(f.calls.map((c) => c.type)).toEqual(['clear_queue']);
    expect(f.view.closing).toBe(false);
    expect(f.view.streaming).toBe(true);
  });
});

describe('header and follow-up safety', () => {
  test('overlapping path paints leave one subtitle and one dots row', async () => {
    const { view } = await fixture();
    const pending: Array<(value: string) => void> = [];
    view.app.vault.cachedRead = () => new Promise((resolve) => pending.push(resolve));
    view.pathFile = new TFile('P.md');
    view.focusFile = null;
    const first = view.paintPath();
    const second = view.paintPath();
    pending[1]('## Learning Path\n\n```mermaid\ngraph TD\n  a["A"]:::held\n```\n');
    await second;
    pending[0]('## Learning Path\n\n```mermaid\ngraph TD\n  a["A"]:::open\nb["B"]\n```\n');
    await first;
    expect(view.pathTitleEl.querySelectorAll('.pi-learn-title-sub')).toHaveLength(1);
    expect(view.dotsEl.children).toHaveLength(1);
  });

  test('a write that lands after close is not opened', async () => {
    const { view } = await fixture();
    let opened = 0;
    view.openInMain = async () => { opened++; };
    view.disposed = true;
    view.app.vault.getAbstractFileByPath = () => new TFile('Note.md');
    await view.follow('Note.md');
    expect(opened).toBe(0);
  });
});

describe('reading position', () => {
  test('streaming leaves a reader above the bottom in place and resumes following at the bottom', async () => {
    const { view } = await fixture();
    Object.defineProperties(view.streamEl, { scrollHeight: { configurable: true, value: 1000 }, clientHeight: { configurable: true, value: 300 } });
    view.streamEl.scrollTop = 100;
    view.streamEl.dispatchEvent(new Event('scroll'));
    message(view, 'A new explanation');
    await flush();
    expect(view.streamEl.scrollTop).toBe(100);
    view.streamEl.scrollTop = 700;
    view.streamEl.dispatchEvent(new Event('scroll'));
    view.scrollDown();
    expect(view.streamEl.scrollTop).toBe(1000);
  });
});
