import { beforeEach, describe, expect, test, vi } from 'vitest';
import { compressToUTF16 } from 'lz-string';

const { sync, local, area } = vi.hoisted(() => {
	const area = (store: Record<string, unknown>) => ({
		get: async (keys: string | string[]) =>
			Object.fromEntries([keys].flat().filter((k) => k in store).map((k) => [k, store[k]])),
		set: async (items: Record<string, unknown>) => Object.assign(store, items),
		remove: async (keys: string[]) => keys.forEach((k) => delete store[k]),
	});
	return { sync: {} as Record<string, unknown>, local: {} as Record<string, unknown>, area };
});
vi.mock('./browser-polyfill', () => ({
	default: { runtime: { sendNativeMessage: async () => ({}) }, storage: { sync: area(sync), local: area(local) } },
}));

import { applySjelInterpreter, interpretWithSjel, sjelManaged, syncSjelSettings } from './sjel-settings';

const template = (id: string, name: string) => ({ id, name, behavior: 'create', properties: [] });
function storeTemplates(templates: any[]) {
	sync.template_list = templates.map((t) => t.id);
	for (const t of templates) sync[`template_${t.id}`] = [compressToUTF16(JSON.stringify(t))];
}

beforeEach(() => {
	for (const s of [sync, local]) for (const k of Object.keys(s)) delete s[k];
});

describe('Sjel-managed settings', () => {
	test('the first contact seeds Sjel with this browser’s templates', async () => {
		storeTemplates([template('a', 'Default')]);
		const sent: any[] = [];
		const managed = await syncSjelSettings(async (m: any) => {
			sent.push(m);
			return m.type === 'settings_get'
				? { ok: true, revision: 0, templates: null, interpreter: { available: true, model: 'qwen' } }
				: { ok: true, revision: 1 };
		});
		expect(sent[1]).toEqual({ type: 'settings_seed', templates: [template('a', 'Default')] });
		expect(managed?.revision).toBe(1);
		expect((await sjelManaged())?.interpreter.model).toBe('qwen');
	});

	test('a new revision replaces the templates and drops ones Sjel removed', async () => {
		storeTemplates([template('a', 'Old'), template('b', 'Gone')]);
		await syncSjelSettings(async () => ({
			ok: true, revision: 4, templates: [template('a', 'New'), template('c', 'Added')], interpreter: { available: false },
		}));
		expect(sync.template_list).toEqual(['a', 'c']);
		expect(sync.template_b).toBeUndefined();
		expect(sync.template_c).toBeDefined();
	});

	test('an unreachable Sjel keeps the last applied state', async () => {
		storeTemplates([template('a', 'Kept')]);
		expect(await syncSjelSettings(async () => { throw new Error('no host'); })).toBeNull();
		expect(await syncSjelSettings(async () => ({ ok: false, error: 'unavailable' }))).toBeNull();
		expect(sync.template_list).toEqual(['a']);
		expect(await sjelManaged()).toBeNull();
	});

	test('a managed Interpreter replaces the browser’s providers and models', () => {
		const base: any = { providers: [{ id: 'openai', apiKey: 'k' }], models: [{ id: 'gpt' }], interpreterModel: 'gpt' };
		expect(applySjelInterpreter(base, null)).toBe(base);
		const managed = applySjelInterpreter(base, { revision: 1, interpreter: { available: true, model: 'qwen' }, syncedAt: '' });
		expect(managed.providers).toEqual([{ id: 'sjel', name: 'Sjel', baseUrl: '', apiKey: '', apiKeyRequired: false }]);
		expect(managed.models[0]).toMatchObject({ id: 'sjel', providerModelId: 'qwen' });
		expect(managed.interpreterModel).toBe('sjel');
	});

	test('an Interpreter answer comes back as text, a refusal as an error', async () => {
		expect(await interpretWithSjel('p', async () => ({ ok: true, content: '{"prompts_responses":{}}' }))).toContain('prompts_responses');
		await expect(interpretWithSjel('p', async () => ({ ok: false, error: 'deferred', detail: 'On the go' }))).rejects.toThrow('deferred: On the go');
	});
});
