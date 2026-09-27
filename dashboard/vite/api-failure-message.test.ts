import { describe, expect, test } from 'bun:test';
import { describeFailure } from '../src/lib/api';

// The case this exists for: a capability whose process is not running. The dev
// proxy cannot reach it, answers with a bare 5xx and an empty non-JSON body, and
// every page rendered "Request failed (500)". That reads as a bug in the page
// rather than a service nobody started, which sends the reader to the wrong place.
describe('describeFailure', () => {
  test('an HTML page is never the message, and a 404 names the missing capability', () => {
    const page = '<!DOCTYPE html><html><head><title>404</title></head><body>File not found</body></html>';
    expect(describeFailure(404, page, '/entities/api/entities')).toBe('entities is not available here (404)');
    expect(describeFailure(502, page, '/finance/api/x')).not.toContain('<html');
  });

  test('a bare 5xx from a proxied capability names the capability and the fix', () => {
    const message = describeFailure(500, '', '/finance/api/subscriptions');
    expect(message).toContain('finance is not running');
    expect(message).toContain('tools/service-runner.sh start finance');
  });

  test('the same holds for a gateway status, which is what a refused upstream can also produce', () => {
    expect(describeFailure(502, '', '/vault/api/tasks')).toContain('vault is not running');
    expect(describeFailure(503, '   ', '/trips/api/plans')).toContain('trips is not running');
  });

  test("a capability's own error message wins, because it knows more than we do", () => {
    const body = JSON.stringify({ error: 'no vault configured; set the overlay config' });
    expect(describeFailure(409, body, '/finance/api/writeback')).toBe(
      'finance: no vault configured; set the overlay config',
    );
  });

  test('a 500 that carries a real message is not rewritten into a start hint', () => {
    const body = JSON.stringify({ error: 'db error: connection refused' });
    expect(describeFailure(500, body, '/finance/api/subscriptions')).toBe(
      'finance: db error: connection refused',
    );
  });

  test('a 4xx with no body keeps the plain fallback rather than blaming the process', () => {
    // A 404 means the route is wrong, not that nothing is listening. Telling
    // someone to start a service that is already up would waste their time.
    expect(describeFailure(404, '', '/finance/api/nonsense')).toBe('finance: request failed (404)');
  });

  test('a dashboard-local route is not attributed to a capability', () => {
    // No capability owns /api/*, so nothing gets blamed and no start hint is offered.
    expect(describeFailure(500, '', '/api/top-processes')).toBe('Request failed (500)');
  });

  test('every message names who it is about, so no page prepends its own prefix', () => {
    // The prefix the three pages carried produced "finance unavailable — finance is
    // not running". One sentence, one owner, written once.
    for (const message of [
      describeFailure(500, '', '/finance/api/subscriptions'),
      describeFailure(409, '{"error":"nope"}', '/finance/api/writeback'),
      describeFailure(404, '', '/finance/api/nonsense'),
    ]) {
      expect(message.startsWith('finance')).toBe(true);
    }
  });

  test('a non-JSON body is still shown, because it is the best message available', () => {
    expect(describeFailure(500, 'upstream timed out', '/comms/api/feed')).toBe(
      'comms: upstream timed out',
    );
  });
});
