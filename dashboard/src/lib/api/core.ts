import { bridgeErrorText, inTauri, macRequest, NOT_CONFIGURED, type MacResponse } from '../mac-bridge';

export class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
  }
}

/**
 * The message inside an error response body. Every capability server answers
 * `{"error": "..."}`, so without this the UI shows a reader the raw JSON —
 * which is what the feed's paste box did on a 404 until this was fixed.
 */
function errorMessage(body: string): string {
  try {
    const parsed = JSON.parse(body);
    if (parsed && typeof parsed === 'object' && typeof parsed.error === 'string') return parsed.error;
  } catch {
    // Not JSON — the raw body is the best message available, unless it is a web page.
  }
  // An HTML page is a host's or proxy's page, not the capability's message: printing it put a
  // whole GitHub 404 page on the demo's People page (2026-09-27).
  if (isHtml(body)) return '';
  return body;
}

function isHtml(body: string): boolean {
  return /<\s*!doctype\s+html|<\s*html[\s>]/i.test(body);
}

/**
 * The capability a proxied path belongs to. Every capability call goes through
 * `/<name>/…`, which is the dev proxy's uniform rule, so the first segment is the
 * name without anything having to declare it twice.
 */
function capabilityFrom(path: string): string | null {
  const segment = path.replace(/^\/+/, '').split(/[/?#]/)[0];
  return segment && !segment.startsWith('api') ? segment : null;
}

/**
 * What to show a reader when a request fails.
 *
 * The case worth naming: when a capability's process is not running, the dev proxy
 * cannot reach it and answers with a bare 5xx and an empty, non-JSON body. Every
 * page then rendered `Request failed (500)`, which says nothing at all — it reads
 * as a bug in the page rather than as a service that was never started, and it
 * sends the reader looking in the wrong place. There is nothing wrong in that
 * situation except that nobody started the thing, so it says so, with the command.
 */
export function describeFailure(status: number, body: string, path: string): string {
  const capability = capabilityFrom(path);
  // A 404 that answers with a web page came from the host or a proxy, not from the capability:
  // the capability is not served here (the demo site has no entities, for one). A 404 with no
  // body or a JSON body stays a wrong route, below.
  if (capability && status === 404 && isHtml(body)) {
    return `${capability} is not available here (404)`;
  }
  const named = errorMessage(body).trim();
  if (named) return capability ? `${capability}: ${named}` : named;
  if (capability && status >= 500) {
    return `${capability} is not running — start it with: tools/service-runner.sh start ${capability}`;
  }
  return capability ? `${capability}: request failed (${status})` : `Request failed (${status})`;
}

/** Status and body text of one answer, whichever way it travelled. */
export interface Answer {
  ok: boolean;
  status: number;
  text: string;
}

/**
 * In the Tauri app a relative path has no server behind it, so a node path goes
 * through the native bridge (`./mac-bridge.ts`). An absolute URL, such as the
 * Wikipedia lookups below, is not a Mac path and stays a plain fetch.
 */
export async function send(path: string, init?: RequestInit): Promise<Answer> {
  if (inTauri() && path.startsWith('/')) {
    let answer: MacResponse;
    try {
      answer = await macRequest(path, init);
    } catch (error) {
      const message = bridgeErrorText(error);
      if (message.startsWith(NOT_CONFIGURED)) {
        throw new ApiError(0, 'The Sjel node address is not set. Set it in Sjel connection (footer).');
      }
      throw new ApiError(0, message);
    }
    return { ok: answer.status >= 200 && answer.status < 300, status: answer.status, text: answer.body };
  }
  const res = await fetch(path, init);
  if (!res.ok) return { ok: false, status: res.status, text: await res.text().catch(() => '') };
  return { ok: true, status: res.status, text: await res.text() };
}

export async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await send(path, init);
  if (!res.ok) {
    throw new ApiError(res.status, describeFailure(res.status, res.text, path));
  }
  const text = res.text;
  if (!text) return undefined as T;

  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    const capability = capabilityFrom(path);
    const html = /<\s*!doctype\s+html|<\s*html[\s>]/i.test(text);
    const message = html
      ? `${capability ?? "API"} did not return JSON; its service is unavailable here`
      : `${capability ?? "API"} returned invalid JSON`;
    throw new ApiError(res.status, message);
  }
  if (parsed && typeof parsed === 'object' && 'error' in parsed && parsed.error) {
    throw new ApiError(res.status, String(parsed.error));
  }
  return parsed as T;
}

export const jsonInit = (method: string, body: unknown): RequestInit => ({
  method,
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify(body),
});
