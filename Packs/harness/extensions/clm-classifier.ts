import type { ClassifierContext, ClassifierModel, ClassifierOptions, ClassifierResult } from '@earendil-works/pi-ai';
import type { ExtensionAPI } from '@earendil-works/pi-coding-agent';

const BASE_URL = 'http://127.0.0.1:8700/v1/';
const MODEL_ID = 'clm-latest';

function record(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function probability(value: unknown): number {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 0 || value > 1) {
    throw new Error('CLM returned an invalid probability');
  }
  return value;
}

function parseAnswers(body: unknown, context: ClassifierContext): ClassifierResult['answers'] {
  if (!record(body) || !record(body.answers) || body.model !== MODEL_ID) {
    throw new Error('CLM returned an unexpected model or response');
  }
  const answers: ClassifierResult['answers'] = Object.create(null);
  for (const [id, question] of Object.entries(context.questions)) {
    const answer = body.answers[id];
    if (!record(answer)) throw new Error(`CLM omitted ${id}`);
    if (question.type === 'bool') {
      if (answer.type !== 'noul') throw new Error(`CLM returned the wrong answer type for ${id}`);
      answers[id] = { type: 'bool', probability: probability(answer.noul) };
    } else if (question.type === 'choice') {
      if (answer.type !== 'choice' || !record(answer.probabilities) ||
          typeof answer.choice !== 'string' || !Object.hasOwn(question.criteria, answer.choice)) {
        throw new Error(`CLM returned an invalid choice for ${id}`);
      }
      const probabilities: Record<string, number> = Object.create(null);
      for (const key of Object.keys(question.criteria)) {
        probabilities[key] = probability(answer.probabilities[key]);
      }
      if (Math.abs(Object.values(probabilities).reduce((sum, p) => sum + p, 0) - 1) > 0.01) {
        throw new Error(`CLM returned an invalid distribution for ${id}`);
      }
      answers[id] = { type: 'choice', choice: answer.choice,
        confidence: probability(answer.confidence), probabilities };
    } else {
      if (answer.type !== 'score' || typeof answer.score !== 'number' ||
          !Number.isFinite(answer.score) || answer.score < 0 || answer.score > question.criteria.length - 1) {
        throw new Error(`CLM returned an invalid score for ${id}`);
      }
      answers[id] = { type: 'score', score: answer.score, confidence: probability(answer.confidence) };
    }
  }
  return answers;
}

export async function classifyClm(
  model: ClassifierModel<'typesafe-system-one'>,
  context: ClassifierContext,
  options?: ClassifierOptions,
): Promise<ClassifierResult> {
  const result: ClassifierResult = {
    api: model.api, provider: model.provider, model: model.id,
    answers: {}, stopReason: 'stop', timestamp: Date.now(),
  };
  try {
    const questions = Object.fromEntries(Object.entries(context.questions).map(([id, question]) =>
      [id, question.type === 'bool' ? { ...question, type: 'noul' } : question]));
    let payload: unknown = { model: model.id, state: context.state, questions };
    payload = await options?.onPayload?.(payload, model) ?? payload;
    const timeout = AbortSignal.timeout(options?.timeoutMs ?? 5000);
    const signal = options?.signal ? AbortSignal.any([options.signal, timeout]) : timeout;
    // This endpoint is fixed to loopback; never route raw Sjel state to a configured remote URL.
    const response = await (options?.fetch ?? fetch)(new URL('systemone', BASE_URL), {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        ...(process.env.CLM_API_KEY ? { authorization: `Bearer ${process.env.CLM_API_KEY}` } : {}),
      },
      body: JSON.stringify(payload),
      signal,
    });
    await options?.onResponse?.({ status: response.status, headers: Object.fromEntries(response.headers) }, model);
    if (!response.ok) throw new Error(`CLM returned HTTP ${response.status}`);
    const body: unknown = await response.json();
    result.answers = parseAnswers(body, context);
    if (record(body) && record(body.usage) && typeof body.usage.input_tokens === 'number' &&
        Number.isFinite(body.usage.input_tokens) && body.usage.input_tokens >= 0) {
      const input = body.usage.input_tokens;
      result.usage = { input, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: input,
        cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } };
    }
  } catch (error) {
    result.stopReason = options?.signal?.aborted ? 'aborted' : 'error';
    result.errorMessage = error instanceof Error ? error.message : 'CLM request failed';
  }
  return result;
}

export default function (pi: ExtensionAPI): void {
  if (process.env.SJEL_CLM_ENABLE !== '1') return;
  pi.registerProvider('sjel-clm', {
    apiKey: 'local',
    models: [{ type: 'classifier', id: MODEL_ID, name: 'CLM v0.1 (local)',
      api: 'typesafe-system-one', baseUrl: BASE_URL, input: ['text'], contextWindow: 2048,
      cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 } }],
    classifiers: { 'typesafe-system-one': { classify: classifyClm } },
  });
  pi.registerCommand('clm-probe', {
    description: 'Check that the local CLM service has a real encoder and head, then classify synthetic text',
    handler: async (_args, ctx) => {
      try {
        const response = await fetch(new URL('../health', BASE_URL), { signal: AbortSignal.timeout(5000) });
        if (!response.ok) throw new Error(`CLM health returned HTTP ${response.status}`);
        const health: unknown = await response.json();
        if (!record(health) || health.ok !== true || health.embedder !== true || health.mock === true ||
            !Array.isArray(health.models) || !health.models.includes(MODEL_ID)) {
          throw new Error('CLM has no real encoder and reference head (or the server is a mock)');
        }
        const model = ctx.modelRegistry.findOfType('classifier', 'sjel-clm', MODEL_ID);
        if (!model) throw new Error('CLM classifier was not registered');
        const result = await ctx.modelRegistry.classify(model, {
          state: { prompt: 'Find a train to Berlin tomorrow' },
          questions: { domain: { type: 'choice', instructions: 'Which topic is this?',
            criteria: { travel: 'Train journey', finance: 'Bank transaction' } } },
        });
        if (result.stopReason !== 'stop') throw new Error(result.errorMessage ?? 'CLM classification failed');
        if (result.answers.domain?.type !== 'choice') throw new Error('CLM returned no domain choice');
        ctx.ui.notify(`CLM: encoder and head responding; synthetic route: ${result.answers.domain.choice}`, 'info');
      } catch (error) {
        ctx.ui.notify(`CLM probe failed: ${error instanceof Error ? error.message : 'unknown error'}`, 'error');
      }
    },
  });
}
