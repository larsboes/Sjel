// Runs in the MAIN world to extract shadow DOM content.
// Content scripts cannot read shadow root content due to Chrome's isolation.
// This script stamps shadow innerHTML into data attributes readable from any world,
// recursively walking child shadow roots to support nested Web Components (e.g. AI Studio).
(function() {
	function walk(root) {
		try {
			const elements = root.querySelectorAll('*');
			for (let i = 0; i < elements.length; i++) {
				const el = elements[i];
				if (el.shadowRoot) {
					try {
						walk(el.shadowRoot);
						if (el.shadowRoot.innerHTML) {
							el.setAttribute('data-defuddle-shadow', el.shadowRoot.innerHTML);
						}
					} catch (e) {
						// Ignore closed or protected shadow roots
					}
				}
			}
		} catch (e) {
			// Ignore access errors on special nodes
		}
	}
	walk(document);
})();
