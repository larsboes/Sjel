// Is every model this machine names still real, still served, and still the newest of its line?
//
// Dependabot watches package registries; a model id is not a package, so nothing watched these.
// The cost of that was measured on 2026-08-30: `summarization_light` named `apple-on-device`
// months after apfel replaced the server that answered to it, and every unattended request 404'd
// with `model_not_found` — at request time, so it read as "the light rung never summarizes
// anything" rather than as a typo.
//
// Three questions, deliberately separate, because each has a different answer and a different
// cost:
//
//   1. Is the declared model in the provider's catalogue?  Absent = broken now.
//   2. Does it actually answer?  (--probe)  Listed is not served: `gemini-3.7-flash` is in the
//      catalogue and returns 503 "high demand" on the free tier, three attempts running.
//   3. Is something newer available in the same family?  Advisory, never a failure — newer is a
//      decision about quality, cost and capacity, and question 2 is why it cannot be automatic.
//
// The full sweep is not wired into `tools/doctor`: it dials third-party APIs and spends quota,
// which no check that runs on every invocation should do. `--local` is, because it restricts the
// sweep to loopback backends and so spends nothing and leaves the machine for nothing — and a
// stopped local server is precisely the condition nothing else reported. `doctor --online` probes
// declared systems endpoints; this one asks whether the model a ROLE names is really there.

import { readFileSync } from "node:fs";
import { join } from "node:path";

import { axonRoot, overlayRoot } from "./lib/overlay.ts";

interface Backend {
  api?: string;
  base_url?: string;
  api_key_file?: string;
}
interface Role {
  backend?: string;
  model?: string;
  max_requests_per_day?: number;
}
interface InferenceConfig {
  backends?: Record<string, Backend>;
  roles?: Record<string, Role>;
}

const PROBE = process.argv.includes("--probe");
// `--local` restricts the sweep to backends on loopback, which removes the one reason this tool
// stays out of `tools/doctor`: it spends no third-party quota and dials nothing off the machine.
// It implies probing, because on loopback a probe is free and "is it listed" without "does it
// answer" is the weaker half of the question a stopped server makes urgent.
const LOCAL_ONLY = process.argv.includes("--local");
/// Set by whichever probe last read a stated quota out of a refusal, consumed immediately after
/// by the role loop. A slot rather than a return value because both probes already answer with
/// the operator-facing string and this is the one extra fact either can learn.
let lastStatedQuota: number | undefined;
const JSON_OUT = process.argv.includes("--json");

function isLoopback(backend: Backend): boolean {
  try {
    return ["127.0.0.1", "localhost", "::1"].includes(
      new URL(backend.base_url ?? "").hostname,
    );
  } catch {
    return false;
  }
}

const SJEL_ROOT = axonRoot();
const OVERLAY = overlayRoot(SJEL_ROOT);
if (!OVERLAY) {
  console.error("model-check: no overlay — run tools/install.sh");
  process.exit(2);
}
const CONFIG_DIR = join(OVERLAY, "config");
const configPath = join(CONFIG_DIR, "inference.json");

let config: InferenceConfig;
try {
  config = JSON.parse(readFileSync(configPath, "utf8"));
} catch (error) {
  console.error(`model-check: cannot read ${configPath}: ${error}`);
  process.exit(2);
}

/** The key file is named relative to the CONFIG directory, the way libs/inference resolves it
 *  (`lib.rs`, `config_directory.join(raw)`). Reading it from the overlay root instead is how a
 *  present credential gets reported as missing — which it was, on 2026-08-30. */
function apiKey(backend: Backend): string | undefined {
  if (!backend.api_key_file) return undefined;
  const path = backend.api_key_file.startsWith("/")
    ? backend.api_key_file
    : join(CONFIG_DIR, backend.api_key_file);
  try {
    return readFileSync(path, "utf8").trim() || undefined;
  } catch {
    return undefined;
  }
}

/** Same-line comparison, on the numeric part of a family name.
 *
 *  `gemini-3.6-flash` and `gemini-3.7-flash` are the same line at different versions;
 *  `nemotron-3-nano` and `nemotron-3-super` are NOT — they are size tiers, and calling a bigger
 *  one "newer" would advise an upgrade nobody asked for onto a model with different economics.
 *  So the family is the id with its version numbers removed, and only an id that matches the
 *  family exactly, differing in those numbers, can be newer. */
function family(id: string): string {
  return id.replace(/[0-9]+(\.[0-9]+)*/g, "#");
}
function versionKey(id: string): number[] {
  return (id.match(/[0-9]+(\.[0-9]+)*/g) ?? []).flatMap((n) => n.split(".").map(Number));
}
function newer(candidate: string, current: string): boolean {
  const a = versionKey(candidate);
  const b = versionKey(current);
  for (let i = 0; i < Math.max(a.length, b.length); i += 1) {
    const left = a[i] ?? 0;
    const right = b[i] ?? 0;
    if (left !== right) return left > right;
  }
  return false;
}

async function catalogue(backend: Backend, key?: string): Promise<string[] | string> {
  const base = (backend.base_url ?? "").replace(/\/$/, "");
  if (!base) return "no base_url declared";
  const headers: Record<string, string> = {};
  if (key) headers.Authorization = `Bearer ${key}`;
  const url = backend.api === "ollama" ? `${base}/api/tags` : `${base}/models`;
  try {
    const response = await fetch(url, { headers, signal: AbortSignal.timeout(20_000) });
    if (!response.ok) return `catalogue unavailable (HTTP ${response.status})`;
    const body = await response.json();
    if (Array.isArray(body?.data)) {
      return body.data.map((m: { id: string }) => String(m.id).replace(/^models\//, ""));
    }
    if (Array.isArray(body?.models)) {
      return body.models.map((m: { name: string }) => String(m.name));
    }
    return "catalogue in an unrecognised shape";
  } catch {
    return "backend not reachable";
  }
}

/// Ask an embedding model for a vector.
///
/// Question 2 is "does it actually answer", and for a retrieval model the answer is a vector.
/// Asking `bge-m3` to reply "OK" gets an HTTP 4xx and reports a model that is installed, served
/// and correct as broken -- the same false alarm in the opposite direction from the one this
/// tool was written to catch.
/// What a refusal says about itself, when it says anything useful.
///
/// A 429 is where a provider states its real limit, and reading it is how `max_requests_per_day`
/// stops being a guess. On 2026-08-30 this deployment declared 1000/day for a Gemini role whose
/// free-tier quota is 20 -- the provider had been naming that number in every rejection for as
/// long as the role had been wrong, in `error.details[].violations[].quotaValue`. Cloudflare
/// meters something else entirely and says so differently, hence the fallback to a short excerpt
/// rather than a second provider-shaped parser.
async function refusalDetail(
  response: Response,
): Promise<{ text: string; statedQuota?: number }> {
  let raw: string;
  try {
    raw = await response.text();
  } catch {
    return { text: "" };
  }
  try {
    const parsed = JSON.parse(raw);
    const error = (Array.isArray(parsed) ? parsed[0] : parsed)?.error;
    for (const detail of error?.details ?? []) {
      for (const violation of detail?.violations ?? []) {
        if (violation?.quotaValue !== undefined) {
          const id = violation.quotaId ? ` (${violation.quotaId})` : "";
          const value = Number(violation.quotaValue);
          return {
            text: `provider states a quota of ${violation.quotaValue}${id}`,
            statedQuota: Number.isFinite(value) ? value : undefined,
          };
        }
      }
    }
    if (typeof error?.message === "string") {
      return { text: error.message.slice(0, 120) };
    }
  } catch {
    // Not JSON, or not a shape we know. The excerpt is still worth more than the bare status.
  }
  return { text: raw.replace(/\s+/g, " ").trim().slice(0, 120) };
}

/// The declared ceiling, checked against the one the provider just named.
///
/// `max_requests_per_day` is a LOCAL guard, so above the provider's real limit it guards
/// nothing -- and community directories are exactly where a wrong one comes from. The two lists
/// consulted on 2026-08-30 both put Groq at 14,400/day where Groq's own docs say 1,000 per model,
/// and both put Gemini in the thousands where the provider itself answers 20. Discovery is what
/// those lists are for; the number has to come from here.
let statedQuotaBreaches = 0;
function reportQuotaMismatch(roleName: string, declared: number | undefined, stated: number) {
  if (declared === undefined || declared <= stated) return;
  statedQuotaBreaches += 1;
  say(
    `  ! ${roleName} declares max_requests_per_day ${declared}, provider states ${stated} ` +
      `— the local guard is above the real ceiling and stops guarding`,
  );
}

async function probeEmbedding(
  backend: Backend,
  model: string,
  key?: string,
): Promise<string> {
  const base = (backend.base_url ?? "").replace(/\/$/, "");
  const headers: Record<string, string> = { "Content-Type": "application/json" };
  if (key) headers.Authorization = `Bearer ${key}`;
  // Two shapes, matching libs/inference `ResolvedRole::embedding_endpoint`: a second opinion
  // about which URL is correct is how this tool starts disagreeing with the thing it checks.
  const ollama = backend.api === "ollama";
  try {
    const response = await fetch(`${base}${ollama ? "/api/embed" : "/embeddings"}`, {
      method: "POST",
      headers,
      body: JSON.stringify(ollama ? { model, input: ["probe"] } : { model, input: "probe" }),
      signal: AbortSignal.timeout(60_000),
    });
    if (!response.ok) {
      const said = await refusalDetail(response);
      if (said.statedQuota !== undefined) lastStatedQuota = said.statedQuota;
      return `answers HTTP ${response.status}${said.text ? ` — ${said.text}` : ""}`;
    }
    const body = await response.json();
    const vector = ollama ? body?.embeddings?.[0] : body?.data?.[0]?.embedding;
    return Array.isArray(vector) && vector.length
      ? `answers (${vector.length}-dimensional)`
      : "answered without a vector";
  } catch {
    return "no answer within 60s";
  }
}

async function probe(backend: Backend, model: string, key?: string): Promise<string> {
  const base = (backend.base_url ?? "").replace(/\/$/, "");
  const headers: Record<string, string> = { "Content-Type": "application/json" };
  if (key) headers.Authorization = `Bearer ${key}`;
  try {
    // Mirrors libs/inference `ResolvedRole::chat_completions_endpoint`: an OpenAI backend's
    // base_url already carries its version segment, an Ollama one does not. Wrong here and a
    // served model answers 404 and reads as absent -- the failure this tool reports, arriving
    // from the tool itself.
    const path =
      backend.api === "ollama" ? "/v1/chat/completions" : "/chat/completions";
    const response = await fetch(`${base}${path}`, {
      method: "POST",
      headers,
      // Generous, because a reasoning model spends its budget thinking and a stingy cap comes
      // back as `finish_reason: length` — which would make a healthy model look broken here,
      // the mirror of the bug that put 15 chains of thought in the Feed.
      body: JSON.stringify({
        model,
        messages: [{ role: "user", content: "Reply with one word: OK" }],
        max_tokens: 2000,
      }),
      signal: AbortSignal.timeout(60_000),
    });
    if (!response.ok) {
      const said = await refusalDetail(response);
      if (said.statedQuota !== undefined) lastStatedQuota = said.statedQuota;
      return `answers HTTP ${response.status}${said.text ? ` — ${said.text}` : ""}`;
    }
    const body = await response.json();
    return body?.choices?.[0]?.message ? "answers" : "answered without a message";
  } catch {
    return "no answer within 60s";
  }
}

let failures = 0;
const entries: {
  role: string;
  backend: string;
  model: string;
  status: "ok" | "missing" | "unreachable" | "incomplete";
  detail: string;
}[] = [];
const say = (line: string) => {
  if (!JSON_OUT) console.log(line);
};
const roles = Object.entries(config.roles ?? {}).filter(([, role]) => {
  if (!LOCAL_ONLY) return true;
  const backend = config.backends?.[role.backend ?? ""];
  return backend ? isLoopback(backend) : false;
});
const cache = new Map<string, string[] | string>();

for (const [roleName, role] of roles) {
  const backendName = role.backend ?? "";
  const backend = config.backends?.[backendName];
  const model = role.model ?? "";
  if (!backend || !model) {
    say(`${roleName}: incomplete declaration (backend='${backendName}' model='${model}')`);
    entries.push({
      role: roleName, backend: backendName, model,
      status: "incomplete", detail: "no backend or no model declared",
    });
    failures += 1;
    continue;
  }
  const key = apiKey(backend);
  // A backend that names a key file it cannot read is not broken, it is not provisioned -- the
  // ordinary state of a provider that has been declared and whose Vaultwarden item has not been
  // materialized yet. Dialling it anyway spends a round trip to be told 401, and reports a
  // pending setup step as a fault. `libs/inference::credential_ready` refuses the same way
  // before any request, which is why comms never sends one either.
  if (backend.api_key_file && key === undefined) {
    say(`${roleName}: ${model} on ${backendName} — credential unavailable, not probed ` +
        `(materialize its key to enable this role)`);
    entries.push({
      role: roleName, backend: backendName, model,
      status: "unreachable", detail: "credential unavailable — key file is absent or empty",
    });
    continue;
  }
  if (!cache.has(backendName)) cache.set(backendName, await catalogue(backend, key));
  const listed = cache.get(backendName)!;

  // `libs/inference` resolves rungs by name -- `role_on("embedding", ...)` is how a caller asks
  // for the retrieval rung -- so the name is the existing contract for what a role is, and the
  // right question to ask it follows from that rather than from a new field.
  const asksForAVector = /(^|_)embedding(_|$)/.test(roleName);
  const ask = async () => {
    lastStatedQuota = undefined;
    const reply = await (asksForAVector ? probeEmbedding : probe)(backend, model, key);
    if (lastStatedQuota !== undefined) {
      reportQuotaMismatch(roleName, role.max_requests_per_day, lastStatedQuota);
    }
    return reply;
  };
  const answered = (reply: string) =>
    reply === "answers" || reply.startsWith("answers (");

  if (typeof listed === "string") {
    // No catalogue is never a failure by itself: a local backend that is not running and a
    // provider that publishes no model list are both ordinary.
    //
    // But it used to skip the PROBE too, and that is the wrong half to drop. Question 1 asks
    // whether the model is listed; question 2 asks whether it answers, and question 2 is still
    // answerable when the list is missing. Cloudflare's catalogue 405s here, so the provider
    // that wrote 160 of this machine's 298 digests was the one role the tool said nothing
    // about at all.
    const reply = PROBE || LOCAL_ONLY ? await ask() : "";
    const detail = reply ? `${listed}, but ${reply}` : listed;
    say(`${roleName}: ${model} on ${backendName} — ${detail}`);
    entries.push({
      role: roleName, backend: backendName, model,
      // Answering is what the role is for. A model that answers while its provider publishes
      // no list is working, and calling that unreachable would be this tool contradicting the
      // request that just succeeded.
      status: answered(reply) ? "ok" : "unreachable",
      detail,
    });
    continue;
  }

  // Ollama resolves an untagged name to its `:latest` tag: `ollama run bge-m3` and the
  // `bge-m3:latest` its catalogue reports are one model. A role naming the untagged form names
  // it the way the CLI does, so an exact-string test called an installed, answering model
  // missing -- and a check that cries wolf is the one the next person learns to ignore.
  const declaredAs = (id: string): string[] =>
    backend.api === "ollama" && id.endsWith(":latest")
      ? [id, id.slice(0, -":latest".length)]
      : [id];
  const present = listed.some((id) => declaredAs(id).includes(model));
  const candidates = listed.filter((id) => family(id) === family(model) && newer(id, model));
  const suffix = candidates.length ? `  newer available: ${candidates.sort().join(", ")}` : "";

  if (!present) {
    failures += 1;
    const near = listed.filter((id) => family(id) === family(model));
    const near_note = near.length ? ` — same family: ${near.sort().join(", ")}` : "";
    say(`${roleName}: ✗ ${model} is NOT in ${backendName}'s catalogue${near_note}`);
    entries.push({
      role: roleName, backend: backendName, model,
      status: "missing", detail: `not in ${backendName}'s catalogue${near_note}`,
    });
    continue;
  }
  const spoke = PROBE || LOCAL_ONLY ? await ask() : "";
  say(`${roleName}: ${model} on ${backendName} ok${spoke ? ` — ${spoke}` : ""}${suffix}`);
  entries.push({
    role: roleName, backend: backendName, model,
    // A probe that came back as an HTTP status or a timeout is not an ok. Reporting it as one
    // would make this tool agree that a served-but-silent model is fine, which is question 2's
    // entire reason for being separate from question 1.
    status: spoke && !answered(spoke) ? "unreachable" : "ok",
    detail: spoke || "listed",
  });
}

if (JSON_OUT) {
  console.log(
    JSON.stringify({
      scope: LOCAL_ONLY ? "loopback" : "all",
      entries,
      stated_quota_breaches: statedQuotaBreaches,
      totals: {
        count: entries.length,
        ok: entries.filter((e) => e.status === "ok").length,
        missing: entries.filter((e) => e.status === "missing").length,
        unreachable: entries.filter((e) => e.status === "unreachable").length,
        incomplete: entries.filter((e) => e.status === "incomplete").length,
      },
    }),
  );
} else {
  const probed = PROBE || LOCAL_ONLY;
  console.log(
    `\n${roles.length} role(s) checked${LOCAL_ONLY ? " on loopback" : ""}` +
      `${probed ? ", each probed" : " (pass --probe to ask each one to answer)"}` +
      `, ${failures} naming a model its backend does not list` +
      (statedQuotaBreaches
        ? `, ${statedQuotaBreaches} declaring a daily ceiling above the provider's own`
        : ""),
  );
}
if (failures > 0) process.exit(1);
