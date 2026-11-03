// @vitest-environment jsdom
import { describe, expect, test } from 'vitest';
import Defuddle from 'defuddle/full';
import { addYouTubeClipSections, captureYouTubeComments, youtubeVideoId } from './youtube-clip';
import { buildVariables } from './shared';
import { compileTemplate } from './template-compiler';

function videoDocument(comments = '') {
	const doc = new DOMParser().parseFromString(`<!doctype html><html><head><title>A video</title>
		<script type="application/ld+json">${JSON.stringify({
			'@type': 'VideoObject', '@id': 'https://www.youtube.com/watch?v=abc123',
			name: 'A video', author: 'A channel', description: 'Full description.\n' + 'Long description. '.repeat(30),
		})}</script></head><body>
		<ytd-engagement-panel-section-list-renderer target-id="engagement-panel-searchable-transcript">
		<div id="segments-container"><ytd-transcript-segment-renderer>
		<div class="segment-timestamp">1:23</div><div class="segment-text">A transcript sentence.</div>
		</ytd-transcript-segment-renderer></div></ytd-engagement-panel-section-list-renderer>
		${comments}</body></html>`, 'text/html');
	Object.defineProperty(doc, 'URL', { value: 'https://www.youtube.com/watch?v=abc123' });
	return doc;
}

describe('YouTube clip sections', () => {
	test('recognizes video URLs without matching unrelated hosts or channel pages', () => {
		for (const url of ['https://www.youtube.com/watch?v=abc123', 'https://youtu.be/abc123', 'https://m.youtube.com/shorts/abc123', 'https://youtube.com/live/abc123']) {
			expect(youtubeVideoId(url)).toBe('abc123');
		}
		for (const url of ['https://youtube.com.evil.test/watch?v=abc123', 'https://youtube.com/@channel', 'invalid']) {
			expect(youtubeVideoId(url)).toBeNull();
		}
	});

	test('preserves full description and timestamped transcript through template compilation', async () => {
		const doc = videoDocument();
		const parsed = await new Defuddle(doc, { url: doc.URL }).parseAsync();
		const result = addYouTubeClipSections(doc, parsed);
		expect(result.variables?.transcriptStatus).toBe('available');
		expect(result.variables?.transcript).toContain('1:23');
		expect(result.variables?.videoDescription?.length).toBeGreaterThan(200);
		expect(result.variables?.videoDescription).not.toContain('A transcript sentence.');
		const variables = buildVariables({ ...result, url: doc.URL, fullHtml: doc.documentElement.outerHTML, contentHtml: result.content, extractedContent: result.variables });
		const markdown = await compileTemplate(0, '{{videoDescription}}\n\n{{transcript}}', variables, doc.URL);
		expect(markdown).toContain('Long description. '.repeat(29).trim());
		expect(markdown).toContain('**1:23** · A transcript sentence.');
	});

	test('exposes loaded top-level comments separately, escapes text, and deduplicates', () => {
		const comment = '<ytd-comment-thread-renderer><span id="author-text">@reader</span><span id="content-text">Use &lt;script&gt; safely</span><div id="replies"><span id="content-text">A reply</span></div></ytd-comment-thread-renderer>';
		const doc = videoDocument(comment.repeat(2));
		const comments = captureYouTubeComments(doc);
		const parsed = new Defuddle(doc, { url: doc.URL }).parse();
		const result = addYouTubeClipSections(doc, parsed, comments);
		expect(result.variables?.commentsCount).toBe('1');
		expect(result.variables?.comments).toContain('@reader');
		expect(result.variables?.comments).toContain('Use');
		expect(result.variables?.comments).not.toContain('A reply');
		expect(result.content).not.toContain('Use &lt;script&gt; safely');
	});

	test('bounds comment capture to 50 loaded threads', () => {
		const doc = videoDocument(Array.from({ length: 55 }, (_, i) => `<ytd-comment-thread-renderer><span id="content-text">Comment ${i}</span></ytd-comment-thread-renderer>`).join(''));
		const comments = captureYouTubeComments(doc);
		const parsed = new Defuddle(doc, { url: doc.URL }).parse();
		const result = addYouTubeClipSections(doc, parsed, comments);
		expect(result.variables?.commentsCount).toBe('50');
		expect(result.variables?.comments).not.toContain('Comment 50');
	});

	test('reports missing transcript without claiming captions are disabled', () => {
		const doc = videoDocument();
		doc.querySelector('ytd-engagement-panel-section-list-renderer')!.remove();
		const result = addYouTubeClipSections(doc, new Defuddle(doc, { url: doc.URL }).parse());
		expect(result.variables?.transcriptStatus).toBe('unavailable');
		expect(result.variables?.commentsCount).toBe('0');
		expect(result.content).toContain('Transcript unavailable for this capture.');
		expect(result.content).toContain('Full description.');
	});
});
