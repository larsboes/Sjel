import Defuddle from 'defuddle/full';
import { setElementHTML } from './dom-utils';
import { extractAiStudio } from './ai-studio-extractor';
import { addYouTubeClipSections, captureYouTubeComments, youtubeVideoId } from './youtube-clip';

/**
 * Extract async sections for popup and direct exports, preserving reader content.
 * @throws If navigation changes the source video during extraction.
 */
export async function parseForClipAsync(doc: Document) {
	if (doc.querySelector('.obsidian-reader-active .obsidian-reader-content article')) return parseForClip(doc);
	const videoId = youtubeVideoId(doc.URL);
	const comments = captureYouTubeComments(doc);
	const defuddle = new Defuddle(doc, { url: doc.URL });
	let timeout: ReturnType<typeof setTimeout> | undefined;
	try {
		const deadline = new Promise<never>((_, reject) => {
			timeout = setTimeout(() => reject(new Error('Async extraction timed out')), videoId ? 30000 : 8000);
		});
		const result = await Promise.race([defuddle.parseAsync(), deadline]).catch(error => {
			console.warn('Sjel Clipper: async extraction failed; using rendered content', error);
			return defuddle.parse();
		});
		if (youtubeVideoId(doc.URL) !== videoId) throw new Error('Video changed during extraction. Open the clipper again.');
		return addYouTubeClipSections(doc, result, comments);
	} finally {
		clearTimeout(timeout);
	}
}

// Parse document content for clipping. In reader mode, extracts from
// the article's original HTML to avoid reader UI artifacts.
export function parseForClip(doc: Document) {
	const aiStudio = extractAiStudio(doc);
	if (aiStudio && aiStudio.turns.length > 0) {
		const parsed = new Defuddle(doc, { url: doc.URL }).parse();
		return {
			...parsed,
			title: aiStudio.title || parsed.title,
			content: aiStudio.html,
			variables: {
				...parsed.variables,
				...aiStudio.variables,
			},
		};
	}

	const readerArticle = doc.querySelector('.obsidian-reader-active .obsidian-reader-content article');
	if (readerArticle) {
		const readerDoc = doc.implementation.createHTMLDocument();
		const originalHtml = readerArticle.getAttribute('data-original-html');
		if (originalHtml) {
			setElementHTML(readerDoc.body, originalHtml);
		} else {
			readerDoc.body.replaceChildren(
				...Array.from(readerArticle.childNodes).map(n => readerDoc.importNode(n, true))
			);
		}
		return new Defuddle(readerDoc, { url: '' }).parse();
	}
	const comments = captureYouTubeComments(doc);
	return addYouTubeClipSections(doc, new Defuddle(doc, { url: doc.URL }).parse(), comments);
}
