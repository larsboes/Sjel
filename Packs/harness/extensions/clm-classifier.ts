import type { ClassifierContext, ClassifierModel, ClassifierOptions, ClassifierResult } from '@earendil-works/pi-ai';
import type { ExtensionAPI } from '@earendil-works/pi-coding-agent';

// One adapter, two local servers with the same wire shape: CLM's reference server
// (https://github.com/Contrastive-LM/CLM#api-reference) and Ollama 0.35+
// (https://docs.ollama.com/api/systemone). Both take POST /v1/systemone with `state` and typed
// `questions` and answer `choice` / `noul` / `score`. They differ in address, model name and
// health check, and nothing else this adapter reads.
//
//   SJEL_SYSTEMONE_BACKEND = "clm" (default) | "ollama"
//   SJEL_SYSTEMONE_URL     = base URL ending in /v1/; defaults per backend; loopback only
//   SJEL_SYSTEMONE_MODEL   = model id; defaults to clm-latest / nimble
type Backend = 'clm' | 'ollama';

interface Endpoint {
  backend: Backend;
  label: string;
  provider: string;
  baseUrl: string;
  model: string;
}

const DEFAULTS: Record<Backend, Omit<Endpoint, 'backend'>> = {
  clm: { label: 'CLM', provider: 'sjel-clm', baseUrl: 'http://127.0.0.1:8700/v1/', model: 'clm-latest' },
  ollama: { label: 'Ollama', provider: 'sjel-ollama', baseUrl: 'http://127.0.0.1:11434/v1/', model: 'nimble' },
};

const LOOPBACK = new Set(['127.0.0.1', 'localhost', '[::1]']);

/** Reads the environment on every call, so a test or a restarted session sees the current value. */
export function endpoint(env: Record<string, string | undefined> = process.env): Endpoint {
  const backend = (env.SJEL_SYSTEMONE_BACKEND ?? 'clm') as Backend;
  if (!Object.hasOwn(DEFAULTS, backend)) {
    throw new Error(`SJEL_SYSTEMONE_BACKEND must be clm or ollama, not ${backend}`);
  }
  const base = { backend, ...DEFAULTS[backend] };
  const baseUrl = env.SJEL_SYSTEMONE_URL ?? base.baseUrl;
  const url = new URL(baseUrl);
  // Raw Sjel state goes in the request body. A configurable URL must not become a way to
  // send it off this machine, so anything but plain HTTP to loopback is refused.
  if (url.protocol !== 'http:' || !LOOPBACK.has(url.hostname)) {
    throw new Error(`SJEL_SYSTEMONE_URL must be http:// on loopback, not ${url.origin}`);
  }
  return { ...base, baseUrl: baseUrl.endsWith('/') ? baseUrl : `${baseUrl}/`, model: env.SJEL_SYSTEMONE_MODEL ?? base.model };
}

/** Ollama answers with the tag it resolved, so `nimble` comes back as `nimble:latest`. */
function sameModel(returned: unknown, requested: string): boolean {
  return returned === requested || (!requested.includes(':') && returned === `${requested}:latest`);
}

function record(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function probability(value: unknown, label: string): number {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 0 || value > 1) {
    throw new Error(`${label} returned an invalid probability`);
  }
  return value;
}

function parseAnswers(body: unknown, context: ClassifierContext, target: Endpoint): ClassifierResult['answers'] {
  const label = target.label;
  const p = (value: unknown) => probability(value, label);
  if (!record(body) || !record(body.answers) || !sameModel(body.model, target.model)) {
    throw new Error(`${label} returned an unexpected model or response`);
  }
  const answers: ClassifierResult['answers'] = Object.create(null);
  for (const [id, question] of Object.entries(context.questions)) {
    const answer = body.answers[id];
    if (!record(answer)) throw new Error(`${label} omitted ${id}`);
    if (question.type === 'bool') {
      if (answer.type !== 'noul') throw new Error(`${label} returned the wrong answer type for ${id}`);
      answers[id] = { type: 'bool', probability: p(answer.noul) };
    } else if (question.type === 'choice') {
      if (answer.type !== 'choice' || !record(answer.probabilities) ||
          typeof answer.choice !== 'string' || !Object.hasOwn(question.criteria, answer.choice)) {
        throw new Error(`${label} returned an invalid choice for ${id}`);
      }
      const probabilities: Record<string, number> = Object.create(null);
      for (const key of Object.keys(question.criteria)) {
        probabilities[key] = p(answer.probabilities[key]);
      }
      if (Math.abs(Object.values(probabilities).reduce((sum, p) => sum + p, 0) - 1) > 0.01) {
        throw new Error(`${label} returned an invalid distribution for ${id}`);
      }
      answers[id] = { type: 'choice', choice: answer.choice,
        confidence: p(answer.confidence), probabilities };
    } else {
      if (answer.type !== 'score' || typeof answer.score !== 'number' ||
          !Number.isFinite(answer.score) || answer.score < 0 || answer.score > question.criteria.length - 1) {
        throw new Error(`${label} returned an invalid score for ${id}`);
      }
      answers[id] = { type: 'score', score: answer.score, confidence: p(answer.confidence) };
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
  let target: Endpoint | undefined;
  try {
    target = endpoint();
    const questions = Object.fromEntries(Object.entries(context.questions).map(([id, question]) =>
      [id, question.type === 'bool' ? { ...question, type: 'noul' } : question]));
    let payload: unknown = { model: model.id, state: context.state, questions };
    payload = await options?.onPayload?.(payload, model) ?? payload;
    const timeout = AbortSignal.timeout(options?.timeoutMs ?? 5000);
    const signal = options?.signal ? AbortSignal.any([options.signal, timeout]) : timeout;
    // endpoint() has already refused anything but loopback.
    const response = await (options?.fetch ?? fetch)(new URL('systemone', target.baseUrl), {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        ...(process.env.CLM_API_KEY ? { authorization: `Bearer ${process.env.CLM_API_KEY}` } : {}),
      },
      body: JSON.stringify(payload),
      signal,
    });
    await options?.onResponse?.({ status: response.status, headers: Object.fromEntries(response.headers) }, model);
    if (!response.ok) throw new Error(`${target.label} returned HTTP ${response.status}`);
    const body: unknown = await response.json();
    result.answers = parseAnswers(body, context, target);
    if (record(body) && record(body.usage) && typeof body.usage.input_tokens === 'number' &&
        Number.isFinite(body.usage.input_tokens) && body.usage.input_tokens >= 0) {
      const input = body.usage.input_tokens;
      result.usage = { input, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: input,
        cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } };
    }
  } catch (error) {
    result.stopReason = options?.signal?.aborted ? 'aborted' : 'error';
    result.errorMessage = error instanceof Error ? error.message : `${target?.label ?? 'System One'} request failed`;
  }
  return result;
}

/**
 * CLM's /health reports whether a real encoder and head are loaded; Ollama has no such route,
 * and /api/tags is how it says which models are pulled. A mock CLM server fails the first.
 */
export async function checkHealth(target: Endpoint, fetcher: typeof fetch = fetch): Promise<void> {
  const signal = AbortSignal.timeout(5000);
  if (target.backend === 'ollama') {
    const response = await fetcher(new URL('/api/tags', target.baseUrl), { signal });
    if (!response.ok) throw new Error(`Ollama /api/tags returned HTTP ${response.status}`);
    const tags: unknown = await response.json();
    const names = record(tags) && Array.isArray(tags.models)
      ? tags.models.map((m) => (record(m) ? m.name : undefined)) : [];
    if (!names.some((name) => sameModel(name, target.model))) {
      throw new Error(`Ollama has no ${target.model}; run: ollama pull ${target.model}`);
    }
    return;
  }
  const response = await fetcher(new URL('../health', target.baseUrl), { signal });
  if (!response.ok) throw new Error(`CLM health returned HTTP ${response.status}`);
  const health: unknown = await response.json();
  if (!record(health) || health.ok !== true || health.embedder !== true || health.mock === true ||
      !Array.isArray(health.models) || !health.models.includes(target.model)) {
    throw new Error('CLM has no real encoder and reference head (or the server is a mock)');
  }
}

export default function (pi: ExtensionAPI): void {
  // The probe is registered whether or not the classifier is, and this is the point rather
  // than tidiness. With the gate closed this extension used to register NOTHING, which made
  // it indistinguishable from a broken one -- tools/pack-extensions.test.ts says exactly
  // that ("an extension that loads but registers nothing is dead code, or a registration
  // guarded by something that is not there") and it was right: the only way to discover the
  // gate was to read this file. The classifier stays opt-in; the way to learn that is now a
  // command that tells you, not an absence you have to notice.
  pi.registerCommand('clm-probe', {
    description: 'Check that the local System One server (CLM or Ollama) has the model, then classify synthetic text',
    handler: async (_args, ctx) => {
      if (process.env.SJEL_CLM_ENABLE !== '1') {
        ctx.ui.notify('CLM is not enabled; set SJEL_CLM_ENABLE=1 and restart to register the classifier', 'info');
        return;
      }
      try {
        const target = endpoint();
        await checkHealth(target);
        const model = ctx.modelRegistry.findOfType('classifier', target.provider, target.model);
        if (!model) throw new Error(`${target.label} classifier was not registered`);
        const result = await ctx.modelRegistry.classify(model, {
          state: { prompt: 'Find a train to Berlin tomorrow' },
          questions: { domain: { type: 'choice', instructions: 'Which topic is this?',
            criteria: { travel: 'Train journey', finance: 'Bank transaction' } } },
        });
        if (result.stopReason !== 'stop') throw new Error(result.errorMessage ?? `${target.label} classification failed`);
        if (result.answers.domain?.type !== 'choice') throw new Error(`${target.label} returned no domain choice`);
        ctx.ui.notify(`${target.label} ${target.model}: responding; synthetic route: ${result.answers.domain.choice}`, 'info');
      } catch (error) {
        ctx.ui.notify(`System One probe failed: ${error instanceof Error ? error.message : 'unknown error'}`, 'error');
      }
    },
  });

  // Opt-in, and the only thing the gate withholds: a provider whose endpoint may not be
  // running. Registering it unconditionally would put a classifier whose origin is absent in
  // front of every pi session that happens to load this Pack.
  if (process.env.SJEL_CLM_ENABLE !== '1') return;
  const target = endpoint();
  pi.registerProvider(target.provider, {
    apiKey: 'local',
    models: [{ type: 'classifier', id: target.model, name: `${target.label} ${target.model} (local)`,
      api: 'typesafe-system-one', baseUrl: target.baseUrl, input: ['text'], contextWindow: 2048,
      cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 } }],
    classifiers: { 'typesafe-system-one': { classify: classifyClm } },
  });
}
