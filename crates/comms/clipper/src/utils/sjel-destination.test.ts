import { beforeEach, describe, expect, test, vi } from 'vitest';

const store: Record<string, unknown> = {};
vi.mock('./browser-polyfill', () => ({
	default: {
		runtime: { sendNativeMessage: async () => ({ ok: false, error: 'unavailable' }) },
		storage: {
			local: {
				get: async (key: string) => (key in store ? { [key]: store[key] } : {}),
				set: async (items: Record<string, unknown>) => Object.assign(store, items),
			},
		},
	},
}));

import { buildClip, flushSjelQueue, queuedClips, saveToSjel, SjelRejected, SjelSaveInput } from './sjel-destination';

const input: SjelSaveInput = {
	url: 'https://www.reddit.com/r/rust/comments/x/',
	title: 'A thread',
	markdown: '---\ntitle: A thread\n---\nBody',
	noteName: 'A thread',
	path: 'Clippings',
	behavior: 'create',
	templateName: 'Default',
	properties: [{ name: 'title', value: 'A thread' }],
	source: { fullHtml: '<html>page</html>', highlights: [{ text: 'h' }] },
};

beforeEach(() => {
	for (const key of Object.keys(store)) delete store[key];
});

describe('Sjel destination', () => {
	test('a clip carries the store contract and the local day', () => {
		const clip = buildClip(input, 'id-1', new Date(2026, 9, 28, 23, 30));
		expect(clip.request_id).toBe('id-1');
		expect(clip.payload).toMatchObject({
			version: 1,
			behavior: 'create',
			markdown: input.markdown,
			destination: { path: 'Clippings', name: 'A thread', date: '2026-10-28' },
			source: { fullHtml: '<html>page</html>' },
		});
	});

	test('an oversized page drops its HTML before the clip is refused', () => {
		const big = buildClip({ ...input, source: { fullHtml: 'x'.repeat(3_000_000) } });
		expect(big.payload.source).toEqual({ full_html_omitted: true });
		expect(() => buildClip({ ...input, markdown: 'x'.repeat(3_000_000) })).toThrow(SjelRejected);
	});

	test('an unreachable Sjel queues the clip and a later flush delivers it once', async () => {
		const sent: any[] = [];
		const down = async () => ({ ok: false, error: 'unavailable' });
		expect(await saveToSjel(input, down)).toEqual({ status: 'queued' });
		expect(await saveToSjel(input, async () => { throw new Error('host missing'); })).toEqual({ status: 'queued' });
		expect(await queuedClips()).toEqual({ queued: 2, failed: 0 });

		const up = async (message: any) => {
			sent.push(message.clip.request_id);
			return { ok: true, status: 'created', document_key: `clip:${message.clip.request_id}` };
		};
		expect(await flushSjelQueue(up)).toBe(2);
		expect(await queuedClips()).toEqual({ queued: 0, failed: 0 });
		expect(new Set(sent).size).toBe(2);
		expect(await flushSjelQueue(up)).toBe(0);
	});

	test('a rejected clip leaves the queue for the failed list without blocking the next', async () => {
		await saveToSjel(input, async () => ({ ok: false, error: 'unavailable' }));
		await saveToSjel(input, async () => ({ ok: false, error: 'unavailable' }));
		let calls = 0;
		const delivered = await flushSjelQueue(async () =>
			++calls === 1 ? { ok: false, error: 'conflict' } : { ok: true, status: 'created' },
		);
		expect(delivered).toBe(1);
		expect(await queuedClips()).toEqual({ queued: 0, failed: 1 });
	});

	test('a direct save reports the stored document', async () => {
		const result = await saveToSjel(input, async () => ({ ok: true, status: 'created', document_key: 'clip:x' }));
		expect(result).toEqual({ status: 'created', document_key: 'clip:x' });
		await expect(saveToSjel(input, async () => ({ ok: false, error: 'invalid' }))).rejects.toThrow(SjelRejected);
	});
});
