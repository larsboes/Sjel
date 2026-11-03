import { homedir } from "node:os";
import { resolve } from "node:path";

type Candidate = {
  id: string;
  language: "de" | "en";
  text: string;
  relevance: number;
  rationale: string;
};

type Query = {
  id: string;
  language: "de" | "en";
  text: string;
  text_variants?: Record<string, string>;
  candidates: Candidate[];
};

type Corpus = {
  schema_version: number;
  acceptance: {
    min_top1_relevant: number;
    min_pairwise_accuracy: number;
    min_mean_ndcg: number;
  };
  queries: Query[];
};

type RerankResult = { index: number; relevance_score: number };

const corpusPath = resolve(
  process.argv[2] ?? new URL("./relevance-corpus.json", import.meta.url).pathname,
);
const settingsPath =
  process.env.OMLX_SETTINGS_PATH ?? `${homedir()}/.omlx/settings.json`;
const baseUrl = process.env.OMLX_BASE_URL ?? "http://127.0.0.1:8000/v1";
const model =
  process.env.OMLX_RERANKING_MODEL ?? "bge-reranker-v2-m3-mlx";
const queryVariant = process.env.RELEVANCE_EVAL_QUERY_VARIANT?.trim() || null;
const noAuth = process.env.OMLX_NO_AUTH === "1";

function fail(message: string): never {
  throw new Error(`reranking eval: ${message}`);
}

function assertLocalEndpoint(value: string): URL {
  const url = new URL(value);
  const loopback = ["127.0.0.1", "localhost", "::1"].includes(url.hostname);
  if (url.protocol !== "http:" || !loopback) {
    fail(`refusing non-loopback oMLX endpoint ${value}`);
  }
  return url;
}

function validateCorpus(value: Corpus): void {
  if (value.schema_version !== 1 || value.queries.length === 0) {
    fail("expected schema_version 1 with at least one query");
  }
  const languages = new Set<string>();
  const queryIds = new Set<string>();
  for (const query of value.queries) {
    languages.add(query.language);
    if (!query.id || !query.text || query.candidates.length < 2) {
      fail(`query ${query.id || "<missing>"} is incomplete`);
    }
    if (
      queryVariant &&
      (!query.text_variants?.[queryVariant] ||
        !query.text_variants[queryVariant].trim())
    ) {
      fail(`query ${query.id} has no text variant ${queryVariant}`);
    }
    if (queryIds.has(query.id)) fail(`duplicate query id ${query.id}`);
    queryIds.add(query.id);
    const candidateIds = new Set<string>();
    for (const candidate of query.candidates) {
      languages.add(candidate.language);
      if (
        !candidate.id ||
        !candidate.text ||
        !candidate.rationale ||
        !Number.isInteger(candidate.relevance) ||
        candidate.relevance < 0 ||
        candidate.relevance > 3
      ) {
        fail(`candidate ${candidate.id || "<missing>"} is incomplete`);
      }
      if (candidateIds.has(candidate.id)) {
        fail(`duplicate candidate id ${candidate.id} in query ${query.id}`);
      }
      candidateIds.add(candidate.id);
    }
  }
  if (!languages.has("de") || !languages.has("en")) {
    fail("corpus must contain both German and English");
  }
}

function dcg(judgements: number[]): number {
  return judgements.reduce(
    (sum, judgement, index) =>
      sum + (2 ** judgement - 1) / Math.log2(index + 2),
    0,
  );
}

const corpus = (await Bun.file(corpusPath).json()) as Corpus;
validateCorpus(corpus);
const endpoint = assertLocalEndpoint(baseUrl);
const apiKey = noAuth
  ? null
  : ((await Bun.file(settingsPath).json()) as { auth?: { api_key?: string } })
      .auth?.api_key?.trim();
if (!noAuth && !apiKey) fail(`no .auth.api_key in ${settingsPath}`);

const queryText = (query: Query): string =>
  queryVariant ? query.text_variants![queryVariant] : query.text;
let top1Relevant = 0;
let pairwiseCorrect = 0;
let pairwiseTotal = 0;
let ndcgTotal = 0;

console.log(
  `Comms DE/EN reranking eval · ${model} · queries=${queryVariant ?? "default"}`,
);
for (const query of corpus.queries) {
  const response = await fetch(
    new URL(`${endpoint.pathname.replace(/\/$/, "")}/rerank`, endpoint),
    {
      method: "POST",
      headers: {
        ...(apiKey ? { Authorization: `Bearer ${apiKey}` } : {}),
        "Content-Type": "application/json",
      },
      body: JSON.stringify({
        model,
        query: queryText(query),
        documents: query.candidates.map((candidate) => candidate.text),
        top_n: query.candidates.length,
        return_documents: false,
      }),
    },
  );
  if (!response.ok) {
    fail(`oMLX returned HTTP ${response.status} for ${query.id}`);
  }
  const body = (await response.json()) as { results?: RerankResult[] };
  const results = body.results ?? [];
  if (results.length !== query.candidates.length) {
    fail(`expected ${query.candidates.length} results for ${query.id}, got ${results.length}`);
  }
  const seen = new Set<number>();
  const ranked = results.map((result) => {
    if (
      !Number.isInteger(result.index) ||
      result.index < 0 ||
      result.index >= query.candidates.length ||
      seen.has(result.index) ||
      !Number.isFinite(result.relevance_score) ||
      result.relevance_score < 0 ||
      result.relevance_score > 1
    ) {
      fail(`invalid rerank result for ${query.id}`);
    }
    seen.add(result.index);
    return { ...query.candidates[result.index], score: result.relevance_score };
  });
  if (ranked[0].relevance >= 2) top1Relevant += 1;

  for (let left = 0; left < ranked.length; left += 1) {
    for (let right = left + 1; right < ranked.length; right += 1) {
      const first = ranked[left];
      const second = ranked[right];
      if (first.relevance === second.relevance) continue;
      pairwiseTotal += 1;
      if (first.relevance > second.relevance) pairwiseCorrect += 1;
    }
  }

  const actualDcg = dcg(ranked.map((candidate) => candidate.relevance));
  const idealDcg = dcg(
    query.candidates
      .map((candidate) => candidate.relevance)
      .sort((left, right) => right - left),
  );
  const ndcg = idealDcg === 0 ? 1 : actualDcg / idealDcg;
  ndcgTotal += ndcg;
  console.log(
    `${query.id}: top=${ranked[0].id} score=${ranked[0].score.toFixed(3)} judgement=${ranked[0].relevance} nDCG=${ndcg.toFixed(3)}`,
  );
  for (const candidate of ranked) {
    console.log(
      `  ${candidate.id.padEnd(28)} score=${candidate.score.toFixed(3)} judgement=${candidate.relevance} lang=${candidate.language}`,
    );
  }
}

const top1Rate = top1Relevant / corpus.queries.length;
const pairwiseAccuracy =
  pairwiseTotal === 0 ? 1 : pairwiseCorrect / pairwiseTotal;
const meanNdcg = ndcgTotal / corpus.queries.length;
const accepted =
  top1Rate >= corpus.acceptance.min_top1_relevant &&
  pairwiseAccuracy >= corpus.acceptance.min_pairwise_accuracy &&
  meanNdcg >= corpus.acceptance.min_mean_ndcg;

console.log(
  `top1-relevant=${top1Rate.toFixed(3)} pairwise=${pairwiseAccuracy.toFixed(3)} mean-nDCG=${meanNdcg.toFixed(3)} result=${accepted ? "PASS" : "FAIL"}`,
);
if (!accepted) process.exitCode = 1;
