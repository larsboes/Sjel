import { existsSync, mkdirSync, renameSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import type { ExtensionAPI, ExtensionContext } from '@earendil-works/pi-coding-agent';

// Reports Pi's state to the Rex terminal as program status, so Rex shows a waiting Pi pane the
// way it shows Claude Code. Protocol: OSC 7501, https://www.superlogical.com/rex/docs/build/program-status.md
// It also records the pane's session for the Rex "Resume Agents" action, in the format that
// ~/.config/rex/bin/rex-agent-record writes for Claude Code and Codex.
// Inert outside Rex (no REX_BLOCK) and outside the interactive TUI.

export type RexState = 'idle' | 'working' | 'done' | 'blocked' | 'error' | 'clear';
export type BlockedKind = 'permission' | 'question' | 'auth';

export interface RexReport {
  state: RexState;
  kind?: BlockedKind;
  msg?: string;
}

const b64 = (text: string) => Buffer.from(text, 'utf8').toString('base64');

/** One OSC 7501 report. `msg` is base64 so that `:` and `;` in it cannot break the pair list. */
export function encode(report: RexReport): string {
  const pairs = [`state=${report.state}`, 'app=pi'];
  if (report.state === 'blocked' && report.kind) pairs.push(`kind=${report.kind}`);
  if (report.msg) pairs.push(`msg=${b64(report.msg.slice(0, 200))}`);
  return `\x1b]7501;${pairs.join(':')}\x1b\\`;
}

/** A confirm is an approval; every other prompt waits for an answer. */
export function promptKind(kind: string | undefined): BlockedKind {
  return kind === 'confirm' ? 'permission' : 'question';
}

export interface ResumeRecord {
  block: string;
  session: string;
  agent: 'pi';
  id: string;
  cwd: string;
  file: string;
  at: string;
}

/** Writes one record per pane. Best effort: a containment profile may refuse the write, and the
 * resume action then falls back to `pi --continue` in the pane's folder. */
export function writeRecord(env: NodeJS.ProcessEnv, id: string, cwd: string, file: string, now = new Date()): string | undefined {
  const block = env.REX_BLOCK;
  const base = env.XDG_STATE_HOME ?? (env.HOME ? join(env.HOME, '.local', 'state') : undefined);
  if (!block || !base || !id) return undefined;
  const record: ResumeRecord = {
    block, session: env.REX_SESSION ?? '', agent: 'pi', id, cwd, file,
    at: now.toISOString().replace(/\.\d{3}Z$/, 'Z'),
  };
  try {
    const dir = join(base, 'rex-agents');
    mkdirSync(dir, { recursive: true });
    const out = join(dir, `${block.replace(/^block:/, '')}.json`);
    const tmp = `${out}.${process.pid}.tmp`;
    writeFileSync(tmp, `${JSON.stringify(record, null, 2)}\n`);
    renameSync(tmp, out);
    return out;
  } catch {
    return undefined;
  }
}

export default function rexStatus(pi: ExtensionAPI, env: NodeJS.ProcessEnv = process.env) {
  if (!env.REX_BLOCK) return;
  let enabled = false;
  const report = (r: RexReport) => {
    if (enabled) process.stdout.write(encode(r));
  };

  // Pi saves a session file only after the first message, so record once the file exists.
  const record = (ctx: ExtensionContext) => {
    if (!enabled) return;
    const file = ctx.sessionManager.getSessionFile();
    if (file && existsSync(file)) {
      writeRecord(env, ctx.sessionManager.getSessionId(), ctx.sessionManager.getCwd(), file);
    }
  };

  pi.on('session_start', async (_event, ctx) => {
    enabled = ctx.mode === 'tui';
    report({ state: 'idle' });
    record(ctx);
  });
  pi.on('agent_start', async () => report({ state: 'working' }));
  pi.on('tool_execution_start', async (event) => report({ state: 'working', msg: event.toolName }));
  pi.on('ui_prompt_start', async (event) => report({ state: 'blocked', kind: promptKind(event.kind), msg: event.title }));
  pi.on('ui_prompt_end', async () => report({ state: 'working' }));
  pi.on('agent_settled', async (event, ctx) => {
    report(event.aborted ? { state: 'idle' } : { state: 'done', msg: 'Ready for input' });
    record(ctx);
  });
  pi.on('session_shutdown', async () => report({ state: 'clear' }));
}
