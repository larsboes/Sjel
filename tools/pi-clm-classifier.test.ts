import { describe, expect, test } from 'bun:test';
import type { ClassifierContext, ClassifierModel, ClassifierOptions } from '@earendil-works/pi-ai';
import register, { classifyClm } from '../Packs/harness/extensions/clm-classifier';

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

  test('is not registered unless deliberately enabled', () => {
    const previous = process.env.SJEL_CLM_ENABLE;
    delete process.env.SJEL_CLM_ENABLE;
    let registered = false;
    try {
      register({ registerProvider: () => { registered = true; } } as any);
      expect(registered).toBe(false);
    } finally {
      if (previous === undefined) delete process.env.SJEL_CLM_ENABLE;
      else process.env.SJEL_CLM_ENABLE = previous;
    }
  });
});
