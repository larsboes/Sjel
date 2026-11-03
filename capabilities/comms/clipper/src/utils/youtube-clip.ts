import type { DefuddleResponse } from 'defuddle';
import { createMarkdownContent } from 'defuddle/full';

/** Return the video ID for a supported YouTube video URL, or null. */
export function youtubeVideoId(url: string): string | null {
	try {
		const parsed = new URL(url);
		if (!['www.youtube.com', 'youtube.com', 'm.youtube.com', 'youtu.be'].includes(parsed.hostname)) return null;
		const id = parsed.hostname === 'youtu.be'
			? parsed.pathname.slice(1).split('/')[0]
			: parsed.pathname === '/watch'
				? parsed.searchParams.get('v')
				: parsed.pathname.match(/^\/(?:shorts|live|embed)\/([^/]+)/)?.[1];
		return id && /^[\w-]+$/.test(id) ? id : null;
	} catch {
		return null;
	}
}

/** Capture at most 50 loaded top-level YouTube comments. */
export function captureYouTubeComments(doc: Document): { comments: string; commentsCount: string } {
	const comments = doc.createElement('div');
	const seen = new Set<string>();
	const threads = youtubeVideoId(doc.URL) ? doc.querySelectorAll('ytd-comment-thread-renderer') : [];
	for (const comment of Array.from(threads).slice(0, 50)) {
		const body = comment.querySelector('[id="content-text"]');
		const text = body?.textContent?.trim();
		if (!text) continue;
		const author = comment.querySelector('[id="author-text"]')?.textContent?.trim() || '';
		const key = comment.querySelector('a[href*="lc="]')?.getAttribute('href') || `${author}\n${text}`;
		if (seen.has(key)) continue;
		seen.add(key);
		const quote = doc.createElement('blockquote');
		const attribution = doc.createElement('p');
		attribution.textContent = author;
		const paragraph = doc.createElement('p');
		paragraph.textContent = text;
		quote.append(attribution, paragraph);
		comments.append(quote);
	}
	return { comments: createMarkdownContent(comments.innerHTML, doc.URL), commentsCount: String(seen.size) };
}

/** Add YouTube section variables without adding comments to the default content. */
export function addYouTubeClipSections(doc: Document, result: DefuddleResponse, comments = captureYouTubeComments(doc)): DefuddleResponse {
	const videoId = youtubeVideoId(doc.URL);
	if (!videoId) return result;
	const content = doc.createElement('div');
	content.innerHTML = result.content;
	const description = content.cloneNode(true) as HTMLElement;
	description.querySelectorAll('iframe, video, .transcript').forEach(el => el.remove());
	const videoDescription = createMarkdownContent(description.innerHTML, doc.URL);
	const transcript = result.variables?.transcript || '';
	if (!transcript.trim()) {
		const notice = doc.createElement('p');
		notice.textContent = 'Transcript unavailable for this capture. Open the transcript on YouTube and clip again to retry.';
		content.append(notice);
	}
	return {
		...result,
		content: content.innerHTML,
		variables: {
			...result.variables,
			videoId,
			videoDescription,
			transcript,
			transcriptStatus: transcript.trim() ? 'available' : 'unavailable',
			...comments,
		},
	};
}
