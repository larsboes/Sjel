import browser from './browser-polyfill';

export async function flattenShadowDom(doc: Document): Promise<void> {
	// 1. Request background script to run walk in MAIN world via chrome.scripting (bypasses CSP)
	try {
		const res = (await browser.runtime.sendMessage({ action: 'flattenShadowDom' })) as any;
		if (res && res.success) {
			return;
		}
	} catch {
		// Continue to fallback
	}

	// 2. Fallback: Check if shadow roots or custom elements exist
	let found = false;
	const all = doc.querySelectorAll('*');
	for (let i = 0; i < all.length; i++) {
		if (all[i].shadowRoot || all[i].tagName.includes('-')) {
			found = true;
			break;
		}
	}
	if (!found) return;

	return new Promise((resolve) => {
		const script = doc.createElement('script');
		script.src = browser.runtime.getURL('flatten-shadow-dom.js');
		script.onload = () => {
			script.remove();
			resolve();
		};
		script.onerror = () => {
			script.remove();
			resolve(); // Continue even if injection fails
		};
		(doc.head || doc.documentElement).appendChild(script);
	});
}
