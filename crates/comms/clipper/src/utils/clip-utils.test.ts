// @vitest-environment jsdom
import { afterEach, describe, expect, test, vi } from 'vitest';

const { parse, parseAsync } = vi.hoisted(() => ({ parse: vi.fn(), parseAsync: vi.fn() }));
vi.mock('defuddle/full', async importOriginal => ({
	createMarkdownContent: (await importOriginal<typeof import('defuddle/full')>()).createMarkdownContent,
	default: class {
		parse = parse;
		parseAsync = parseAsync;
	},
}));

import { parseForClipAsync } from './clip-utils';

function videoDocument() {
	const doc = document.implementation.createHTMLDocument();
	Object.defineProperty(doc, 'URL', { value: 'https://www.youtube.com/watch?v=abc123', configurable: true });
	return doc;
}

const parsed = { content: '<p>Description</p>', variables: { transcript: '**0:00** · Caption' } };

afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); vi.resetAllMocks(); });

describe('async clip extraction', () => {
	test('waits beyond eight seconds for YouTube captions and clears its deadline', async () => {
		vi.useFakeTimers();
		const doc = videoDocument();
		parseAsync.mockImplementation(() => new Promise(resolve => setTimeout(() => resolve(parsed), 12000)));
		const pending = parseForClipAsync(doc);
		await vi.advanceTimersByTimeAsync(12000);
		const result = await pending;
		expect(result.variables?.transcriptStatus).toBe('available');
		expect(parse).not.toHaveBeenCalled();
		expect(vi.getTimerCount()).toBe(0);
	});

	test('falls back to rendered content at thirty seconds with a missing-transcript notice', async () => {
		vi.useFakeTimers();
		vi.spyOn(console, 'warn').mockImplementation(() => undefined);
		const doc = videoDocument();
		parseAsync.mockReturnValue(new Promise(() => {}));
		parse.mockReturnValue({ ...parsed, variables: {} });
		const pending = parseForClipAsync(doc);
		await vi.advanceTimersByTimeAsync(30000);
		const result = await pending;
		expect(result.variables?.transcriptStatus).toBe('unavailable');
		expect(result.content).toContain('Transcript unavailable');
		expect(vi.getTimerCount()).toBe(0);
	});

	test('rejects results after navigation to a different video', async () => {
		const doc = videoDocument();
		parseAsync.mockImplementation(async () => {
			Object.defineProperty(doc, 'URL', { value: 'https://www.youtube.com/watch?v=other456' });
			return parsed;
		});
		await expect(parseForClipAsync(doc)).rejects.toThrow('Video changed during extraction');
	});
});
