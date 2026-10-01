import { describe, expect, test } from 'bun:test';
import type { ClassifierContext, ClassifierModel, ClassifierOptions } from '@earendil-works/pi-ai';
import register, { checkHealth, classifyClm, endpoint } from '../Packs/harness/extensions/clm-classifier';

const model = {
  type: 'classifier', id: 'clm-latest', name: 'CLM', provider: 'sjel-clm',
  api: 'typesafe-system-one', baseUrl: 'http://127.0.0.1:8700/v1/',
  input: ['text'], contextWindow: 2048,
  cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
} as ClassifierModel<'typesafe-system-one'>;

const context: ClassifierContext = {
  state: { prompt: 'Find me a train tomorrow' },
  questions: {
    route: { type: 'choice', instructions: 'Where?', criteria: { travel: 'Trips', finance: 'Ledger' } },
    urgent: { type: 'bool', instructions: 'Is this urgent?', criteria: { true: 'Urgent', false: 'Routine' } },
    priority: { type: 'score', instructions: 'Priority?', criteria: ['Low', 'High'] },
  },
};

const valid = { model: 'clm-latest', answers: {
  route: { type: 'choice', choice: 'travel', confidence: 0.8, probabilities: { travel: 0.9, finance: 0.1 } },
  urgent: { type: 'noul', noul: 0.2 },
  priority: { type: 'score', score: 0.6, confidence: 0.7 },
}, usage: { input_tokens: 42 } };

function response(body: unknown, status = 200): ClassifierOptions {
  return { fetch: async () => new Response(JSON.stringify(body), { status }) };
}

describe('local CLM classifier (synthetic wire responses, not model validation)', () => {
  test('maps typed answers and usage without generating text', async () => {
    let sent: unknown;
    const result = await classifyClm(model, context, {
      fetch: async (url, init) => {
        expect(String(url)).toBe('http://127.0.0.1:8700/v1/systemone');
        sent = JSON.parse(String(init?.body));
        return new Response(JSON.stringify(valid));
      },
    });
    expect(result.stopReason).toBe('stop');
    expect(result.answers.route).toEqual({ type: 'choice', choice: 'travel', confidence: 0.8,
      probabilities: { travel: 0.9, finance: 0.1 } });
    expect(result.answers.urgent).toEqual({ type: 'bool', probability: 0.2 });
    expect(result.usage?.totalTokens).toBe(42);
    expect((sent as { questions: { urgent: { type: string } } }).questions.urgent.type).toBe('noul');
  });

  test('rejects wrong models, invalid distributions and missing answers', async () => {
    for (const body of [
      { ...valid, model: 'mock' },
      { ...valid, answers: { ...valid.answers, route: { ...valid.answers.route, probabilities: { travel: 0.9, finance: 0.9 } } } },
      { ...valid, answers: { route: valid.answers.route } },
    ]) {
      const result = await classifyClm(model, context, response(body));
      expect(result.stopReason).toBe('error');
      expect(result.answers).toEqual({});
    }
  });

  test('HTTP errors do not disclose server bodies', async () => {
    const result = await classifyClm(model, context, response({ detail: 'private input' }, 502));
    expect(result.stopReason).toBe('error');
    expect(result.errorMessage).toBe('CLM returned HTTP 502');
  });

  test('registers the probe always, and the classifier only when deliberately enabled', () => {
    const previous = process.env.SJEL_CLM_ENABLE;
    // Recording both halves, because the property that matters is not "the factory returns
    // early" -- it is that a session which has not opted in holds no provider that could
    // reach the service. The probe is not that: with the gate shut it says so and returns.
    const load = () => {
      const seen = { providers: [] as string[], commands: [] as string[] };
      register({
        registerProvider: (id: string) => { seen.providers.push(id); },
        registerCommand: (name: string) => { seen.commands.push(name); },
      } as any);
      return seen;
    };
    try {
      delete process.env.SJEL_CLM_ENABLE;
      expect(load()).toEqual({ providers: [], commands: ['clm-probe'] });

      process.env.SJEL_CLM_ENABLE = '1';
      expect(load()).toEqual({ providers: ['sjel-clm'], commands: ['clm-probe'] });
    } finally {
      if (previous === undefined) delete process.env.SJEL_CLM_ENABLE;
      else process.env.SJEL_CLM_ENABLE = previous;
    }
  });

  test('the endpoint defaults to CLM and refuses anything but plain HTTP on loopback', () => {
    expect(endpoint({})).toMatchObject({ backend: 'clm', baseUrl: 'http://127.0.0.1:8700/v1/', model: 'clm-latest' });
    expect(endpoint({ SJEL_SYSTEMONE_BACKEND: 'ollama' })).toMatchObject(
      { backend: 'ollama', provider: 'sjel-ollama', baseUrl: 'http://127.0.0.1:11434/v1/', model: 'nimble' });
    expect(endpoint({ SJEL_SYSTEMONE_BACKEND: 'ollama', SJEL_SYSTEMONE_MODEL: 'tev1:0.8b' }).model).toBe('tev1:0.8b');
    for (const url of ['http://192.168.1.5:11434/v1/', 'https://127.0.0.1:8700/v1/', 'http://example.com/v1/']) {
      expect(() => endpoint({ SJEL_SYSTEMONE_URL: url })).toThrow('loopback');
    }
    expect(() => endpoint({ SJEL_SYSTEMONE_BACKEND: 'jev' })).toThrow('clm or ollama');
  });

  test('an Ollama backend is called on its own port and may answer with the :latest tag', async () => {
    const previous = process.env.SJEL_SYSTEMONE_BACKEND;
    process.env.SJEL_SYSTEMONE_BACKEND = 'ollama';
    try {
      const result = await classifyClm({ ...model, id: 'nimble', provider: 'sjel-ollama' }, context, {
        fetch: async (url) => {
          expect(String(url)).toBe('http://127.0.0.1:11434/v1/systemone');
          return new Response(JSON.stringify({ ...valid, model: 'nimble:latest' }));
        },
      });
      expect(result.stopReason).toBe('stop');
      const wrong = await classifyClm(model, context, response({ ...valid, model: 'tev1:latest' }));
      expect(wrong.errorMessage).toBe('Ollama returned an unexpected model or response');
    } finally {
      if (previous === undefined) delete process.env.SJEL_SYSTEMONE_BACKEND;
      else process.env.SJEL_SYSTEMONE_BACKEND = previous;
    }
  });

  test('the Ollama health check reads /api/tags, because Ollama has no CLM /health', async () => {
    const target = endpoint({ SJEL_SYSTEMONE_BACKEND: 'ollama' });
    const tags = (names: string[]) => (async (url: string | URL | Request) => {
      expect(String(url)).toBe('http://127.0.0.1:11434/api/tags');
      return new Response(JSON.stringify({ models: names.map((name) => ({ name })) }));
    }) as typeof fetch;
    await checkHealth(target, tags(['qwen3:4b', 'nimble:latest']));
    await expect(checkHealth(target, tags(['qwen3:4b']))).rejects.toThrow('ollama pull nimble');
  });
});
