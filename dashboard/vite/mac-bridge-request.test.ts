import { afterEach, describe, expect, mock, test } from 'bun:test';
import { ApiError, request } from '../src/lib/api';
import { NOT_CONFIGURED } from '../src/lib/mac-bridge';

type Invoke = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;

const realFetch = globalThis.fetch;
const g = globalThis as unknown as { window?: Record<string, unknown> };

function enterTauri(invoke: Invoke) {
  g.window = { __TAURI_INTERNALS__: { invoke, transformCallback: () => 0 } };
}

afterEach(() => {
  delete g.window;
  globalThis.fetch = realFetch;
});

describe('request()', () => {
  test('uses plain fetch outside Tauri', async () => {
    const fetchMock = mock(async () => new Response('{"items":[1]}', { status: 200 }));
    globalThis.fetch = fetchMock as unknown as typeof fetch;
    const result = await request<{ items: number[] }>('/interior/api/items');
    expect(result.items).toEqual([1]);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect((fetchMock.mock.calls[0] as unknown[])[0]).toBe('/interior/api/items');
  });

  test('uses the native bridge inside Tauri, not fetch', async () => {
    const fetchMock = mock(async () => new Response('should not be used'));
    globalThis.fetch = fetchMock as unknown as typeof fetch;
    const invoke = mock<Invoke>(async () => ({
      status: 200,
      content_type: 'application/json',
      body: '{"ok":true}',
    }));
    enterTauri(invoke);

    const result = await request<{ ok: boolean }>('/interior/api/items', {
      method: 'post',
      headers: { 'Content-Type': 'application/json', 'If-Match': '"r1"' },
      body: '{"name":"lamp"}',
    });

    expect(result.ok).toBe(true);
    expect(fetchMock).not.toHaveBeenCalled();
    expect(invoke).toHaveBeenCalledTimes(1);
    const [cmd, args] = invoke.mock.calls[0];
    expect(cmd).toBe('mac_request');
    expect(args).toEqual({
      request: {
        method: 'POST',
        path: '/interior/api/items',
        body: '{"name":"lamp"}',
        headers: [
          ['Content-Type', 'application/json'],
          ['If-Match', '"r1"'],
        ],
      },
    });
  });

  test('keeps describeFailure semantics for an error answer in Tauri', async () => {
    enterTauri(async () => ({ status: 404, content_type: 'application/json', body: '{"error":"no such item"}' }));
    const error = await request('/interior/api/items/x').catch((e) => e);
    expect(error).toBeInstanceOf(ApiError);
    expect(error.status).toBe(404);
    expect(error.message).toBe('interior: no such item');
  });

  test('names the setting when no Sjel node address is set', async () => {
    enterTauri(async () => {
      throw `${NOT_CONFIGURED}: set the Sjel node address in Settings, Sjel connection`;
    });
    const error = await request('/interior/api/items').catch((e) => e);
    expect(error).toBeInstanceOf(ApiError);
    expect(error.message).toContain('Sjel node address is not set');
    expect(error.message).toContain('Sjel connection');
  });

  test('an absolute URL stays a plain fetch inside Tauri', async () => {
    const fetchMock = mock(async () => new Response('{"q":1}'));
    globalThis.fetch = fetchMock as unknown as typeof fetch;
    const invoke = mock<Invoke>(async () => ({}));
    enterTauri(invoke);
    await request('https://de.wikipedia.org/w/api.php?x=1');
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(invoke).not.toHaveBeenCalled();
  });
});
