import { afterAll, beforeAll, describe, expect, spyOn, test } from 'bun:test';
import { copyFileSync, existsSync, mkdtempSync, readFileSync, rmSync, symlinkSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { join } from 'node:path';
import { sittingPrompt } from '../Packs/learning/plugins/pi-learn/src/path';

const install = process.env.PI_MANAGED_INSTALL_ROOT ?? join(homedir(), '.pi/agent/install');
const marker = join(install, 'current-version');
const modules = existsSync(marker)
  ? join(install, 'releases', readFileSync(marker, 'utf8').trim(), 'node_modules')
  : undefined;
let fixture: string;
let extension: typeof import('../Packs/learning/extensions/sessions');
let SessionManager: any;

if (!modules) console.log('learn-sessions: skipped — managed Pi installation unavailable');

describe.skipIf(!modules)('learning session history', () => {
  beforeAll(async () => {
    fixture = mkdtempSync(join(tmpdir(), 'learn-sessions-'));
    symlinkSync(modules!, join(fixture, 'node_modules'), 'dir');
    copyFileSync(join(import.meta.dir, '../Packs/learning/extensions/sessions.ts'), join(fixture, 'sessions.ts'));
    extension = await import(join(fixture, 'sessions.ts'));
    ({ SessionManager } = await import(join(modules!, '@earendil-works/pi-coding-agent/dist/index.js')));
  });
  afterAll(() => { if (fixture) rmSync(fixture, { recursive: true, force: true }); });

  const session = (firstMessage: string, overrides = {}) => ({
    path: '/sessions/one.jsonl', firstMessage, modified: new Date('2026-11-01T10:00:00Z'), messageCount: 2,
    ...overrides,
  });

  test('named sessions keep the entire target, including separators, and override prompts', () => {
    for (const kind of ['teach', 'practice', 'curate']) {
      const target = 'Knowledge/Queues · delivery · trade-offs.md';
      expect(extension.learningSessions([session('unrelated', { name: `Learn · ${kind} · ${target}` })]))
        .toEqual([{ path: '/sessions/one.jsonl', name: `Learn · ${kind} · ${target}`, kind, target,
          modified: '2026-11-01T10:00:00.000Z', messageCount: 2 }]);
    }
  });

  test('legacy Learn openings prefer the learning path over focus', () => {
    for (const kind of ['teach', 'practice'] as const) {
      for (const focus of [null, '[[Knowledge/Queues|queueing]]']) {
        const firstMessage = sittingPrompt(kind, '[[Learning/Systems — Learning Path|Systems]]', focus, 'Follow my goal');
        const [result] = extension.learningSessions([session(firstMessage)]);
        expect(result.kind).toBe(kind);
        expect(result.target).toBe('Learning/Systems — Learning Path.md');
      }
    }
  });

  test('legacy focus, plain markdown path, topic and curate prompts retain only the target', () => {
    for (const kind of ['teach', 'practice', 'curate'] as const) {
      const [focus] = extension.learningSessions([session(sittingPrompt(kind, null, '[[Knowledge/Queues.md|Queues]]', 'My goal'))]);
      expect(focus.target).toBe('Knowledge/Queues.md');
      const [topic] = extension.learningSessions([session(sittingPrompt(kind, null, null, 'Distributed queues'))]);
      expect(topic.target).toBe('Distributed queues');
    }
    expect(extension.learningSessions([session('/skill:teach Continue the learning path Learning/Systems.md at [[Knowledge/Queues]].')])[0].target)
      .toBe('Learning/Systems.md');
  });

  test('expanded skills use the final request, never examples in the skill body', () => {
    const expanded = '<skill name="teach" location="/skills/teach/SKILL.md">\n'
      + '/skill:practice Practice the claims of [[Wrong]].\n</skill>\n\n';
    const [result] = extension.learningSessions([session(expanded
      + 'Continue the learning path [[Learning/Data Pipeline Systems — Learning Path]]. Start at A1.')]);
    expect(result.kind).toBe('teach');
    expect(result.target).toBe('Learning/Data Pipeline Systems — Learning Path.md');
    expect(extension.learningSessions([session(expanded + '/skill:curate Curate [[Knowledge/Queues]]. Start with this note and its directly related active notes.')])[0].kind)
      .toBe('curate');
    expect(extension.learningSessions([session(expanded)])).toEqual([]);
  });

  test('incidental quotes, generic sessions, malformed names and empty sessions are omitted', () => {
    const unrelated = [
      'Explain how /skill:teach Rust works',
      '"/skill:teach Continue the learning path [[Learning/Systems]]."',
      'Review this prompt:\n/skill:practice Practice the claims of [[Knowledge/Queues]].',
      'Continue the learning path [[Learning/Systems]].',
      '<skill name="coding" location="/coding">\n/skill:teach Rust\n</skill>\n\nWrite code',
      '<skill name="teach" location="/teach">\nExample /skill:teach Rust',
    ];
    expect(extension.learningSessions(unrelated.map((prompt) => session(prompt)))).toEqual([]);
    expect(extension.learningSessions([
      session('generic', { name: 'Learn · teach · ' }),
      session('generic', { name: 'Learn · coding · Rust' }),
      session('/skill:teach Rust', { messageCount: 0 }),
    ])).toEqual([]);
  });

  test('metadata projection excludes transcript fields, sorts newest first and has no history cap', () => {
    const input = Array.from({ length: 301 }, (_, i) => session('/skill:teach Queues', {
      path: `/sessions/${i}.jsonl`, modified: new Date(2026, 0, 1, 0, i), allMessagesText: 'PRIVATE TRANSCRIPT',
    }));
    const result = extension.learningSessions(input);
    expect(result).toHaveLength(301);
    expect(result[0].path).toBe('/sessions/300.jsonl');
    expect(Object.keys(result[0]).sort()).toEqual(['kind', 'messageCount', 'modified', 'name', 'path', 'target']);
    expect(JSON.stringify(result)).not.toContain('PRIVATE TRANSCRIPT');
    expect(input[0].path).toBe('/sessions/0.jsonl');
  });

  test('command lists the current cwd only for the supported RPC host, without model messages', async () => {
    const commands = new Map<string, any>();
    extension.default({ registerCommand: (name: string, command: any) => commands.set(name, command) } as any);
    expect([...commands.keys()]).toEqual(['learn-sessions']);
    const widgets: unknown[][] = [];
    const notifications: string[] = [];
    const list = spyOn(SessionManager, 'list').mockResolvedValue([session('/skill:teach Queues')]);
    const old = process.env.PI_QUESTIONS_CLIENT;
    const ctx = { cwd: '/vault', mode: 'rpc', ui: {
      setWidget: (...args: unknown[]) => widgets.push(args),
      notify: (message: string) => notifications.push(message),
    } };
    try {
      process.env.PI_QUESTIONS_CLIENT = 'pi-questions/v1';
      await commands.get('learn-sessions').handler('', ctx);
      expect(list.mock.calls).toEqual([['/vault']]);
      expect(widgets).toEqual([['learn-sessions', [JSON.stringify({ sessions: extension.learningSessions([session('/skill:teach Queues')]) })]]]);
      for (const [mode, client] of [['tui', 'pi-questions/v1'], ['rpc', 'unknown'], ['print', 'pi-questions/v1']]) {
        ctx.mode = mode;
        process.env.PI_QUESTIONS_CLIENT = client;
        await commands.get('learn-sessions').handler('', ctx);
      }
      expect(list).toHaveBeenCalledTimes(1);
      expect(widgets).toHaveLength(1);
      expect(notifications).toHaveLength(3);
      expect(notifications[0]).toContain('/resume');
    } finally {
      list.mockRestore();
      if (old === undefined) delete process.env.PI_QUESTIONS_CLIENT;
      else process.env.PI_QUESTIONS_CLIENT = old;
    }
  });
});
