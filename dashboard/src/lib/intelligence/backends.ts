/**
 * The I/O half of the model ladder in `./ladder.ts`: probe each rung, then run a task down the
 * plan until one rung answers. Returns `text: null` with `source: 'rules'` when no model can take
 * it, so the caller keeps its deterministic result.
 */
import { invoke } from '@tauri-apps/api/core';
import { inTauri, macRequest } from '$lib/mac-bridge';
import {
  DEFAULT_REPLY_TOKENS,
  FALL_THROUGH_CODES,
  CHARS_PER_TOKEN,
  plan,
  type ModelTask,
  type Rung,
  type RungStatus,
  type Skip,
} from './ladder';

interface PluginModelStatus {
  status: 'available' | 'unavailable';
  reason?: string | null;
  contextSize?: number | null;
}

interface PluginAvailability {
  onDevice: PluginModelStatus;
  privateCloud: PluginModelStatus;
}

export interface Probe {
  statuses: RungStatus[];
  /** Apple Private Cloud Compute, reported only. No task is sent there. */
  privateCloud: { available: boolean; reason?: string } | null;
}

export interface ModelResult {
  text: string | null;
  source: Rung;
  skipped: Skip[];
}

/** The model id apfel serves; see capabilities/foundation-models/README.md "Wiring it up". */
const MAC_MODEL = 'apple-foundationmodel';
/** Shell-proxied route to capabilities/foundation-models. */
const LOCAL_BASE = '/assistant';
const MAC_BASE = '/foundation-models';

function isIosApp(): boolean {
  return inTauri() && /iPhone|iPad|iPod/.test(window.navigator.userAgent);
}

function fromPlugin(rung: Rung, status: PluginModelStatus): RungStatus {
  return status.status === 'available'
    ? { rung, available: true, contextTokens: status.contextSize ?? undefined }
    : { rung, available: false, reason: status.reason ?? 'unavailable' };
}

async function probeOnDevice(): Promise<{ onDevice: RungStatus; privateCloud: Probe['privateCloud'] }> {
  if (!isIosApp()) {
    return { onDevice: { rung: 'on-device', available: false, reason: 'not the iOS app' }, privateCloud: null };
  }
  try {
    const result = await invoke<PluginAvailability>('plugin:foundation-models|availability');
    return {
      onDevice: fromPlugin('on-device', result.onDevice),
      privateCloud:
        result.privateCloud.status === 'available'
          ? { available: true }
          : { available: false, reason: result.privateCloud.reason ?? 'unavailable' },
    };
  } catch (error) {
    return { onDevice: { rung: 'on-device', available: false, reason: String(error) }, privateCloud: null };
  }
}

interface MacReply {
  status: number;
  body: string;
  /** The bridge answered from the device's copy because the Mac was not reached. */
  stale: boolean;
}

/** In the app, through the native bridge (signed, allow-listed in src-tauri/src/mac_bridge.rs);
 *  in a browser, same-origin through the shell. Rejects when the Mac was not reached. */
async function callMac(path: string, init?: RequestInit): Promise<MacReply> {
  if (inTauri()) {
    const res = await macRequest(path, init);
    return { status: res.status, body: res.body, stale: res.stale === true };
  }
  const res = await fetch(path, init);
  return { status: res.status, body: await res.text(), stale: false };
}

async function probeMac(): Promise<RungStatus> {
  try {
    const res = await callMac(`${MAC_BASE}/health`);
    // A cached health answer says the model was up when it was fetched, not that it is now.
    if (res.stale) return { rung: 'mac', available: false, reason: 'Mac not reachable' };
    if (res.status !== 200) return { rung: 'mac', available: false, reason: `health ${res.status}` };
    const health = JSON.parse(res.body) as { model_available?: boolean; context_window?: number; status?: string };
    return health.model_available
      ? { rung: 'mac', available: true, contextTokens: health.context_window }
      : { rung: 'mac', available: false, reason: health.status ?? 'model unavailable' };
  } catch {
    return { rung: 'mac', available: false, reason: 'Mac not reachable' };
  }
}

async function probeLocal(): Promise<RungStatus> {
  try {
    const res = await callMac(`${LOCAL_BASE}/ready`);
    if (res.stale) return { rung: 'local', available: false, reason: 'Mac not reachable' };
    if (res.status !== 200) return { rung: 'local', available: false, reason: `ready ${res.status}` };
    const ready = JSON.parse(res.body) as { status?: string; model?: string; max_input_bytes?: number };
    return ready.status === 'ready'
      ? {
          rung: 'local',
          available: true,
          contextTokens:
            ready.max_input_bytes === undefined
              ? undefined
              : Math.floor(ready.max_input_bytes / CHARS_PER_TOKEN),
        }
      : { rung: 'local', available: false, reason: ready.status ?? 'model unavailable' };
  } catch {
    return { rung: 'local', available: false, reason: 'Local assistant not reachable' };
  }
}

export async function probe(): Promise<Probe> {
  const [device, local, mac] = await Promise.all([probeOnDevice(), probeLocal(), probeMac()]);
  return {
    statuses: [device.onDevice, local, mac, { rung: 'rules', available: true }],
    privateCloud: device.privateCloud,
  };
}

class RungError extends Error {
  constructor(readonly code: string, message: string) {
    super(message);
  }
}

async function runOnDevice(task: ModelTask): Promise<string> {
  try {
    const result = await invoke<{ text: string }>('plugin:foundation-models|respond', {
      prompt: task.prompt,
      instructions: task.instructions ?? null,
      maxResponseTokens: task.maxResponseTokens ?? DEFAULT_REPLY_TOKENS,
    });
    return result.text;
  } catch (error) {
    const code = typeof error === 'object' && error && 'code' in error ? String(error.code) : 'generation_failed';
    const message = typeof error === 'object' && error && 'message' in error ? String(error.message) : String(error);
    throw new RungError(code, message);
  }
}

async function runLocal(task: ModelTask): Promise<string> {
  let res: MacReply;
  try {
    res = await callMac(`${LOCAL_BASE}/api/generate`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        prompt: task.prompt,
        instructions: task.instructions ?? null,
        max_tokens: task.maxResponseTokens ?? DEFAULT_REPLY_TOKENS,
      }),
    });
  } catch {
    throw new RungError('unavailable', 'Local assistant not reachable');
  }
  if (res.stale || res.status >= 500) throw new RungError('unavailable', 'Local assistant not reachable');
  if (res.status === 413) throw new RungError('context_length_exceeded', 'too long for the local assistant');
  if (res.status !== 200) throw new RungError('generation_failed', `Local assistant answered ${res.status}`);
  const body = JSON.parse(res.body) as { text?: string };
  return body.text ?? '';
}

async function runMac(task: ModelTask): Promise<string> {
  const messages = [
    ...(task.instructions ? [{ role: 'system', content: task.instructions }] : []),
    { role: 'user', content: task.prompt },
  ];
  let res: MacReply;
  try {
    res = await callMac(`${MAC_BASE}/v1/chat/completions`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        model: MAC_MODEL,
        messages,
        max_tokens: task.maxResponseTokens ?? DEFAULT_REPLY_TOKENS,
      }),
    });
  } catch {
    throw new RungError('unavailable', 'Mac not reachable');
  }
  if (res.stale) throw new RungError('unavailable', 'Mac not reachable');
  // apfel answers an over-window prompt with an OpenAI 400 `context_length_exceeded`
  // (capabilities/foundation-models/README.md, "Over-window requests now answer 400").
  if (res.status === 400) throw new RungError('context_length_exceeded', 'too long for the Mac model');
  if (res.status >= 500) throw new RungError('unavailable', `Mac model answered ${res.status}`);
  if (res.status !== 200) throw new RungError('generation_failed', `Mac model answered ${res.status}`);
  const body = JSON.parse(res.body) as { choices?: Array<{ message?: { content?: string } }> };
  return body.choices?.[0]?.message?.content ?? '';
}

/** Runs one task down the ladder. Throws only for a failure the reader should see. */
export async function generate(task: ModelTask, probed?: Probe): Promise<ModelResult> {
  const { statuses } = probed ?? (await probe());
  const { tries, skipped } = plan(task, statuses);
  for (const rung of tries) {
    if (rung === 'rules') return { text: null, source: 'rules', skipped };
    try {
      const text =
        rung === 'on-device'
          ? await runOnDevice(task)
          : rung === 'local'
            ? await runLocal(task)
            : await runMac(task);
      return { text, source: rung, skipped };
    } catch (error) {
      if (error instanceof RungError && FALL_THROUGH_CODES.has(error.code)) {
        skipped.push({ rung, reason: error.code });
        continue;
      }
      throw error;
    }
  }
  return { text: null, source: 'rules', skipped };
}
