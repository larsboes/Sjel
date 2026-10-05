import { afterEach, describe, expect, test } from 'bun:test';
import { interior, InteriorConflict } from '../src/lib/api';

// PRD §10 A5: the phone and the Mac edit the same interior items. A write that carries a stale
// revision gets 409 from the capability, and the page must show the values that stand now
// instead of retrying and overwriting them. The contract is capabilities/interior/README.md,
// section "Gleichzeitiges Bearbeiten".
//
// Nothing here reaches the network. `fetch` is replaced per test and restored.

const realFetch = globalThis.fetch;
afterEach(() => {
  globalThis.fetch = realFetch;
});

interface Call {
  method: string;
  path: string;
  ifMatch: string | null;
}

function serve(answers: Record<string, () => Response>): Call[] {
  const calls: Call[] = [];
  globalThis.fetch = (async (input: string, init?: RequestInit) => {
    const method = init?.method ?? 'GET';
    const headers = new Headers(init?.headers);
    calls.push({ method, path: input, ifMatch: headers.get('If-Match') });
    const answer = answers[`${method} ${input}`];
    if (!answer) throw new Error(`unexpected ${method} ${input}`);
    return answer();
  }) as unknown as typeof fetch;
  return calls;
}

const current = { id: 'schrank', kind: 'piece', label: 'vom Mac', b: 120, revision: 2 };

describe('interior item writes', () => {
  test('a 409 surfaces as InteriorConflict with the current values and is not retried', async () => {
    const calls = serve({
      'PATCH /inventory/api/items/schrank': () =>
        new Response(
          JSON.stringify({
            error: '`schrank` wurde inzwischen geaendert: erwartet Revision 1, aktuell 2',
            current: { item: current, state: 'owned' },
          }),
          { status: 409 },
        ),
      'GET /inventory/api/inventory': () =>
        new Response(JSON.stringify([{ item: current, state: 'owned' }]), { status: 200 }),
    });

    const failure = await interior.patchItem('schrank', { b: 90 }, 1).catch((cause) => cause);

    expect(failure).toBeInstanceOf(InteriorConflict);
    expect(failure.status).toBe(409);
    expect(failure.message).toContain('inzwischen geaendert');
    expect(failure.current.item.label).toBe('vom Mac');
    expect(failure.current.item.revision).toBe(2);
    const writes = calls.filter((c) => c.method === 'PATCH');
    expect(writes).toHaveLength(1);
    expect(writes[0].ifMatch).toBe('"1"');
  });

  test('a matching write sends the revision it read and returns the next one', async () => {
    const calls = serve({
      'PUT /inventory/api/items/schrank': () =>
        new Response(JSON.stringify({ id: 'schrank', ok: true, revision: 3 }), { status: 200 }),
    });

    const result = await interior.saveItem('schrank', current as never, 2);

    expect(result.revision).toBe(3);
    expect(calls).toEqual([{ method: 'PUT', path: '/inventory/api/items/schrank', ifMatch: '"2"' }]);
  });

  test('without a revision no If-Match is sent, as before A5', async () => {
    const calls = serve({
      'PATCH /inventory/api/items/schrank': () =>
        new Response(JSON.stringify({ id: 'schrank', ok: true, revision: 4 }), { status: 200 }),
    });

    await interior.patchItem('schrank', { b: 80 });

    expect(calls[0].ifMatch).toBeNull();
  });
});
