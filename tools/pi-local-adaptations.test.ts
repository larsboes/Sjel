import { describe, expect, test } from 'bun:test';
import { abortable } from '../Packs/harness/pi-packages/pi-subagents/src/abortable';
import { installExtensionToolScope, resumeAgent, runAgent } from '../Packs/harness/pi-packages/pi-subagents/src/agent-runner';
import { resolveModel } from '../Packs/harness/pi-packages/pi-subagents/src/model-resolver';
import { extractHtmlContent } from '../Packs/harness/pi-packages/pi-web-access/extract';
import { formatSearchResultsAsAnswer } from '../Packs/harness/pi-packages/pi-web-access/search-answer-formatting';
import { findModelWithProviderRouting } from '../Packs/harness/pi-packages/pi-web-access/summary-model-scope';

const models = [
  { provider: 'router-a', id: 'vendor/model', name: 'Model' },
  { provider: 'router-b', id: 'vendor/model', name: 'Model' },
  { provider: 'vendor', id: 'model', name: 'Model' },
];
const registry = {
  getAvailable: () => models,
  getAll: () => models,
  find: (provider: string, id: string) => models.find(m => m.provider === provider && m.id === id),
};

describe('local Pi model routing', () => {
  test('selects an enabled route even when an earlier route and native model are disabled', () => {
    expect(findModelWithProviderRouting(registry, 'vendor', 'model', [{ model: models[1] }])).toBe(models[1]);
  });

  test('does not escape the enabled scope through the registry fallback', () => {
    expect(findModelWithProviderRouting(registry, 'vendor', 'model', [{ model: { provider: 'other', id: 'other' } }])).toBeUndefined();
    expect(findModelWithProviderRouting(registry, 'router-a', 'vendor/model', [{ model: models[1] }])).toBeUndefined();
  });

  test('preserves native preference and explicit router selection', () => {
    expect(findModelWithProviderRouting(registry, 'vendor', 'model')).toBe(models[2]);
    expect(findModelWithProviderRouting(registry, 'router-b', 'vendor/model')).toBe(models[1]);
  });

  test('empty and malformed model pins cannot select an arbitrary available provider', () => {
    for (const input of ['', '  ', '/model', 'vendor/']) {
      expect(resolveModel(input, registry)).toContain('Invalid model selector');
    }
  });

  test('resolves case-insensitive exact model pins using the canonical registry keys', () => {
    const ambiguous = [{ provider: 'vendor', id: 'model-preview', name: 'Model preview' }, ...models];
    const canonicalRegistry = { ...registry, getAvailable: () => ambiguous };
    expect(resolveModel(' VENDOR/MODEL ', canonicalRegistry)).toBe(models[2]);
  });
});

type Session = Parameters<typeof resumeAgent>[0];
type Message = { role: 'assistant'; content: Array<{ type: 'text'; text: string }>; stopReason: 'stop' | 'error' | 'length' | 'aborted'; errorMessage?: string };

function resumedSession(message: Message) {
  const messages: Message[] = [];
  const listeners = new Set<unknown>();
  let prompts = 0;
  const session = {
    messages,
    subscribe(listener: unknown) { listeners.add(listener); return () => listeners.delete(listener); },
    async prompt() { prompts++; messages.push(message); },
    async abort() {},
  } as unknown as Session;
  return { session, listeners, prompts: () => prompts };
}

describe('subagent cancellation and outcomes', () => {
  test('a cancelled resume cannot start a new prompt or install listeners', async () => {
    const fixture = resumedSession({ role: 'assistant', content: [], stopReason: 'stop' });
    const controller = new AbortController();
    controller.abort(new Error('Stopped by caller'));
    await expect(resumeAgent(fixture.session, 'continue', { signal: controller.signal })).rejects.toThrow('Stopped by caller');
    expect(fixture.prompts()).toBe(0);
    expect(fixture.listeners.size).toBe(0);
  });

  test('a cancelled spawn exits before consulting the parent or creating resources', async () => {
    const controller = new AbortController();
    controller.abort(new Error('Stopped before spawn'));
    await expect(runAgent({} as Parameters<typeof runAgent>[0], 'Explore', 'search', {
      signal: controller.signal,
    } as Parameters<typeof runAgent>[3])).rejects.toThrow('Stopped before spawn');
  });

  test('provider aborts and errors retain partial text and report failure', async () => {
    for (const stopReason of ['aborted', 'error'] as const) {
      const fixture = resumedSession({ role: 'assistant', content: [{ type: 'text', text: 'Partial finding' }], stopReason, errorMessage: 'Provider unavailable' });
      const result = await resumeAgent(fixture.session, 'continue');
      expect(result.text).toBe('Partial finding');
      expect(result.failure).toBe(stopReason === 'aborted' ? 'provider aborted before completion' : 'Provider unavailable');
      expect(fixture.listeners.size).toBe(0);
    }
  });

  test('a failed empty resume cannot report a previous turn as its answer', async () => {
    const fixture = resumedSession({ role: 'assistant', content: [], stopReason: 'length' });
    fixture.session.messages.push({ role: 'assistant', content: [{ type: 'text', text: 'Old result' }], stopReason: 'stop' } as Session['messages'][number]);
    const result = await resumeAgent(fixture.session, 'new task');
    expect(result.text).toBe('');
    expect(result.failure).toContain('output token limit');
  });

  test('an already-cancelled wait observes a child that rejects afterward', async () => {
    let fail!: (error: Error) => void;
    const child = new Promise<string>((_resolve, reject) => { fail = reject; });
    const controller = new AbortController();
    controller.abort(new Error('Already stopped'));
    await expect(abortable(child, controller.signal)).rejects.toThrow('Already stopped');
    fail(new Error('Child failed later'));
    await new Promise(resolve => setTimeout(resolve, 0));
  });

  test('cancelling a result wait leaves the child promise available to another caller', async () => {
    let finish!: (value: string) => void;
    const child = new Promise<string>(resolve => { finish = resolve; });
    const controller = new AbortController();
    const waiting = abortable(child, controller.signal);
    controller.abort(new Error('Stop waiting'));
    await expect(waiting).rejects.toThrow('Stop waiting');
    finish('Completed child');
    expect(await child).toBe('Completed child');
  });
});

describe('subagent tool scope', () => {
  test('late tools remain narrowed, explicit denies win, and prior guards remain active', async () => {
    let onTurn: ((event: { type: string }) => void) | undefined;
    let active: string[] = [];
    const extensionTools = new Map([['fetch', {}], ['write', {}]]);
    const session = {
      agent: { beforeToolCall: async () => ({ block: true, reason: 'Parent policy' }) },
      getAllTools: () => ['read', 'bash', 'fetch', 'write', 'late', 'Agent'].map(name => ({ name })),
      getActiveToolNames: () => active,
      setActiveToolsByName: (names: string[]) => { active = names; },
      subscribe: (listener: typeof onTurn) => { onTurn = listener; return () => {}; },
    };
    installExtensionToolScope(session as unknown as Session, {
      loader: { getExtensions: () => ({ extensions: [{ path: '/extensions/search/index.ts', tools: extensionTools }] }) } as unknown as Parameters<typeof installExtensionToolScope>[1]['loader'],
      toolNames: ['read', 'bash'], disallowedSet: new Set(['bash', 'write']),
      extNames: new Set(['search']), narrowing: new Map([['search', new Set(['fetch'])]]), readmitToolNames: new Set(),
    });
    expect(active).toEqual(['read', 'fetch']);
    extensionTools.set('late', {});
    onTurn?.({ type: 'turn_end' });
    expect(active).toEqual(['read', 'fetch']);
    const guard = session.agent.beforeToolCall as unknown as (context: { toolCall: { name: string } }, signal: AbortSignal) => Promise<{ block: boolean; reason: string }>;
    for (const name of ['bash', 'write', 'late', 'Agent']) {
      expect((await guard({ toolCall: { name } }, new AbortController().signal)).reason).toContain('not available');
    }
    expect((await guard({ toolCall: { name: 'read' } }, new AbortController().signal)).reason).toBe('Parent policy');
  });
});

describe('web content and attribution', () => {
  test('retains short HTML content and resolves links against the final page URL', async () => {
    const result = await extractHtmlContent('<html><head><title>Reference</title></head><body><article><h1>Short reference</h1><p>A useful explanation with a <a href="guide">guide</a> and a short body.</p></article></body></html>', 'https://example.org/redirect', undefined, 'https://example.org/docs/page');
    expect(result.url).toBe('https://example.org/redirect');
    expect(result.content).toContain('A useful explanation');
    expect(result.content).toContain('https://example.org/docs/guide');
    expect(result.error).toContain('incomplete');
  });

  test('HTML base declarations resolve relative source links', async () => {
    const html = '<html><head><base href="https://docs.example.org/reference/"><title>Links</title></head><body><article><h1>Reference links</h1><p>This page links to the <a href="guide">complete guide</a> for its details.</p></article></body></html>';
    const result = await extractHtmlContent(html, 'https://example.org/original');
    expect(result.content).toContain('https://docs.example.org/reference/guide');
  });

  test('local HTML extraction respects an already-cancelled request', async () => {
    const controller = new AbortController();
    controller.abort(new Error('Stop extraction'));
    await expect(extractHtmlContent('<p>Unused</p>', 'https://example.org', controller.signal)).rejects.toThrow('Stop extraction');
  });

  test('formats clickable sources without losing snippets or URL parameters', () => {
    const answer = formatSearchResultsAsAnswer([{ title: 'Docs [stable]', url: 'https://example.org/docs?q=a&b=c', snippet: 'Supported behavior.' }]);
    expect(answer).toContain('Supported behavior.');
    expect(answer).toContain('[Docs \\[stable\\]](<https://example.org/docs?q=a&b=c>)');
    expect(formatSearchResultsAsAnswer([{ title: '', url: 'https://example.org', snippet: '' }])).toBe('Source: [https://example.org](<https://example.org>)');
  });
});
