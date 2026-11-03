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

type EmbeddingDatum = {
  index: number;
  embedding: number[];
};

const corpusPath = resolve(
  process.argv[2] ?? new URL("./relevance-corpus.json", import.meta.url).pathname,
);
const settingsPath =
  process.env.OMLX_SETTINGS_PATH ?? `${homedir()}/.omlx/settings.json`;
const baseUrl = process.env.OMLX_BASE_URL ?? "http://127.0.0.1:8000/v1";
const model = process.env.OMLX_EMBEDDING_MODEL ?? "multilingual-e5-base-mlx";
// Same escape hatch, same spelling, as run-reranking.ts: a loopback server that
// enforces no key of its own. Ollama's OpenAI-compatible endpoint is one, and
// without this the only way to measure a candidate served by it is a throwaway
// settings file holding a key nothing reads -- a shim that makes the run
// unreproducible and is therefore not evidence.
const noAuth = process.env.OMLX_NO_AUTH === "1";
// The role carries its own `query_prefix`/`document_prefix` (libs/inference), and a model that
// wants none is as ordinary as one that wants E5's. Measuring a candidate under prefixes it will
// not run with is measuring a configuration nobody deploys, so these are overridable here under
// the field names the role uses. Empty string is a real value and must survive `??`.
const queryPrefix = process.env.OMLX_QUERY_PREFIX ?? "query: ";
const documentPrefix = process.env.OMLX_DOCUMENT_PREFIX ?? "passage: ";
const queryVariant = process.env.RELEVANCE_EVAL_QUERY_VARIANT?.trim() || null;

function fail(message: string): never {
  throw new Error(`relevance eval: ${message}`);
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

function cosine(left: number[], right: number[]): number {
  if (left.length === 0 || left.length !== right.length) return 0;
  let dot = 0;
  let leftNorm = 0;
  let rightNorm = 0;
  for (let index = 0; index < left.length; index += 1) {
    dot += left[index] * right[index];
    leftNorm += left[index] ** 2;
    rightNorm += right[index] ** 2;
  }
  return dot / Math.sqrt(leftNorm * rightNorm);
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

const inputs: string[] = [];
const queryIndexes = new Map<string, number>();
const candidateIndexes = new Map<string, number>();
const candidateKey = (queryId: string, candidateId: string): string =>
  `${queryId}\u0000${candidateId}`;
const queryText = (query: Query): string =>
  queryVariant ? query.text_variants![queryVariant] : query.text;
for (const query of corpus.queries) {
  queryIndexes.set(
    query.id,
    inputs.push(`${queryPrefix}${queryText(query)}`) - 1,
  );
  for (const candidate of query.candidates) {
    candidateIndexes.set(
      candidateKey(query.id, candidate.id),
      inputs.push(`${documentPrefix}${candidate.text}`) - 1,
    );
  }
}

const response = await fetch(
  new URL(`${endpoint.pathname.replace(/\/$/, "")}/embeddings`, endpoint),
  {
    method: "POST",
    headers: {
      ...(apiKey ? { Authorization: `Bearer ${apiKey}` } : {}),
      "Content-Type": "application/json",
    },
    body: JSON.stringify({ model, input: inputs }),
  },
);
if (!response.ok) {
  fail(`oMLX returned HTTP ${response.status}`);
}
const body = (await response.json()) as { data?: EmbeddingDatum[] };
const vectors = (body.data ?? [])
  .sort((left, right) => left.index - right.index)
  .map((datum) => datum.embedding);
if (
  vectors.length !== inputs.length ||
  vectors.some((vector) => vector.length === 0)
) {
  fail(`expected ${inputs.length} non-empty vectors, received ${vectors.length}`);
}

let top1Relevant = 0;
let pairwiseCorrect = 0;
let pairwiseTotal = 0;
let ndcgTotal = 0;

console.log(
  `Comms DE/EN relevance eval · ${model} · ${vectors[0].length} dimensions · queries=${queryVariant ?? "default"}`,
);
for (const query of corpus.queries) {
  const queryVector = vectors[queryIndexes.get(query.id)!];
  const ranked = query.candidates
    .map((candidate) => ({
      ...candidate,
      score: cosine(
        queryVector,
        vectors[candidateIndexes.get(candidateKey(query.id, candidate.id))!],
      ),
    }))
    .sort((left, right) => right.score - left.score);
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
