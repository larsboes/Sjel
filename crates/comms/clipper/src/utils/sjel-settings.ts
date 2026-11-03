// Settings Sjel owns: the templates and the Interpreter model (capabilities/comms
// store/clipper_settings.rs). The extension reads them over native messaging and locks
// its own editors for them. Until Sjel holds templates, the first sync seeds Sjel with
// the ones this browser has.
import { compressToUTF16, decompressFromUTF16 } from 'lz-string';
import browser from './browser-polyfill';
import { ModelConfig, Provider, Settings, Template } from '../types/types';
import { SJEL_HOST } from './sjel-destination';

export const SJEL_PROVIDER_ID = 'sjel';
const MANAGED_KEY = 'sjelManaged';
const CHUNK_SIZE = 8000;

export interface SjelManaged {
	revision: number;
	interpreter: { available: boolean; model?: string; backend?: string };
	syncedAt: string;
}

type Send = (message: unknown) => Promise<any>;
const nativeSend: Send = (message) => browser.runtime.sendNativeMessage(SJEL_HOST, message);

export async function sjelManaged(): Promise<SjelManaged | null> {
	const stored = await browser.storage.local.get(MANAGED_KEY);
	return (stored[MANAGED_KEY] as SjelManaged | undefined) ?? null;
}

async function readTemplates(): Promise<Template[]> {
	const { template_list } = await browser.storage.sync.get('template_list');
	const ids = ((template_list as string[] | undefined) ?? []).filter(Boolean);
	const templates: Template[] = [];
	for (const id of ids) {
		const chunks = (await browser.storage.sync.get(`template_${id}`))[`template_${id}`] as string[] | undefined;
		if (!chunks) continue;
		try {
			templates.push(JSON.parse(decompressFromUTF16(chunks.join(''))));
		} catch {
			// upstream's loader skips an unreadable template the same way
		}
	}
	return templates;
}

// Upstream's storage format (managers/template-manager.ts): lz-string chunks per template.
async function writeTemplates(templates: Template[]): Promise<void> {
	const { template_list } = await browser.storage.sync.get('template_list');
	const next = templates.map((t) => t.id);
	const stale = ((template_list as string[] | undefined) ?? []).filter((id) => id && !next.includes(id));
	const items: Record<string, unknown> = { template_list: next };
	for (const template of templates) {
		const compressed = compressToUTF16(JSON.stringify(template));
		const chunks: string[] = [];
		for (let i = 0; i < compressed.length; i += CHUNK_SIZE) chunks.push(compressed.slice(i, i + CHUNK_SIZE));
		items[`template_${template.id}`] = chunks;
	}
	await browser.storage.sync.set(items);
	if (stale.length) await browser.storage.sync.remove(stale.map((id) => `template_${id}`));
}

/**
 * Pull Sjel's settings and apply them. Returns what is now managed, or null when Sjel did
 * not answer, in which case the browser keeps what it last applied.
 */
export async function syncSjelSettings(send: Send = nativeSend): Promise<SjelManaged | null> {
	let answer: any;
	try {
		answer = await send({ type: 'settings_get' });
	} catch {
		return null;
	}
	if (!answer?.ok) return null;
	let revision: number = answer.revision;
	if (revision === 0) {
		const local = await readTemplates();
		if (local.length === 0) return null;
		const seeded = await send({ type: 'settings_seed', templates: local }).catch(() => null);
		if (!seeded?.ok) return null;
		revision = seeded.revision;
	} else {
		const previous = await sjelManaged();
		if (previous?.revision !== revision && Array.isArray(answer.templates)) {
			await writeTemplates(answer.templates);
		}
	}
	const managed: SjelManaged = {
		revision,
		interpreter: answer.interpreter ?? { available: false },
		syncedAt: new Date().toISOString(),
	};
	await browser.storage.local.set({ [MANAGED_KEY]: managed });
	return managed;
}

export function sjelInterpreterModel(managed: SjelManaged): { provider: Provider; model: ModelConfig } {
	const name = managed.interpreter.model ?? 'local model';
	return {
		provider: { id: SJEL_PROVIDER_ID, name: 'Sjel', baseUrl: '', apiKey: '', apiKeyRequired: false },
		model: { id: SJEL_PROVIDER_ID, providerId: SJEL_PROVIDER_ID, providerModelId: name, name: `Sjel · ${name}`, enabled: true },
	};
}

/** Replace the browser's providers and models with Sjel's once Sjel manages the Interpreter. */
export function applySjelInterpreter(settings: Settings, managed: SjelManaged | null): Settings {
	if (!managed) return settings;
	const { provider, model } = sjelInterpreterModel(managed);
	return { ...settings, providers: [provider], models: [model], interpreterModel: model.id };
}

/** One Interpreter call answered by Sjel's local model; the text upstream would parse. */
export async function interpretWithSjel(prompt: string, send: Send = nativeSend): Promise<string> {
	const answer = await send({ type: 'interpret', prompt });
	if (!answer?.ok) {
		const reason = answer?.detail ? `${answer.error}: ${answer.detail}` : answer?.error ?? 'unavailable';
		throw new Error(`Sjel could not interpret this page (${reason}).`);
	}
	return answer.content as string;
}
