import { describe, expect, test } from 'bun:test';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import register, { encode, promptKind, writeRecord } from '../Packs/harness/extensions/rex-status';

type Handler = (event: any, ctx: any) => Promise<void>;

function fakePi() {
  const handlers = new Map<string, Handler>();
  return { handlers, pi: { on: (name: string, fn: Handler) => handlers.set(name, fn) } as any };
}

function captureStdout(run: () => Promise<void>): Promise<string> {
  const original = process.stdout.write.bind(process.stdout);
  let out = '';
  (process.stdout as any).write = (chunk: string) => { out += chunk; return true; };
  return run().then(() => out).finally(() => { (process.stdout as any).write = original; });
}

describe('rex-status', () => {
  test('encodes a blocked report with kind and a base64 message', () => {
    const msg = Buffer.from('Allow rm: yes;no').toString('base64');
    expect(encode({ state: 'blocked', kind: 'permission', msg: 'Allow rm: yes;no' }))
      .toBe(`\x1b]7501;state=blocked:app=pi:kind=permission:msg=${msg}\x1b\\`);
  });

  test('drops kind on states other than blocked', () => {
    expect(encode({ state: 'working', kind: 'question' })).toBe('\x1b]7501;state=working:app=pi\x1b\\');
  });

  test('maps a confirm prompt to permission and others to question', () => {
    expect(promptKind('confirm')).toBe('permission');
    expect(promptKind('select')).toBe('question');
    expect(promptKind(undefined)).toBe('question');
  });

  test('registers nothing outside Rex', () => {
    const { handlers, pi } = fakePi();
    register(pi, {});
    expect(handlers.size).toBe(0);
  });

  test('stays silent outside the TUI', async () => {
    const { handlers, pi } = fakePi();
    register(pi, { REX_BLOCK: 'block:x' });
    const out = await captureStdout(async () => {
      await handlers.get('session_start')!({}, { mode: 'print' });
      await handlers.get('agent_start')!({}, {});
    });
    expect(out).toBe('');
  });

  test('reports the lifecycle in the TUI', async () => {
    const { handlers, pi } = fakePi();
    register(pi, { REX_BLOCK: 'block:x' });
    const out = await captureStdout(async () => {
      await handlers.get('session_start')!({}, { mode: 'tui', sessionManager: { getSessionId: () => '', getCwd: () => '/tmp', getSessionFile: () => undefined } });
      await handlers.get('agent_start')!({}, {});
      await handlers.get('ui_prompt_start')!({ kind: 'confirm', title: 'Run bash?' }, {});
      await handlers.get('agent_settled')!({ aborted: false }, { sessionManager: { getSessionFile: () => undefined } });
    });
    expect(out.match(/state=\w+/g)).toEqual(['state=idle', 'state=working', 'state=blocked', 'state=done']);
    expect(out).toContain('kind=permission');
  });

  test('writes one resume record per pane', () => {
    const dir = mkdtempSync(join(tmpdir(), 'rex-status-'));
    try {
      const file = writeRecord({ REX_BLOCK: 'block:abc', REX_SESSION: 'session:s', XDG_STATE_HOME: dir }, 'sess-1', '/work', '/s.jsonl', new Date('2026-11-19T07:00:00.123Z'));
      expect(file).toBe(join(dir, 'rex-agents', 'abc.json'));
      expect(JSON.parse(readFileSync(file!, 'utf8'))).toEqual({
        block: 'block:abc', session: 'session:s', agent: 'pi', id: 'sess-1', cwd: '/work', file: '/s.jsonl', at: '2026-11-19T07:00:00Z',
      });
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test('records nothing without a session id', () => {
    expect(writeRecord({ REX_BLOCK: 'block:abc', XDG_STATE_HOME: '/nonexistent' }, '', '/work', '')).toBeUndefined();
  });

  test('records a saved session in the TUI and skips an unsaved one', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'rex-status-'));
    try {
      const saved = join(dir, 'session.jsonl');
      writeFileSync(saved, '{}\n');
      const env = { REX_BLOCK: 'block:p1', XDG_STATE_HOME: dir };
      const ctx = (file: string) => ({ mode: 'tui', sessionManager: { getSessionId: () => 's1', getCwd: () => '/w', getSessionFile: () => file } });
      const { handlers, pi } = fakePi();
      register(pi, env);
      await captureStdout(() => handlers.get('session_start')!({}, ctx(join(dir, 'missing.jsonl'))));
      expect(existsSync(join(dir, 'rex-agents', 'p1.json'))).toBe(false);
      await captureStdout(() => handlers.get('agent_settled')!({ aborted: false }, ctx(saved)));
      expect(JSON.parse(readFileSync(join(dir, 'rex-agents', 'p1.json'), 'utf8')).file).toBe(saved);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});
