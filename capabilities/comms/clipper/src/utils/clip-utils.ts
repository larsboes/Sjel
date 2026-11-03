import Defuddle from 'defuddle/full';
import { setElementHTML } from './dom-utils';
import { extractAiStudio } from './ai-studio-extractor';

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
	return new Defuddle(doc, { url: doc.URL }).parse();
}
