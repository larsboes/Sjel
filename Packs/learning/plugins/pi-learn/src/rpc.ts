import { spawn, type ChildProcessWithoutNullStreams } from 'child_process';
import type { Launch } from './launch';

/** One record Pi writes on stdout that is not a command response. */
export type PiEvent = { type: string; [key: string]: any };

type Pending = { resolve: (data: any) => void; reject: (err: Error) => void };

/** One model as `get_available_models` lists it (`docs/rpc-commands.md`, Model object). */
export interface PiModel {
  id: string;
  name?: string;
  provider: string;
  reasoning?: boolean;
  contextWindow?: number;
}

/** `provider/id`, the form the settings store and `splitModel` reads. */
export const modelKey = (m: Pick<PiModel, 'provider' | 'id'>) => `${m.provider}/${m.id}`;

/** Start a short-lived pi to list its models and its default, for the settings tab. */
export async function listModels(launch: Launch): Promise<{ models: PiModel[]; current: string | null }> {
  const pi = new PiRpc(launch, () => undefined, () => undefined);
  pi.start();
  try {
    const [list, state] = await Promise.all([
      pi.command({ type: 'get_available_models' }),
      pi.command({ type: 'get_state' }),
    ]);
    return { models: list?.models ?? [], current: state?.model ? modelKey(state.model) : null };
  } finally {
    pi.stop();
  }
}

/**
 * Pi in RPC mode as a child process: JSONL commands on stdin, responses and events on stdout.
 * Records split on LF only, as `docs/rpc.md` (Framing) requires: JSON strings may hold U+2028.
 */
export class PiRpc {
  private proc: ChildProcessWithoutNullStreams | null = null;
  private buffer = '';
  private seq = 0;
  private pending = new Map<string, Pending>();
  private stderr = '';

  constructor(
    private readonly launch: Launch,
    private readonly onEvent: (event: PiEvent) => void,
    private readonly onExit: (code: number | null, stderr: string) => void,
  ) {}

  get running(): boolean {
    return this.proc !== null;
  }

  start(): void {
    const proc = spawn(this.launch.command, this.launch.args, {
      cwd: this.launch.cwd,
      env: this.launch.env,
      shell: this.launch.shell,
      windowsHide: true,
    });
    this.proc = proc;
    proc.stdout.setEncoding('utf8');
    proc.stdout.on('data', (chunk: string) => this.read(chunk));
    proc.stderr.setEncoding('utf8');
    proc.stderr.on('data', (chunk: string) => {
      this.stderr = (this.stderr + chunk).slice(-4000);
    });
    proc.on('error', (err) => this.exit(null, `${err.message}\n${this.stderr}`));
    proc.on('close', (code) => this.exit(code, this.stderr));
  }

  /** Send one command and resolve with its response `data`, or reject with its `error`. */
  command(cmd: Record<string, unknown>): Promise<any> {
    if (!this.proc) return Promise.reject(new Error('pi is not running'));
    const id = `c${++this.seq}`;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.write({ ...cmd, id });
    });
  }

  /** Answer an `extension_ui_request` dialog. */
  respond(id: string, answer: { value?: string; confirmed?: boolean; cancelled?: true }): void {
    this.write({ type: 'extension_ui_response', id, ...answer });
  }

  stop(): void {
    this.proc?.kill();
  }

  private write(record: Record<string, unknown>): void {
    this.proc?.stdin.write(JSON.stringify(record) + '\n');
  }

  private read(chunk: string): void {
    this.buffer += chunk;
    let nl: number;
    while ((nl = this.buffer.indexOf('\n')) >= 0) {
      const line = this.buffer.slice(0, nl).replace(/\r$/, '');
      this.buffer = this.buffer.slice(nl + 1);
      if (!line.trim()) continue;
      let record: PiEvent;
      try {
        record = JSON.parse(line);
      } catch {
        continue;
      }
      const waiter = record.type === 'response' && record.id ? this.pending.get(record.id) : undefined;
      if (waiter) {
        this.pending.delete(record.id);
        if (record.success === false) waiter.reject(new Error(record.error ?? `${record.command} failed`));
        else waiter.resolve(record.data);
      } else {
        this.onEvent(record);
      }
    }
  }

  private exit(code: number | null, stderr: string): void {
    if (!this.proc) return;
    this.proc = null;
    for (const p of this.pending.values()) p.reject(new Error('pi exited'));
    this.pending.clear();
    this.onExit(code, stderr);
  }
}
