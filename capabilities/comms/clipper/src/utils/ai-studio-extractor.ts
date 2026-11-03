export interface ExtractedTurn {
	id?: string;
	index: number;
	prompt: string;
	thoughts?: string;
	response: string;
}

export interface AiStudioExtractResult {
	title: string;
	url: string;
	/** Raw turns retained for scan merging; markdown and HTML expose the cleaned output. */
	turns: ExtractedTurn[];
	markdown: string;
	html: string;
	variables: Record<string, string>;
}

const TEXT_NODE = 3;
const ELEMENT_NODE = 1;

/** Check if document belongs to Google AI Studio. */
export function isAiStudio(doc: Document): boolean {
	const url = doc.URL || (typeof window !== 'undefined' ? window.location.href : '');
	if (url.includes('aistudio.google.com')) return true;
	return !!doc.querySelector('ms-chat-turn, ms-user-query, ms-cmark-node, [data-testid="prompt-name"]');
}

/** Extract title from Google AI Studio page. */
export function extractAiStudioTitle(doc: Document): string {
	const candidates = [
		() => doc.querySelector('[data-testid="prompt-name"]')?.textContent,
		() => (doc.querySelector('input.name-input, input[aria-label*="Prompt title" i]') as HTMLInputElement)?.value,
		() => doc.querySelector('.prompt-name, .file-name, .header-title, .prompt-title')?.textContent,
		() => doc.querySelector('h1')?.textContent,
		() => {
			const title = doc.title || '';
			return title.replace(/\s*[-–—|]\s*Google\s*AI\s*Studio.*$/i, '').trim();
		},
	];

	for (const candidate of candidates) {
		try {
			const val = candidate();
			if (val && val.trim().length > 0) {
				return val.trim();
			}
		} catch {
			// ignore selector errors
		}
	}
	return 'Untitled Prompt';
}

/**
 * Convert a DOM node to markdown, specifically preserving footnote citation links
 * such as `<a href="...">1</a>` -> `[1](url)` and standard markdown formatting.
 */
export function nodeToMarkdownWithCitations(node: Node): string {
	if (node.nodeType === TEXT_NODE) {
		return node.textContent || '';
	}
	if (node.nodeType !== ELEMENT_NODE) {
		return '';
	}

	const el = node as HTMLElement;
	const tag = el.tagName.toLowerCase();

	// Check if this element is an expanded shadow root from flatten-shadow-dom.js
	const shadowHtml = el.getAttribute('data-defuddle-shadow');
	if (shadowHtml) {
		const tempDoc = el.ownerDocument.implementation.createHTMLDocument();
		tempDoc.body.innerHTML = shadowHtml;
		const shadowMd = Array.from(tempDoc.body.childNodes)
			.map(n => nodeToMarkdownWithCitations(n))
			.join('');
		if (shadowMd.trim().length > 0) return shadowMd;
	}

	// Handle KaTeX / MathML formulas to avoid duplicated text (e.g. mathml + html)
	if (el.classList?.contains('katex')) {
		const annotation = el.querySelector('annotation[encoding="application/x-tex"]');
		if (annotation && annotation.textContent?.trim()) {
			return annotation.textContent.trim();
		}
		const mathml = el.querySelector('.katex-mathml');
		if (mathml && mathml.textContent?.trim()) {
			return mathml.textContent.trim();
		}
		const htmlPart = el.querySelector('.katex-html');
		if (htmlPart && htmlPart.textContent?.trim()) {
			return htmlPart.textContent.trim();
		}
		return el.textContent?.trim() || '';
	}
	if (el.classList?.contains('katex-html') && (el.parentElement?.classList?.contains('katex') || el.getAttribute('aria-hidden') === 'true')) {
		return '';
	}
	if (el.classList?.contains('katex-mathml') && el.parentElement?.classList?.contains('katex')) {
		return '';
	}

	// Footnote citation anchor or general link
	if (tag === 'a' || el.hasAttribute('data-href') || el.hasAttribute('data-url')) {
		let href = el.getAttribute('href') || el.getAttribute('data-href') || el.getAttribute('data-url') || '';
		const innerText = el.textContent?.trim() || '';
		if (!href) return innerText;

		// Unwrap Google search redirect URLs: https://www.google.com/url?q=<target>&...
		if (href.includes('google.com/url?') || href.includes('google.de/url?') || href.includes('/url?q=')) {
			try {
				const u = new URL(href);
				const realUrl = u.searchParams.get('q') || u.searchParams.get('url');
				if (realUrl) {
					href = decodeURIComponent(realUrl);
				}
			} catch {
				// keep href
			}
		}

		// If this is a citation marker like "1" or "[1]"
		const numMatch = innerText.match(/^\[?(\d+)\]?$/);
		if (numMatch) {
			return `[${numMatch[1]}](${href})`;
		}
		return `[${innerText}](${href})`;
	}

	// Code block
	if (tag === 'pre') {
		const codeEl = el.querySelector('code');
		const codeText = codeEl ? codeEl.textContent : el.textContent;
		const lang = codeEl?.className.match(/language-([a-z0-9_-]+)/i)?.[1] || '';
		return `\n\`\`\`${lang}\n${codeText || ''}\n\`\`\`\n`;
	}

	// Inline code
	if (tag === 'code') {
		return `\`${el.textContent || ''}\``;
	}

	// Formatting
	if (tag === 'strong' || tag === 'b') {
		return `**${childrenToMarkdown(el)}**`;
	}
	if (tag === 'em' || tag === 'i') {
		return `*${childrenToMarkdown(el)}*`;
	}
	if (tag === 'del' || tag === 's') {
		return `~~${childrenToMarkdown(el)}~~`;
	}

	// Block elements
	if (tag === 'p' || tag === 'div' || tag === 'section') {
		const text = childrenToMarkdown(el).trim();
		return text ? `\n\n${text}\n\n` : '';
	}
	if (tag === 'blockquote') {
		const text = childrenToMarkdown(el).trim();
		return text ? `\n> ${text.replace(/\n/g, '\n> ')}\n` : '';
	}
	if (tag === 'h1' || tag === 'h2' || tag === 'h3' || tag === 'h4' || tag === 'h5' || tag === 'h6') {
		const level = parseInt(tag.charAt(1), 10) || 2;
		const hashes = '#'.repeat(level);
		return `\n\n${hashes} ${childrenToMarkdown(el).trim()}\n\n`;
	}
	if (tag === 'li') {
		return `\n- ${childrenToMarkdown(el).trim()}`;
	}
	if (tag === 'ul' || tag === 'ol') {
		return `\n${childrenToMarkdown(el)}\n`;
	}
	if (tag === 'br') {
		return '\n';
	}
	if (tag === 'hr') {
		return '\n\n---\n\n';
	}

	// Table elements
	if (tag === 'table') {
		const rows = Array.from(el.querySelectorAll('tr'));
		if (rows.length === 0) return '';

		let tableMd = '\n\n';
		let isFirstRow = true;

		for (const row of rows) {
			// AI Studio wraps each cell in an ms-cmark-node, so cells are not direct row children.
			const cells = Array.from(row.querySelectorAll('th, td')).filter(c => c.closest('tr') === row);
			if (cells.length === 0) continue;

			const rowText = cells
				.map(cell => childrenToMarkdown(cell as HTMLElement).trim().replace(/\n+/g, ' '))
				.join(' | ');

			tableMd += `| ${rowText} |\n`;

			if (isFirstRow) {
				const separator = cells.map(() => '---').join(' | ');
				tableMd += `| ${separator} |\n`;
				isFirstRow = false;
			}
		}
		return tableMd + '\n';
	}
	if (tag === 'tr' || tag === 'thead' || tag === 'tbody' || tag === 'th' || tag === 'td') {
		return '';
	}

	const md = childrenToMarkdown(el);
	if (!md && el.textContent && !['script', 'style', 'noscript'].includes(tag)) {
		return el.textContent;
	}
	return md;
}

function childrenToMarkdown(el: HTMLElement): string {
	return Array.from(el.childNodes)
		.map(n => nodeToMarkdownWithCitations(n))
		.join('');
}

function topLevelCmarkNodes(root: Element): Element[] {
	return Array.from(root.querySelectorAll('ms-cmark-node')).filter(node => {
		let parent = node.parentElement;
		while (parent && parent !== root) {
			if (parent.tagName.toLowerCase() === 'ms-cmark-node') return false;
			parent = parent.parentElement;
		}
		return true;
	});
}

/** Detect if an element represents a user message, a model message, or a combined turn container. */
export function detectRole(el: Element, previousRole?: 'user' | 'model'): 'user' | 'model' | 'combined' {
	// 0. Check for combined container (contains both user query and model response)
	const rawText = (el.textContent || '').trim();
	const hasUserHeader = /\bUser\s*[•·]/i.test(rawText) || !!el.querySelector('[data-turn-role="User" i], ms-user-query, .user-query');
	const hasModelHeader = /\bModel\s*[•·]/i.test(rawText) || !!el.querySelector('[data-turn-role="Model" i], ms-thought-chunk, .model-response');
	if (hasUserHeader && hasModelHeader) {
		return 'combined';
	}

	// 1. Explicit data-turn-role attribute (AI Studio Web Component standard)
	const turnRole = el.getAttribute('data-turn-role') || el.querySelector('[data-turn-role]')?.getAttribute('data-turn-role');
	if (turnRole) {
		if (/model/i.test(turnRole)) return 'model';
		if (/user/i.test(turnRole)) return 'user';
	}

	// 2. Definite Model indicators
	// Model turns have thoughts chunks or grounding redirect links
	if (
		el.querySelector('ms-thought-chunk, .thoughts-container, .thought-content, .thoughts, [aria-label*="thought" i]') ||
		el.classList.contains('model') ||
		el.classList.contains('model-turn') ||
		el.classList.contains('model-response') ||
		el.getAttribute('data-role') === 'model' ||
		el.querySelector('[data-role="model"]') ||
		el.querySelector('a[href*="grounding-api-redirect"], a[href*="google.com/url?"]')
	) {
		return 'model';
	}

	// 3. Definite User indicators
	if (
		el.querySelector('ms-user-query, .user-query, .user-prompt') ||
		el.classList.contains('user') ||
		el.classList.contains('user-turn') ||
		el.classList.contains('user-prompt') ||
		el.getAttribute('data-role') === 'user' ||
		el.querySelector('[data-role="user"]')
	) {
		return 'user';
	}

	// 4. Text content prefixes: In AI Studio, the author header starts with "User •" or "Model •"
	const prefix = rawText.slice(0, 120);
	if (/^Model\s*[•·\s]/i.test(prefix) || /\bModel\s*[•·]/i.test(prefix)) {
		return 'model';
	}
	if (/^User\s*[•·\s]/i.test(prefix) || /\bUser\s*[•·]/i.test(prefix)) {
		return 'user';
	}

	// 5. Specific author element check (avoid generic div/span!)
	const authorEl = el.querySelector(
		'.author, .author-label, .turn-author, .turn-header, ms-turn-header, .role-label, ms-author'
	);
	if (authorEl) {
		const aText = (authorEl.textContent || '').trim();
		if (/\bModel\b/i.test(aText)) return 'model';
		if (/\bUser\b/i.test(aText)) return 'user';
	}

	// 6. Continuation fallback: If this chunk has no explicit author, it continues the preceding role
	if (previousRole) {
		return previousRole;
	}

	return 'user';
}

function isComposerPlaceholder(text: string): boolean {
	const t = text.trim();
	return (
		t === '' ||
		/^Start typing a prompt/i.test(t) ||
		/^Type something/i.test(t) ||
		/^Enter a prompt/i.test(t)
	);
}

/** Extract user prompt text from a user message element. */
export function extractUserContent(el: Element): string {
	// 1. Explicit user query containers
	const userSection = el.querySelector('ms-user-query, .user-query, .user-prompt, [data-turn-role="User" i]');
	if (userSection) {
		const cmarks = topLevelCmarkNodes(userSection);
		if (cmarks.length > 0) {
			const md = cmarks
				.map(n => nodeToMarkdownWithCitations(n).trim())
				.filter(s => s.length > 0)
				.join('\n\n');
			if (md.length > 0 && !isComposerPlaceholder(md)) return md;
		}

		const md = nodeToMarkdownWithCitations(userSection).trim();
		if (md.length > 0 && !isComposerPlaceholder(md)) return md;

		const raw = (userSection.textContent || '').trim();
		if (raw.length > 0 && !isComposerPlaceholder(raw)) return raw;
	}

	// 2. Cmark nodes outside model thoughts and model responses
	const thoughtsEl = el.querySelector('ms-thought-chunk, .thoughts-container, .thought-content, .thoughts');
	const modelSection = el.querySelector('.model-response, [data-role="model"], [data-turn-role="Model" i]');

	const allCmarkNodes = topLevelCmarkNodes(el);
	const targetNodes = allCmarkNodes.filter(node => {
		if (thoughtsEl && thoughtsEl.contains(node)) return false;
		if (modelSection && modelSection.contains(node)) return false;
		return true;
	});

	if (targetNodes.length > 0) {
		const md = targetNodes
			.map(n => nodeToMarkdownWithCitations(n).trim())
			.filter(s => s.length > 0)
			.join('\n\n');
		if (md.length > 0 && !isComposerPlaceholder(md)) return md;
	}

	// 3. Fallback to inputs, text areas or text chunks
	const fallbackInput = el.querySelector('textarea, [role="textbox"], ms-text-chunk, [contenteditable="true"]');
	if (fallbackInput) {
		const md = nodeToMarkdownWithCitations(fallbackInput).trim();
		if (md.length > 0 && !isComposerPlaceholder(md)) return md;
	}

	// 4. Fallback: clone element and strip header and model parts
	const clone = el.cloneNode(true) as HTMLElement;
	if (thoughtsEl) {
		clone.querySelector('ms-thought-chunk, .thoughts-container, .thought-content, .thoughts')?.remove();
	}
	if (modelSection) {
		clone.querySelector('.model-response, [data-role="model"], [data-turn-role="Model" i]')?.remove();
	}
	clone.querySelectorAll(
		'.author, .author-label, .turn-author, .turn-header, ms-turn-header, .header, mat-icon, button'
	).forEach(h => h.remove());

	let text = nodeToMarkdownWithCitations(clone).trim();
	if (!text) {
		text = (clone.textContent || '').trim();
	}

	text = text.replace(/^\s*User\s*[•·]\s*\d+:\d+\s*[AP]M\s*/i, '');
	text = text.replace(/^\s*User\s*/i, '');
	text = text.trim();

	if (isComposerPlaceholder(text)) return '';
	return text;
}

/** Extract model thoughts and response text from a model message element. */
export function extractModelContent(el: Element): { thoughts?: string; response: string } {
	// 1. Locate thoughts element
	let thoughtsEl: HTMLElement | null = el.querySelector(
		'ms-thought-chunk, .thoughts-container, .thought-content, .thoughts, [aria-label*="thought" i]'
	);
	if (!thoughtsEl) {
		const btn = el.querySelector('button, summary');
		if (btn && /thoughts/i.test(btn.textContent || '')) {
			thoughtsEl = (btn.closest('details, .thought-content, .thoughts') as HTMLElement) || (btn.parentElement as HTMLElement);
		}
	}

	let thoughts: string | undefined;
	if (thoughtsEl) {
		const thoughtsCmarks = topLevelCmarkNodes(thoughtsEl);
		let tText = thoughtsCmarks.length > 0
			? thoughtsCmarks.map(n => nodeToMarkdownWithCitations(n).trim()).filter(s => s.length > 0).join('\n\n')
			: nodeToMarkdownWithCitations(thoughtsEl);

		tText = tText
			.replace(/^[✦\s*|\\]*Thoughts[✦\s*|\\]*/i, '')
			.replace(/^[✦\s*|\\]*Expand to view model thoughts[✦\s*|\\]*/i, '')
			.replace(/^[✦\s*|\\]*Hide model thoughts[✦\s*|\\]*/i, '')
			.replace(/^\\\*\|\*|\*\|\*|\|\*/g, '')
			.trim();
		if (tText.length > 0) {
			thoughts = tText;
		}
	}

	// 2. Locate response content: ms-cmark-node elements OUTSIDE the thoughts element AND outside user query
	const userSection = el.querySelector('ms-user-query, .user-query, .user-prompt');
	const allCmarkNodes = topLevelCmarkNodes(el);
	const responseCmarkNodes = allCmarkNodes.filter(node => {
		if (thoughtsEl && thoughtsEl.contains(node)) return false;
		if (userSection && userSection.contains(node)) return false;
		return true;
	});

	let response = '';
	if (responseCmarkNodes.length > 0) {
		response = responseCmarkNodes
			.map(n => nodeToMarkdownWithCitations(n).trim())
			.filter(s => s.length > 0)
			.join('\n\n');
	} else {
		// Fallback: clone, remove thoughts and header, convert
		const clone = el.cloneNode(true) as HTMLElement;
		if (thoughtsEl) {
			const cloneThoughts = clone.querySelector(
				'ms-thought-chunk, .thoughts-container, .thought-content, .thoughts, [aria-label*="thought" i]'
			);
			if (cloneThoughts) cloneThoughts.remove();
		}
		if (userSection) {
			const cloneUser = clone.querySelector('ms-user-query, .user-query, .user-prompt');
			if (cloneUser) cloneUser.remove();
		}
		clone.querySelectorAll(
			'.author, .author-label, .turn-header, ms-turn-header, .header, mat-icon, button'
		).forEach(h => h.remove());

		let text = nodeToMarkdownWithCitations(clone);
		text = text.replace(/^\s*Model\s*[•·]\s*\d+:\d+\s*[AP]M\s*/i, '');
		text = text.replace(/^\s*Model\s*/i, '');
		response = text.trim();
	}

	return { thoughts, response };
}

/** Extract turns from Google AI Studio. */
export function extractAiStudioTurns(doc: Document): ExtractedTurn[] {
	// 0. High-precision extraction via AI Studio's built-in turn navigation scrollbar
	const scrollbarButtons = Array.from(
		doc.querySelectorAll<HTMLButtonElement>('ms-items-scrollbar button[aria-controls]')
	);
	if (scrollbarButtons.length > 0) {
		const turns: ExtractedTurn[] = [];
		for (let i = 0; i < scrollbarButtons.length; i++) {
			const btn = scrollbarButtons[i];
			const userTurnId = btn.getAttribute('aria-controls');
			const userTurnEl = userTurnId ? doc.getElementById(userTurnId) : null;
			const prompt = userTurnEl ? extractUserContent(userTurnEl).trim() : '';

			let thoughts = '';
			let response = '';

			let sibling = userTurnEl ? (userTurnEl.nextElementSibling as HTMLElement | null) : null;
			while (sibling && sibling.tagName.toLowerCase() === 'ms-chat-turn') {
				if (sibling.id && doc.querySelector(`ms-items-scrollbar button[aria-controls="${sibling.id}"]`)) {
					// Hit next user turn
					break;
				}
				const model = extractModelContent(sibling);
				if (model.thoughts) {
					thoughts = thoughts ? thoughts + '\n\n' + model.thoughts : model.thoughts;
				}
				if (model.response) {
					response = response ? response + '\n\n' + model.response : model.response;
				}
				sibling = sibling.nextElementSibling as HTMLElement | null;
			}

			// ONLY push if this turn has actual content mounted in the DOM
			if (prompt.length > 0 || response.trim().length > 0 || thoughts.trim().length > 0) {
				turns.push({
					id: userTurnId || undefined,
					index: i + 1,
					prompt,
					thoughts: thoughts.trim() ? thoughts.trim() : undefined,
					response: response.trim(),
				});
			}
		}
		if (turns.length > 0) {
			return turns;
		}
	}

	// 1. Collect all candidate message turn elements
	let messageElements = Array.from(doc.querySelectorAll('ms-chat-turn'));

	// 2. If no <ms-chat-turn> found, search for message containers by role or classes
	if (messageElements.length === 0) {
		messageElements = Array.from(
			doc.querySelectorAll(
				'[data-role="user"], [data-role="model"], .chat-turn, .user-turn, .model-turn, .chat-message'
			)
		);
	}

	// 3. Fallback: Find message containers by "User •" and "Model •" author headers
	if (messageElements.length === 0) {
		const allDivs = Array.from(doc.querySelectorAll('div, section, article'));
		const headerEls = allDivs.filter(el => {
			const text = el.textContent || '';
			return (
				(/^(User|Model)\s*[•·]\s*\d+:\d+\s*[AP]M/i.test(text.trim()) ||
				/^(User|Model)\b/i.test(text.trim())) &&
				el.children.length <= 3
			);
		});

		const seen = new Set<Element>();
		for (const h of headerEls) {
			let container: HTMLElement | null = h.parentElement;
			while (container && container !== doc.body && container.children.length === 1) {
				container = container.parentElement;
			}
			if (container && container !== doc.body && !seen.has(container)) {
				seen.add(container);
				messageElements.push(container as any);
			}
		}
	}

	const turns: ExtractedTurn[] = [];
	let currentTurn: ExtractedTurn | null = null;
	let lastRole: 'user' | 'model' = 'user';

	for (let i = 0; i < messageElements.length; i++) {
		const el = messageElements[i];
		const role = detectRole(el, lastRole);

		if (role === 'combined') {
			const prompt = extractUserContent(el);
			const { thoughts, response } = extractModelContent(el);
			if (!prompt && !thoughts && !response) continue;

			if (currentTurn) {
				turns.push(currentTurn);
				currentTurn = null;
			}
			turns.push({
				index: turns.length + 1,
				prompt,
				thoughts,
				response,
			});
			lastRole = 'model';
		} else if (role === 'user') {
			const promptText = extractUserContent(el);
			if (!promptText) {
				// Ignore empty user turn or draft composer box at the bottom
				continue;
			}
			if (currentTurn) {
				// If current turn already has response or thoughts, this starts a new dialogue turn!
				if (currentTurn.response.length > 0 || (currentTurn.thoughts && currentTurn.thoughts.length > 0)) {
					turns.push(currentTurn);
					currentTurn = {
						index: turns.length + 1,
						prompt: promptText,
						response: '',
					};
				} else {
					// User continuation chunk within the same prompt
					currentTurn.prompt = currentTurn.prompt
						? currentTurn.prompt + '\n\n' + promptText
						: promptText;
				}
			} else {
				currentTurn = {
					index: turns.length + 1,
					prompt: promptText,
					response: '',
				};
			}
			lastRole = 'user';
		} else if (role === 'model') {
			const { thoughts, response } = extractModelContent(el);
			if (!thoughts && !response) {
				// Ignore empty model chunk
				continue;
			}
			if (!currentTurn) {
				currentTurn = {
					index: turns.length + 1,
					prompt: '',
					thoughts,
					response,
				};
			} else {
				if (thoughts && thoughts.trim().length > 0) {
					currentTurn.thoughts = currentTurn.thoughts
						? currentTurn.thoughts + '\n\n' + thoughts
						: thoughts;
				}
				if (response && response.trim().length > 0) {
					currentTurn.response = currentTurn.response
						? currentTurn.response + '\n\n' + response
						: response;
				}
			}
			lastRole = 'model';
		}
	}

	if (currentTurn && (currentTurn.prompt.trim().length > 0 || currentTurn.response.trim().length > 0 || currentTurn.thoughts?.trim().length)) {
		turns.push(currentTurn);
	}

	// Re-index turns cleanly 1..N
	turns.forEach((t, i) => {
		t.index = i + 1;
	});

	return turns;
}

export function getAiStudioConversationKey(href: string): string {
	const url = new URL(href);
	return `${url.origin}${url.pathname}`;
}

const MIN_DEDUPLICATED_BLOCK_LENGTH = 200;

function splitMarkdownBlocks(markdown: string): string[] {
	const blocks: string[] = [];
	let lines: string[] = [];
	let fence: { character: string; length: number } | null = null;

	const flush = () => {
		const block = lines.join('\n');
		if (block.trim()) blocks.push(block);
		lines = [];
	};

	for (const line of markdown.split('\n')) {
		const fenceMatch = unquoteMarkdownLine(line).match(/^\s*(`{3,}|~{3,})/);
		if (fenceMatch) {
			const marker = fenceMatch[1];
			if (!fence) {
				fence = { character: marker[0], length: marker.length };
			} else if (marker[0] === fence.character && marker.length >= fence.length) {
				fence = null;
			}
			lines.push(line);
			continue;
		}

		if (!fence && !line.trim()) {
			flush();
		} else {
			lines.push(line);
		}
	}
	flush();
	return blocks;
}

function unquoteMarkdownLine(line: string): string {
	return line.replace(/^\s*(?:>\s*)+/, '').trimStart();
}

function isStructuredMarkdownBlock(block: string): boolean {
	return block.split('\n').some(line => {
		const unquoted = unquoteMarkdownLine(line);
		return /^\s*(`{3,}|~{3,}|\|)/.test(unquoted) || /^\s*#{1,6}\s/.test(unquoted);
	});
}

function deduplicateMarkdownBlocks(markdown: string): string {
	const seen = new Set<string>();
	const retained: string[] = [];

	for (const block of splitMarkdownBlocks(markdown)) {
		const key = block.trim().replace(/\s+/g, ' ');
		if (key.length < MIN_DEDUPLICATED_BLOCK_LENGTH || isStructuredMarkdownBlock(block)) {
			retained.push(block);
			continue;
		}
		if (seen.has(key)) continue;
		seen.add(key);
		retained.push(block);
	}

	return retained.join('\n\n');
}

/** Returns turns with repeated long prose blocks removed from prompts and responses. */
export function cleanAiStudioTurns(turns: ExtractedTurn[]): ExtractedTurn[] {
	return turns.map(turn => ({
		...turn,
		prompt: deduplicateMarkdownBlocks(turn.prompt),
		response: deduplicateMarkdownBlocks(turn.response),
	}));
}

/** Formats turns into canonical Sjel Markdown dialogue. */
export function turnsToMarkdown(turns: ExtractedTurn[]): string {
	return cleanAiStudioTurns(turns)
		.map(turn => {
			let md = `## Prompt ${turn.index}\n${turn.prompt || ''}\n`;
			if (turn.thoughts && turn.thoughts.trim().length > 0) {
				const quotedThoughts = turn.thoughts
					.trim()
					.split('\n')
					.map(l => `> ${l}`)
					.join('\n');
				md += `\n> [!NOTE] Thoughts\n${quotedThoughts}\n`;
			}
			md += `\n## Response ${turn.index}\n${turn.response || ''}\n`;
			return md;
		})
		.join('\n---\n\n');
}

/** Formats turns into clean HTML representation. */
export function turnsToHtml(turns: ExtractedTurn[]): string {
	turns = cleanAiStudioTurns(turns);
	const escape = (s: string) =>
		s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

	return turns
		.map(turn => {
			let html = `<div class="chat-turn" data-turn-index="${turn.index}">\n`;
			html += `  <h2>Prompt ${turn.index}</h2>\n`;
			const promptParagraphs = turn.prompt
				.split(/\n\s*\n/)
				.map(p => `<p>${escape(p).replace(/\n/g, '<br/>')}</p>`)
				.join('\n');
			html += `  <div class="user-query">${promptParagraphs}</div>\n`;

			if (turn.thoughts && turn.thoughts.trim().length > 0) {
				const thoughtParagraphs = turn.thoughts
					.split(/\n\s*\n/)
					.map(p => `<p>${escape(p).replace(/\n/g, '<br/>')}</p>`)
					.join('\n');
				html += `  <blockquote class="model-thoughts"><p><strong>Thoughts</strong></p>${thoughtParagraphs}</blockquote>\n`;
			}

			if (turn.response && turn.response.trim().length > 0) {
				html += `  <h2>Response ${turn.index}</h2>\n`;
				// Convert markdown links back to real HTML links so Defuddle preserves them
				let respHtml = escape(turn.response);
				respHtml = respHtml.replace(/\[([^\]]+)\]\((https?:\/\/[^\s\)]+)\)/g, '<a href="$2">$1</a>');
				const respParagraphs = respHtml
					.split(/\n\s*\n/)
					.map(p => `<p>${p.replace(/\n/g, '<br/>')}</p>`)
					.join('\n');
				html += `  <div class="model-response">${respParagraphs}</div>\n`;
			}
			html += `</div>\n`;
			return html;
		})
		.join('<hr/>\n');
}

/**
 * Locate the scrolling container of the AI Studio chat session.
 */
export function findChatScrollContainer(doc: Document): HTMLElement | null {
	// 1. Explicit AI Studio autoscroll container, CDK viewport or chat container
	const explicit = doc.querySelector<HTMLElement>(
		'ms-autoscroll-container, cdk-virtual-scroll-viewport, .chat-session-content, .chat-viewport, .chat-history, ms-chat-session, .chat-scroll-container'
	);
	if (explicit) return explicit;

	// 2. Look for scrollable parent of ms-chat-turn
	const turn = doc.querySelector('ms-chat-turn, [data-turn-role]');
	let parent = turn ? turn.parentElement : null;
	const win = doc.defaultView || (typeof window !== 'undefined' ? window : null);
	while (parent && parent !== doc.body && parent !== doc.documentElement) {
		if (win) {
			try {
				const style = win.getComputedStyle(parent);
				const overflowY = style.overflowY;
				if ((overflowY === 'auto' || overflowY === 'scroll') && parent.scrollHeight > parent.clientHeight) {
					return parent;
				}
			} catch {
				// ignore
			}
		}
		if (parent.scrollHeight > parent.clientHeight && parent.clientHeight > 200) {
			return parent;
		}
		parent = parent.parentElement;
	}

	// 3. Fallback to scrollingElement or documentElement
	return (doc.scrollingElement as HTMLElement) || doc.documentElement || doc.body;
}

/** Safely dispatches a scroll event without failing cross-realm checks */
function safeDispatchScroll(el: HTMLElement | null): void {
	if (!el) return;
	try {
		const doc = el.ownerDocument;
		const win = doc?.defaultView || (typeof window !== 'undefined' ? window : null);
		const Evt = (win && (win as any).Event) ? (win as any).Event : Event;
		el.dispatchEvent(new Evt('scroll', { bubbles: true }));
	} catch {
		try {
			const evt = el.ownerDocument.createEvent('Event');
			evt.initEvent('scroll', true, true);
			el.dispatchEvent(evt);
		} catch {
			// ignore
		}
	}
}

/** Safely clicks an element and triggers pointer/mouse events for Angular */
function safeClick(el: HTMLElement | null): void {
	if (!el) return;
	try {
		el.click();
	} catch {
		// ignore
	}
	try {
		const doc = el.ownerDocument;
		const win = doc?.defaultView || (typeof window !== 'undefined' ? window : null);
		const MouseEvt = (win && (win as any).MouseEvent) ? (win as any).MouseEvent : MouseEvent;
		el.dispatchEvent(new MouseEvt('click', { bubbles: true, cancelable: true, view: win as any }));
	} catch {
		// ignore
	}
}

/**
 * Merges newly observed raw turns by ID or normalized content, comparing cleaned lengths.
 * Cleanup is deferred to output formatting so later scans retain full source text.
 */
export function mergeTurns(existingTurns: ExtractedTurn[], newTurns: ExtractedTurn[]): ExtractedTurn[] {
	const result: ExtractedTurn[] = existingTurns.map(t => ({ ...t }));
	const normalize = (s: string) => (s || '').trim().replace(/\s+/g, ' ').slice(0, 150);

	for (const newTurn of newTurns) {
		// Only consider new turns that have content
		if (!newTurn.prompt?.trim() && !newTurn.response?.trim() && !newTurn.thoughts?.trim()) {
			continue;
		}

		const normNewPrompt = normalize(newTurn.prompt);
		const normNewResp = normalize(newTurn.response);

		const matchIndex = result.findIndex(existing => {
			// 1. Stable turn ID match (exact match for Google AI Studio)
			if (newTurn.id && existing.id) {
				return newTurn.id === existing.id;
			}

			const normExPrompt = normalize(existing.prompt);
			const normExResp = normalize(existing.response);

			// 2. Prompt match
			if (normNewPrompt && normExPrompt) {
				if (normNewPrompt === normExPrompt ||
					normNewPrompt.startsWith(normExPrompt.slice(0, 50)) ||
					normExPrompt.startsWith(normNewPrompt.slice(0, 50))) {
					return true;
				}
			}

			// 3. Response match
			if (normNewResp && normExResp) {
				if (normNewResp === normExResp ||
					normNewResp.startsWith(normExResp.slice(0, 50)) ||
					normExResp.startsWith(normNewResp.slice(0, 50))) {
					return true;
				}
			}

			// 4. Do NOT match if prompts or responses explicitly differ
			if (normNewPrompt && normExPrompt && normNewPrompt !== normExPrompt) {
				return false;
			}

			// 5. Match by index only if neither has conflicting prompt or response
			if (existing.index === newTurn.index) {
				return true;
			}

			return false;
		});

		if (matchIndex >= 0) {
			const existing = result[matchIndex];
			if (newTurn.id) {
				existing.index = newTurn.index;
				if (!existing.id) existing.id = newTurn.id;
			}
			if (deduplicateMarkdownBlocks(newTurn.response).length > deduplicateMarkdownBlocks(existing.response).length) {
				existing.response = newTurn.response;
			}
			if ((newTurn.thoughts?.length || 0) > (existing.thoughts?.length || 0)) {
				existing.thoughts = newTurn.thoughts;
			}
			if (deduplicateMarkdownBlocks(newTurn.prompt).length > deduplicateMarkdownBlocks(existing.prompt).length) {
				existing.prompt = newTurn.prompt;
			}
		} else {
			result.push({ ...newTurn });
		}
	}

	// Stable AI Studio IDs carry source indices across partial scans; fallback captures are local.
	const hasStableSourceIndices = existingTurns.some(turn => turn.id) || newTurns.some(turn => turn.id);
	if (hasStableSourceIndices) {
		result.sort((a, b) => a.index - b.index);
	} else {
		result.forEach((turn, index) => {
			turn.index = index + 1;
		});
	}

	return result;
}

/** Merges turns only when both observations belong to the same conversation URL. */
export function mergeTurnsForConversation(
	existingTurns: ExtractedTurn[],
	newTurns: ExtractedTurn[],
	existingConversationKey: string,
	newConversationKey: string,
): ExtractedTurn[] {
	return mergeTurns(
		existingConversationKey === newConversationKey ? existingTurns : [],
		newTurns,
	);
}

/**
 * Smoothly scrolls through Google AI Studio chat container in the active document,
 * triggering Angular CDK virtual scrolling to mount all turns, extracting them step-by-step,
 * and restoring the original scroll position.
 */
export async function scanFullAiStudioChat(doc: Document): Promise<ExtractedTurn[]> {
	const delay = (ms: number) => new Promise(res => setTimeout(res, ms));
	const container = findChatScrollContainer(doc);
	const initialScrollTop = container ? container.scrollTop : 0;

	// Check if AI Studio's turn navigation scrollbar is available
	const scrollbarButtons = Array.from(
		doc.querySelectorAll<HTMLButtonElement>('ms-items-scrollbar button[aria-controls]')
	);

	let accumulatedTurns = extractAiStudioTurns(doc);

	try {
		if (scrollbarButtons.length > 0) {
			// Strategy A: Direct Turn Jump via AI Studio's built-in turn navigation dots
			for (let i = 0; i < scrollbarButtons.length; i++) {
				const btn = scrollbarButtons[i];
				const targetTurnId = btn.getAttribute('aria-controls');
				const targetEl = targetTurnId ? doc.getElementById(targetTurnId) : null;

				// 1. Click the navigation dot to jump
				safeClick(btn);

				// 2. Ensure target turn element is scrolled into view
				if (targetEl && typeof targetEl.scrollIntoView === 'function') {
					targetEl.scrollIntoView({ behavior: 'instant', block: 'start' });
				}

				// Dispatch scroll on container
				safeDispatchScroll(container);

				// Wait for Angular virtualizer to mount the turn content
				await delay(250);

				// Accumulate turns
				accumulatedTurns = mergeTurns(accumulatedTurns, extractAiStudioTurns(doc));

				// 3. If there are model response turns, also scroll them into view
				let sibling = targetEl ? (targetEl.nextElementSibling as HTMLElement | null) : null;
				while (sibling && sibling.tagName.toLowerCase() === 'ms-chat-turn') {
					if (sibling.id && doc.querySelector(`ms-items-scrollbar button[aria-controls="${sibling.id}"]`)) {
						break;
					}
					if (typeof sibling.scrollIntoView === 'function') {
						sibling.scrollIntoView({ behavior: 'instant', block: 'center' });
					}
					safeDispatchScroll(container);
					await delay(150);
					accumulatedTurns = mergeTurns(accumulatedTurns, extractAiStudioTurns(doc));
					sibling = sibling.nextElementSibling as HTMLElement | null;
				}
			}
		}

		// Check if any turns are still incomplete or missing
		const stillMissing = scrollbarButtons.length > 0
			? scrollbarButtons.length > accumulatedTurns.filter(t => t.prompt.trim() && t.response.trim()).length
			: false;

		if ((scrollbarButtons.length === 0 || stillMissing) && container) {
			// Strategy B: Incremental container scrolling
			container.scrollTop = 0;
			safeDispatchScroll(container);
			await delay(200);
			accumulatedTurns = mergeTurns(accumulatedTurns, extractAiStudioTurns(doc));

			const viewportHeight = container.clientHeight || 800;
			const step = Math.max(300, Math.floor(viewportHeight * 0.75));
			const maxScroll = Math.max(0, container.scrollHeight - viewportHeight);

			let currentPos = 0;
			while (currentPos < maxScroll) {
				currentPos = Math.min(currentPos + step, maxScroll);
				container.scrollTop = currentPos;
				safeDispatchScroll(container);
				await delay(200);
				accumulatedTurns = mergeTurns(accumulatedTurns, extractAiStudioTurns(doc));
			}

			// Final check at bottom
			container.scrollTop = container.scrollHeight;
			safeDispatchScroll(container);
			await delay(200);
			accumulatedTurns = mergeTurns(accumulatedTurns, extractAiStudioTurns(doc));
		}
	} catch (err) {
		console.warn('[scanFullAiStudioChat] Error during scroll scan:', err);
		accumulatedTurns = mergeTurns(accumulatedTurns, extractAiStudioTurns(doc));
	} finally {
		// Restore user's original scroll position
		try {
			if (container) {
				container.scrollTop = initialScrollTop;
				safeDispatchScroll(container);
			}
		} catch {
			// ignore
		}
	}

	return accumulatedTurns;
}

/** Main extraction entry point for Google AI Studio. */
export function extractAiStudio(doc: Document): AiStudioExtractResult | null {
	if (!isAiStudio(doc)) return null;

	const title = extractAiStudioTitle(doc);
	const turns = extractAiStudioTurns(doc);
	const cleanedTurns = cleanAiStudioTurns(turns);
	const url = doc.URL || (typeof window !== 'undefined' ? window.location.href : '');

	const markdown = turnsToMarkdown(cleanedTurns);
	const html = turnsToHtml(cleanedTurns);

	const variables: Record<string, string> = {
		title,
		url,
		turnsCount: String(cleanedTurns.length),
		firstPrompt: cleanedTurns[0]?.prompt || '',
		lastResponse: cleanedTurns[cleanedTurns.length - 1]?.response || '',
	};

	return {
		title,
		url,
		turns,
		markdown,
		html,
		variables,
	};
}

/** Async extraction entry point for Google AI Studio. */
export async function extractAiStudioAsync(doc: Document): Promise<AiStudioExtractResult | null> {
	if (!isAiStudio(doc)) return null;

	const title = extractAiStudioTitle(doc);
	const url = doc.URL || (typeof window !== 'undefined' ? window.location.href : '');
	const conversationKey = getAiStudioConversationKey(url);

	let turns = extractAiStudioTurns(doc);

	// Smart check: Check if the AI Studio scrollbar indicates more turns than currently mounted in DOM
	const scrollbarButtons = doc.querySelectorAll('ms-items-scrollbar button[aria-controls]');
	const totalExpected = scrollbarButtons.length;
	const completeTurns = turns.filter(t => t.prompt.trim().length > 0 && t.response.trim().length > 0);
	const needsScan = totalExpected > completeTurns.length ||
		turns.some(t => !t.prompt.trim() || !t.response.trim());

	if (needsScan) {
		turns = await scanFullAiStudioChat(doc);
		const scannedUrl = doc.URL || (typeof window !== 'undefined' ? window.location.href : '');
		if (getAiStudioConversationKey(scannedUrl) !== conversationKey) return null;
	}
	const cleanedTurns = cleanAiStudioTurns(turns);

	const markdown = turnsToMarkdown(cleanedTurns);
	const html = turnsToHtml(cleanedTurns);

	const variables: Record<string, string> = {
		title,
		url,
		turnsCount: String(cleanedTurns.length),
		firstPrompt: cleanedTurns[0]?.prompt || '',
		lastResponse: cleanedTurns[cleanedTurns.length - 1]?.response || '',
	};

	return {
		title,
		url,
		turns,
		markdown,
		html,
		variables,
	};
}
