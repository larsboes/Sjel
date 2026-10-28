// Settings Sjel owns are read-only here. Upstream re-renders the template editor on every
// template switch, so the lock is a body class and a stylesheet rather than disabled inputs.
import { SjelManaged } from './sjel-settings';

const STYLE = `
body.sjel-managed #template-editor,
body.sjel-managed #provider-list,
body.sjel-managed #model-list { pointer-events: none; opacity: 0.6; }
body.sjel-managed #new-template-btn,
body.sjel-managed .import-template-btn,
body.sjel-managed #duplicate-template-btn,
body.sjel-managed #delete-template-btn,
body.sjel-managed #add-provider-btn,
body.sjel-managed #add-model-btn { display: none; }
.sjel-managed-note { margin: 0 0 1em; padding: 0.6em 0.8em; border-radius: 6px; background: var(--background-modifier-hover, rgba(127,127,127,0.12)); }
`;

export function lockManagedSettings(managed: SjelManaged | null, doc: Document = document): void {
	if (!managed) return;
	doc.body.classList.add('sjel-managed');
	const style = doc.createElement('style');
	style.textContent = STYLE;
	doc.head.appendChild(style);
	const model = managed.interpreter.available ? managed.interpreter.model ?? 'a local model' : 'no local model configured';
	const notes: Record<string, string> = {
		'templates-section': `Templates are managed by Sjel (revision ${managed.revision}). Edit them in the Sjel app under Settings › Clipper.`,
		'interpreter-section': `The Interpreter runs on Sjel's local model: ${model}. Sjel chooses it; no provider key is kept in this browser.`,
	};
	for (const [section, text] of Object.entries(notes)) {
		const header = doc.querySelector(`#${section} .settings-section-header`);
		if (!header) continue;
		const note = doc.createElement('p');
		note.className = 'sjel-managed-note';
		note.textContent = text;
		header.insertAdjacentElement('afterend', note);
	}
}
