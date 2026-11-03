import browser from './utils/browser-polyfill';
import * as highlighter from './utils/highlighter';
import { removeExistingHighlights } from './utils/highlighter-overlays';
import { loadSettings, generalSettings } from './utils/storage-utils';
import { getDomain } from './utils/string-utils';
import { extractContentBySelector as extractContentBySelectorShared } from './utils/shared';
import { createMarkdownContent } from 'defuddle/full';
import { flattenShadowDom } from './utils/flatten-shadow-dom';
import { serializeChildren } from './utils/dom-utils';
import { saveFile } from './utils/file-utils';
import { debugLog } from './utils/debug';
import { updateSidebarWidth, addResizeHandle, cleanupResizeHandlers } from './utils/iframe-resize';
import { parseForClipAsync } from './utils/clip-utils';
import { cleanAiStudioTurns, extractAiStudio, extractAiStudioAsync, extractAiStudioTurns, getAiStudioConversationKey, scanFullAiStudioChat, turnsToMarkdown, mergeTurnsForConversation, turnsToHtml } from './utils/ai-studio-extractor';

declare global {
	interface Window {
		obsidianClipperGeneration?: number;
	}
}

// IIFE to scope variables and allow safe re-execution
(function() {
	// Bump the generation counter on every injection. Older listeners close
	// over their own generation value and bail out when they see a newer one,
	// so a zombie content script (runtime invalidated after extension update)
	// will silently yield to the freshly-injected instance.
	window.obsidianClipperGeneration = (window.obsidianClipperGeneration ?? 0) + 1;
	const myGeneration = window.obsidianClipperGeneration;

	debugLog('Clipper', 'Initializing content script, generation', myGeneration);

	let liveAiStudioTurns: any[] = [];
	let liveAiStudioConversationKey = '';
	const currentAiStudioConversationKey = () => getAiStudioConversationKey(window.location.href);
	const isCurrentAiStudioConversation = (sourceUrl: string) =>
		getAiStudioConversationKey(sourceUrl) === currentAiStudioConversationKey();
	const mergeLiveAiStudioTurns = (turns: any[], sourceUrl = window.location.href) => {
		const conversationKey = getAiStudioConversationKey(sourceUrl);
		liveAiStudioTurns = mergeTurnsForConversation(
			liveAiStudioTurns,
			turns,
			liveAiStudioConversationKey,
			conversationKey,
		);
		liveAiStudioConversationKey = conversationKey;
		return liveAiStudioTurns;
	};
	if (typeof window !== 'undefined' && (window.location.href.includes('aistudio.google.com') || window.location.host.includes('aistudio.google.com'))) {
		const harvestTurns = () => {
			try {
				const visible = extractAiStudioTurns(document);
				if (visible.length > 0) {
					mergeLiveAiStudioTurns(visible);
				}
			} catch {
				// ignore
			}
		};

		let harvestTimer: any = null;
		const debouncedHarvest = () => {
			clearTimeout(harvestTimer);
			harvestTimer = setTimeout(harvestTurns, 200);
		};

		window.addEventListener('scroll', debouncedHarvest, { passive: true });
		document.addEventListener('scroll', debouncedHarvest, { passive: true, capture: true });
		try {
			const obs = new MutationObserver(debouncedHarvest);
			obs.observe(document.body, { childList: true, subtree: true });
		} catch {
			// ignore
		}
		setTimeout(harvestTurns, 500);
	}

	let isHighlighterMode = false;
	const iframeId = 'obsidian-clipper-iframe';
	const containerId = 'obsidian-clipper-container';

	function removeContainer(container: HTMLElement) {
		container.classList.add('is-closing');
		updateSidebarWidth(document, null);
		cleanupResizeHandlers(document);
		container.addEventListener('animationend', () => {
			container.remove();
			highlighter.repositionHighlights();
		}, { once: true });
	}

	async function toggleIframe() {
		const existingContainer = document.getElementById(containerId);
		if (existingContainer) {
			removeContainer(existingContainer);
			return;
		}

		await ensureHighlighterCSS();

		const container = document.createElement('div');
		container.id = containerId;
		container.classList.add('is-open');

		const { clipperIframeWidth, clipperIframeHeight } = await browser.storage.local.get(['clipperIframeWidth', 'clipperIframeHeight']);
		if (clipperIframeWidth) {
			container.style.width = `${clipperIframeWidth}px`;
		}
		if (clipperIframeHeight) {
			container.style.height = `${clipperIframeHeight}px`;
		}

		const iframe = document.createElement('iframe');
		iframe.id = iframeId;
		iframe.allow = 'clipboard-write; web-share';
		iframe.src = browser.runtime.getURL('side-panel.html?context=iframe');
		container.appendChild(iframe);

		const resizeCallbacks = {
			onResize: () => highlighter.repositionHighlights(),
			onResizeEnd: () => highlighter.repositionHighlights(),
		};
		addResizeHandle(document, container, 'w', resizeCallbacks);
		addResizeHandle(document, container, 's', resizeCallbacks);
		addResizeHandle(document, container, 'sw', resizeCallbacks);

		document.body.appendChild(container);
		updateSidebarWidth(document, container);
		container.addEventListener('animationend', () => highlighter.repositionHighlights(), { once: true });
	}

	// Firefox
	browser.runtime.sendMessage({ action: "contentScriptLoaded" });

	interface ContentResponse {
		content: string;
		selectedHtml: string;
		extractedContent: { [key: string]: string };
		schemaOrgData: any;
		fullHtml: string;
		highlights: string[];
		title: string;
		description: string;
		domain: string;
		favicon: string;
		image: string;
		parseTime: number;
		published: string;
		author: string;
		site: string;
		wordCount: number;
		language: string;
		metaTags: { name?: string | null; property?: string | null; content: string | null }[];
	}

	let initializationPromise: Promise<void> = Promise.resolve();

	browser.runtime.onMessage.addListener((request: any, sender, sendResponse) => {
		// If a newer generation of this content script has been injected,
		// yield to it rather than responding from a potentially stale context.
		if (window.obsidianClipperGeneration !== myGeneration) {
			return;
		}

		if (request.action === "ping") {
			initializationPromise.then(() => sendResponse({}));
			return true;
		}

		if (request.action === "toggle-iframe") {
			toggleIframe().then(() => {
				sendResponse({ success: true });
			});
			return true;
		}

		if (request.action === "close-iframe") {
			const existingContainer = document.getElementById(containerId);
			if (existingContainer) {
				removeContainer(existingContainer);
			}
			return;
		}

		if (request.action === "copy-text-to-clipboard") {
			const textArea = document.createElement("textarea");
			textArea.value = request.text;
			document.body.appendChild(textArea);
			textArea.select();
			try {
				document.execCommand('copy');
				sendResponse({success: true});
			} catch (err) {
				sendResponse({success: false});
			}
			document.body.removeChild(textArea);
			return true;
		}

		if (request.action === "copyMarkdownToClipboard") {
			const requestConversationKey = currentAiStudioConversationKey();
			flattenShadowDom(document).then(async () => {
				try {
					const aiStudio = await extractAiStudioAsync(document);
					if (currentAiStudioConversationKey() !== requestConversationKey) {
						sendResponse({ success: false, error: 'Conversation changed during export' });
						return;
					}
					const turns = aiStudio ? mergeLiveAiStudioTurns(aiStudio.turns, aiStudio.url) : [];
					const markdown = (aiStudio && turns.length > 0)
						? turnsToMarkdown(turns)
						: createMarkdownContent((await parseForClipAsync(document)).content, document.URL);

					// Copy to clipboard
					const textArea = document.createElement("textarea");
					textArea.value = markdown;
					document.body.appendChild(textArea);
					textArea.select();
					document.execCommand('copy');
					document.body.removeChild(textArea);

					sendResponse({ success: true });
				} catch (err) {
					console.error('Failed to copy markdown to clipboard:', err);
					sendResponse({ success: false, error: (err as Error).message });
				}
			});
			return true;
		}

		if (request.action === "saveMarkdownToFile") {
			const requestConversationKey = currentAiStudioConversationKey();
			flattenShadowDom(document).then(async () => {
				try {
					const aiStudio = await extractAiStudioAsync(document);
					if (currentAiStudioConversationKey() !== requestConversationKey) {
						sendResponse({ success: false, error: 'Conversation changed during export' });
						return;
					}
					const turns = aiStudio ? mergeLiveAiStudioTurns(aiStudio.turns, aiStudio.url) : [];
					let markdown: string;
					let title: string;
					if (aiStudio && turns.length > 0) {
						markdown = turnsToMarkdown(turns);
						title = aiStudio.title || 'Untitled';
					} else {
						const defuddled = await parseForClipAsync(document);
						markdown = createMarkdownContent(defuddled.content, document.URL);
						title = defuddled.title || document.title || 'Untitled';
					}
					const fileName = title.replace(/[/\\?%*:|"<>]/g, '-');
					await saveFile({
						content: markdown,
						fileName,
						mimeType: 'text/markdown',
					});
					sendResponse({ success: true });
				} catch (err) {
					console.error('Failed to save markdown file:', err);
					sendResponse({ success: false, error: (err as Error).message });
				}
			});
			return true;
		}

		if (request.action === "getPageContent") {
			const requestConversationKey = currentAiStudioConversationKey();
			// Flatten shadow DOM before extraction (async, needs main world)
			const flattenTimeout = new Promise<void>(resolve => setTimeout(resolve, 3000));
			Promise.race([flattenShadowDom(document), flattenTimeout]).then(async () => {
				let selectedHtml = '';
				const selection = window.getSelection();

				if (selection && selection.rangeCount > 0) {
					const range = selection.getRangeAt(0);
					const clonedSelection = range.cloneContents();
					const div = document.createElement('div');
					div.appendChild(clonedSelection);
					selectedHtml = serializeChildren(div);
				}

				const defuddled = await parseForClipAsync(document);
				if (currentAiStudioConversationKey() !== requestConversationKey) {
					sendResponse({ success: false, error: 'Conversation changed during extraction' });
					return;
				}
				const extractedContent: { [key: string]: string } = {
					...defuddled.variables,
				};

				const aiStudio = await extractAiStudioAsync(document);
				if (currentAiStudioConversationKey() !== requestConversationKey) {
					sendResponse({ success: false, error: 'Conversation changed during extraction' });
					return;
				}
				if (aiStudio) {
					aiStudio.turns = mergeLiveAiStudioTurns(aiStudio.turns, aiStudio.url);
					if (aiStudio.turns.length > 0) {
						const cleanedTurns = cleanAiStudioTurns(aiStudio.turns);
						aiStudio.markdown = turnsToMarkdown(cleanedTurns);
						aiStudio.html = turnsToHtml(cleanedTurns);
						aiStudio.variables.turnsCount = String(cleanedTurns.length);
						aiStudio.variables.firstPrompt = cleanedTurns[0]?.prompt || '';
						aiStudio.variables.lastResponse = cleanedTurns[cleanedTurns.length - 1]?.response || '';

						defuddled.title = aiStudio.title || defuddled.title;
						defuddled.content = aiStudio.html;
						Object.assign(extractedContent, aiStudio.variables);
						extractedContent.markdown = aiStudio.markdown;
						extractedContent.turns = JSON.stringify(cleanedTurns);
					}
				}

				// Create a new DOMParser
				const parser = new DOMParser();
				// Parse the document's HTML
				const doc = parser.parseFromString(document.documentElement.outerHTML, 'text/html');

				// Remove all script and style elements
				doc.querySelectorAll('script, style').forEach(el => el.remove());

				// Remove style attributes from all elements
				doc.querySelectorAll('*').forEach(el => el.removeAttribute('style'));

				// Convert all relative URLs to absolute
				doc.querySelectorAll('[src], [href]').forEach(element => {
					['src', 'href', 'srcset'].forEach(attr => {
						const value = element.getAttribute(attr);
						if (!value) return;

						if (attr === 'srcset') {
							const newSrcset = value.split(',').map(src => {
								const [url, size] = src.trim().split(' ');
								try {
									const absoluteUrl = new URL(url, document.baseURI).href;
									return `${absoluteUrl}${size ? ' ' + size : ''}`;
								} catch (e) {
									return src;
								}
							}).join(', ');
							element.setAttribute(attr, newSrcset);
						} else if (!value.startsWith('http') && !value.startsWith('data:') && !value.startsWith('#') && !value.startsWith('//')) {
							try {
								const absoluteUrl = new URL(value, document.baseURI).href;
								element.setAttribute(attr, absoluteUrl);
							} catch (e) {
								console.warn(`Failed to process ${attr} URL:`, value);
							}
						}
					});
				});

				// Get the modified HTML without scripts, styles, and style attributes
				const cleanedHtml = doc.documentElement.outerHTML;

				const response: ContentResponse = {
					author: defuddled.author,
					content: defuddled.content,
					description: defuddled.description,
					domain: getDomain(document.URL),
					extractedContent: extractedContent,
					favicon: defuddled.favicon,
					fullHtml: cleanedHtml,
					highlights: highlighter.getHighlights(),
					image: defuddled.image,
					language: defuddled.language || '',
					parseTime: defuddled.parseTime,
					published: defuddled.published,
					schemaOrgData: defuddled.schemaOrgData,
					selectedHtml: selectedHtml,
					site: defuddled.site,
					title: defuddled.title,
					wordCount: defuddled.wordCount,
					metaTags: defuddled.metaTags || []
				};
				if (defuddled.title) {
					highlighter.setPageTitle(defuddled.title);
				}
				highlighter.updatePageDomainSettings({ site: defuddled.site, favicon: defuddled.favicon });
				sendResponse(response);
			}).catch((error: unknown) => {
				console.error('[Obsidian Clipper] getPageContent error:', error);
				sendResponse({ success: false, error: error instanceof Error ? error.message : String(error) });
			});
			return true;
		} else if (request.action === "extractContent") {
			const content = extractContentBySelector(request.selector, request.attribute, request.extractHtml);
			sendResponse({ content: content });
		} else if (request.action === "scanFullAiStudioChat") {
			const scanConversationKey = currentAiStudioConversationKey();
			scanFullAiStudioChat(document).then((turns) => {
				if (currentAiStudioConversationKey() !== scanConversationKey) {
					sendResponse({ success: false, error: 'Conversation changed during scan' });
					return;
				}
				mergeLiveAiStudioTurns(turns, scanConversationKey);
				const markdown = turnsToMarkdown(liveAiStudioTurns);
				sendResponse({ success: true, turns: cleanAiStudioTurns(liveAiStudioTurns), markdown });
			}).catch((error: unknown) => {
				console.error('[Obsidian Clipper] scanFullAiStudioChat error:', error);
				sendResponse({ success: false, error: error instanceof Error ? error.message : String(error) });
			});
			return true;
		} else if (request.action === "captureVisibleTurns") {
			try {
				const turns = extractAiStudioTurns(document);
				mergeLiveAiStudioTurns(turns);
				const markdown = turnsToMarkdown(liveAiStudioTurns);
				sendResponse({ success: true, turns: cleanAiStudioTurns(liveAiStudioTurns), markdown });
			} catch (error: unknown) {
				console.error('[Obsidian Clipper] captureVisibleTurns error:', error);
				sendResponse({ success: false, error: error instanceof Error ? error.message : String(error) });
			}
			return true;
		} else if (request.action === "paintHighlights") {
			ensureHighlighterCSS().then(() => highlighter.loadHighlights()).then(() => {
				if (generalSettings.alwaysShowHighlights) {
					highlighter.applyHighlights();
				}
				sendResponse({ success: true });
			});
			return true;
		} else if (request.action === "setHighlighterMode") {
			isHighlighterMode = request.isActive;
			ensureHighlighterCSS();
			highlighter.toggleHighlighterMenu(isHighlighterMode);
			updateHasHighlights();
			sendResponse({ success: true });
			return true;
		} else if (request.action === "getHighlighterMode") {
			browser.runtime.sendMessage({ action: "getHighlighterMode" }).then(sendResponse);
			return true;
		} else if (request.action === "toggleHighlighter") {
			ensureHighlighterCSS();
			highlighter.toggleHighlighterMenu(request.isActive);
			updateHasHighlights();
			sendResponse({ success: true });
		} else if (request.action === "highlightSelection") {
			ensureHighlighterCSS();
			highlighter.toggleHighlighterMenu(request.isActive);
			const selection = window.getSelection();
			if (selection && !selection.isCollapsed) {
				highlighter.handleTextSelection(selection);
			}
			updateHasHighlights();
			sendResponse({ success: true });
		} else if (request.action === "highlightElement") {
			ensureHighlighterCSS();
			highlighter.toggleHighlighterMenu(request.isActive);
			if (request.targetElementInfo) {
				const { mediaType, srcUrl, pageUrl } = request.targetElementInfo;
				
				let elementToHighlight: Element | null = null;

				// Function to compare URLs, handling both absolute and relative paths
				const urlMatches = (elementSrc: string, targetSrc: string) => {
					const elementUrl = new URL(elementSrc, pageUrl);
					const targetUrl = new URL(targetSrc, pageUrl);
					return elementUrl.href === targetUrl.href;
				};

				// Try to find the element using the src attribute
				elementToHighlight = document.querySelector(`${mediaType}[src="${srcUrl}"]`);

				// If not found, try with relative URL
				if (!elementToHighlight) {
					const relativeSrc = new URL(srcUrl).pathname;
					elementToHighlight = document.querySelector(`${mediaType}[src="${relativeSrc}"]`);
				}

				// If still not found, iterate through all elements of the media type
				if (!elementToHighlight) {
					const elements = Array.from(document.getElementsByTagName(mediaType));
					for (const el of elements) {
						if (el instanceof HTMLImageElement || el instanceof HTMLVideoElement || el instanceof HTMLAudioElement) {
							if (urlMatches(el.src, srcUrl)) {
								elementToHighlight = el;
								break;
							}
						}
					}
				}

				if (elementToHighlight) {
					highlighter.highlightElement(elementToHighlight);
				} else {
					console.warn('Could not find element to highlight. Info:', request.targetElementInfo);
				}
			}
			updateHasHighlights();
			sendResponse({ success: true });
		} else if (request.action === "clearHighlights") {
			highlighter.clearHighlights();
			updateHasHighlights();
			sendResponse({ success: true });
		} else if (request.action === "getHighlighterState") {
			browser.runtime.sendMessage({ action: "getHighlighterMode" })
				.then(response => {
					sendResponse(response);
				})
				.catch(error => {
					console.error("Error getting highlighter mode:", error);
					sendResponse({ isActive: false });
				});
			return true;
		} else if (request.action === "getReaderModeState") {
			sendResponse({ isActive: document.documentElement.classList.contains('obsidian-reader-active') });
			return true;
		}
		return true;
	});

	function extractContentBySelector(selector: string, attribute?: string, extractHtml: boolean = false): string | string[] {
		return extractContentBySelectorShared(document, selector, attribute, extractHtml);
	}

	function updateHasHighlights() {
		const hasHighlights = highlighter.getHighlights().length > 0;
		browser.runtime.sendMessage({ action: "updateHasHighlights", hasHighlights });
	}

	let highlighterCSSPromise: Promise<void> | null = null;
	function ensureHighlighterCSS(): Promise<void> {
		if (!highlighterCSSPromise) {
			highlighterCSSPromise = new Promise<void>((resolve) => {
				const link = document.createElement('link');
				link.rel = 'stylesheet';
				link.href = browser.runtime.getURL('highlighter.css');
				link.onload = () => resolve();
				link.onerror = () => resolve();
				(document.head || document.documentElement).appendChild(link);
			});
		}
		return highlighterCSSPromise;
	}

	async function initializeHighlighter() {
		await loadSettings();

		if (generalSettings.alwaysShowHighlights) {
			const result = await browser.storage.local.get('highlights');
			const allHighlights = (result.highlights || {}) as Record<string, unknown>;
			if (allHighlights[window.location.href]) {
				await ensureHighlighterCSS();
			}
		}

		await highlighter.loadHighlights();
		highlighter.setPageTitle(document.title);
		updateHasHighlights();
	}

	// A ping should only report ready after settings and saved highlights have
	// finished loading. This keeps the first lazy-loaded action from racing
	// content-script initialization.
	initializationPromise = initializeHighlighter().catch((error) => {
		console.error('[Obsidian Clipper] Failed to initialize highlighter:', error);
	});

	// Expose highlighter API on window so reader-script.js (a separate
	// webpack bundle injected when reader mode activates) can delegate
	// all state operations to this single module instance. Without this,
	// both bundles own a copy of highlighter.ts with independent mutable
	// state — the bridge ensures one source of truth per tab.
	window.__obsidianHighlighter = {
		toggleHighlighterMenu: highlighter.toggleHighlighterMenu,
		handleTextSelection: highlighter.handleTextSelection,
		highlightElement: highlighter.highlightElement,
		applyHighlights: highlighter.applyHighlights,
		loadHighlights: highlighter.loadHighlights,
		invalidateHighlightCache: highlighter.invalidateHighlightCache,
		repositionHighlights: highlighter.repositionHighlights,
		getHighlights: highlighter.getHighlights,
		setPageUrl: highlighter.setPageUrl,
		setPageTitle: highlighter.setPageTitle,
		updatePageDomainSettings: highlighter.updatePageDomainSettings,
		clearHighlights: highlighter.clearHighlights,
		saveHighlights: highlighter.saveHighlights,
		updateHighlighterMenu: highlighter.updateHighlighterMenu,
		removeExistingHighlights,
		ensureHighlighterCSS: () => { ensureHighlighterCSS(); },
	} satisfies highlighter.HighlighterAPI;

	// Call updateHasHighlights when the page loads
	window.addEventListener('load', updateHasHighlights);

	// Deactivate highlighter mode on unload
	function handlePageUnload() {
		if (isHighlighterMode) {
			highlighter.toggleHighlighterMenu(false);
			browser.runtime.sendMessage({ action: "highlighterModeChanged", isActive: false });
			browser.storage.local.set({ isHighlighterMode: false });
		}
	}

	window.addEventListener('beforeunload', handlePageUnload);

	// Listen for custom events from the reader script
	document.addEventListener('obsidian-reader-init', async () => {
		// Find the highlighter button
		const button = document.querySelector('[data-action="toggle-highlighter"]');
		if (button) {
			// Handle highlighter button clicks
			button.addEventListener('click', async (e) => {
				try {
					// First try to get the tab ID from the background script
					const response = await browser.runtime.sendMessage({ action: "ensureContentScriptLoaded" });
					
					let tabId: number | undefined;
					if (response && typeof response === 'object') {
						tabId = (response as { tabId: number }).tabId;
					}

					// If we didn't get a tab ID, try to get it from the background script
					if (!tabId) {
						try {
							const response = await browser.runtime.sendMessage({ action: "getActiveTab" }) as { tabId?: number; error?: string };
							if (response && !response.error && response.tabId) {
								tabId = response.tabId;
							}
						} catch (error) {
							console.error('[Content] Failed to get tab ID from background script:', error);
						}
					}

					if (tabId) {
						await browser.runtime.sendMessage({ action: "toggleHighlighterMode", tabId });
					} else {
						console.error('[Content]','Could not determine tab ID');
					}
				} catch (error) {
					console.error('[Content]','Error in toggle flow:', error);
				}
			});
		}
	});

})();
