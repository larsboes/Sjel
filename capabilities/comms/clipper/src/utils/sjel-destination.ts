// Sjel destination: replaces the obsidian:// handoff with a native messaging save into
// the comms clip store (capabilities/comms/src/web_clip_socket.rs). A clip that cannot
// reach Sjel stays in a local queue under its retry key until a later attempt lands it.
import browser from './browser-polyfill';
import { Template } from '../types/types';

export const SJEL_HOST = 'dev.sjel.clip';
const QUEUE_KEY = 'sjelQueue';
const FAILED_KEY = 'sjelFailed';
// The store refuses payloads above 2 MiB; stay below it with room for JSON escaping.
const MAX_PAYLOAD_CHARS = 1_900_000;

export interface SjelClip {
	request_id: string;
	url: string;
	payload: Record<string, unknown>;
}

export interface SjelSaveResult {
	status: 'created' | 'replayed' | 'queued';
	document_key?: string;
}

export interface SjelSaveInput {
	url: string;
	title: string;
	markdown: string;
	noteName: string;
	path: string;
	behavior: Template['behavior'];
	templateName: string;
	properties: { name: string; value: string; type?: string }[];
	source?: {
		fullHtml?: string;
		selectedHtml?: string;
		highlights?: unknown[];
		schemaOrgData?: unknown;
		meta?: Record<string, unknown>;
	};
}

// A failure that a retry cannot fix: the clip is moved out of the queue and reported.
export class SjelRejected extends Error {}

function localDate(now = new Date()): string {
	const pad = (n: number) => String(n).padStart(2, '0');
	return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

export function buildClip(input: SjelSaveInput, requestId: string = crypto.randomUUID(), now = new Date()): SjelClip {
	const payload: Record<string, unknown> = {
		version: 1,
		behavior: input.behavior,
		title: input.title,
		markdown: input.markdown,
		destination: { path: input.path, name: input.noteName, date: localDate(now) },
		template: input.templateName,
		properties: input.properties,
		source: input.source ?? {},
		client: { name: 'sjel-clipper', upstream: 'obsidian-clipper@6d56d618', saved_at: now.toISOString() },
	};
	if (JSON.stringify(payload).length > MAX_PAYLOAD_CHARS) {
		// The page HTML is the only part that can be dropped without losing the clip itself.
		const { fullHtml: _dropped, ...rest } = (input.source ?? {}) as Record<string, unknown>;
		payload.source = { ...rest, full_html_omitted: true };
	}
	if (JSON.stringify(payload).length > MAX_PAYLOAD_CHARS) {
		throw new SjelRejected('too_large');
	}
	return { request_id: requestId, url: input.url, payload };
}

type Send = (message: unknown) => Promise<any>;
const nativeSend: Send = (message) => browser.runtime.sendNativeMessage(SJEL_HOST, message);

async function send(clip: SjelClip, sendMessage: Send): Promise<SjelSaveResult | null> {
	let answer: any;
	try {
		answer = await sendMessage({ type: 'save', clip });
	} catch {
		return null; // host not installed or not startable: retry later
	}
	if (answer?.ok) return { status: answer.status, document_key: answer.document_key };
	if (answer?.error === 'unavailable' || answer?.error === 'storage') return null;
	throw new SjelRejected(answer?.error ?? 'invalid');
}

async function readList(key: string): Promise<SjelClip[]> {
	const stored = await browser.storage.local.get(key);
	return (stored[key] as SjelClip[] | undefined) ?? [];
}

async function writeList(key: string, list: SjelClip[]): Promise<void> {
	await browser.storage.local.set({ [key]: list });
}

export async function queuedClips(): Promise<{ queued: number; failed: number }> {
	return { queued: (await readList(QUEUE_KEY)).length, failed: (await readList(FAILED_KEY)).length };
}

/** Save now, or queue under the same retry key when Sjel cannot be reached. */
export async function saveToSjel(input: SjelSaveInput, sendMessage: Send = nativeSend): Promise<SjelSaveResult> {
	const clip = buildClip(input);
	const result = await send(clip, sendMessage);
	if (result) {
		void flushSjelQueue(sendMessage);
		return result;
	}
	await writeList(QUEUE_KEY, [...(await readList(QUEUE_KEY)), clip]);
	return { status: 'queued' };
}

/** Retry queued clips in order. A rejected clip moves to the failed list instead of blocking the rest. */
export async function flushSjelQueue(sendMessage: Send = nativeSend): Promise<number> {
	const queue = await readList(QUEUE_KEY);
	const remaining: SjelClip[] = [];
	const failed: SjelClip[] = [];
	let delivered = 0;
	for (const clip of queue) {
		try {
			if (await send(clip, sendMessage)) delivered++;
			else remaining.push(clip);
		} catch (error) {
			failed.push({ ...clip, payload: { ...clip.payload, rejected: String((error as Error).message) } });
		}
	}
	// Re-read before writing: a save in another context may have queued a clip meanwhile.
	const queuedSince = (await readList(QUEUE_KEY)).filter((c) => !queue.some((q) => q.request_id === c.request_id));
	await writeList(QUEUE_KEY, [...remaining, ...queuedSince]);
	if (failed.length) await writeList(FAILED_KEY, [...(await readList(FAILED_KEY)), ...failed]);
	return delivered;
}
