// The drawer's local-time logic is only meaningful in a zone with an offset. Set before any
// Date is built; Bun applies a runtime TZ change.
const previousTz = process.env.TZ;
process.env.TZ = 'Europe/Berlin';

import { afterAll, afterEach, beforeEach, describe, expect, it } from 'bun:test';
import type { CalendarEntry, Journey, TripPlan } from '../src/lib/api';
import { assistantEngine } from '../src/lib/assistant/assistant-engine';
import {
  addDays,
  checkSlot,
  findFreeSlots,
  instantMinutes,
  localDate,
  overlaps,
} from '../src/lib/assistant/calendar-slots';
import { extractRouteContext } from '../src/lib/assistant/context';
import {
  executeCalendarAccept,
  executeJourneyPin,
  settleCardAction,
} from '../src/lib/assistant/executor';
import { routeByKeywords } from '../src/lib/assistant/keyword-router';

// ─── fetch mock ──────────────────────────────────────────────────────────────

interface Call {
  url: string;
  method: string;
  body: unknown;
}

let calls: Call[] = [];
const realFetch = globalThis.fetch;

type Responder = (url: string, init?: RequestInit) => Response | Promise<Response>;

function mockFetch(respond: Responder): void {
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    calls.push({
      url,
      method: init?.method ?? 'GET',
      body: typeof init?.body === 'string' ? JSON.parse(init.body) : undefined,
    });
    return respond(url, init);
  }) as typeof fetch;
}

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });

const backendDown: Responder = () => {
  throw new TypeError('fetch failed: connection refused');
};

beforeEach(() => {
  calls = [];
});
afterEach(() => {
  globalThis.fetch = realFetch;
});
afterAll(() => {
  if (previousTz === undefined) delete process.env.TZ;
  else process.env.TZ = previousTz;
});

// ─── fixtures ────────────────────────────────────────────────────────────────

function entry(title: string, starts_at: string, ends_at: string, all_day = false): CalendarEntry {
  return {
    id: `cal:entry:${title}`,
    kind: 'event',
    commitment: 'committed',
    title,
    starts_at,
    ends_at,
    all_day,
    location: null,
    notes: null,
    source: 'manual',
    external_id: null,
    rhythm_id: null,
    payload: null,
    created_at: '2026-09-01T00:00:00',
    updated_at: '2026-09-01T00:00:00',
  };
}

const journey: Journey = {
  id: 'hafas:abc',
  start_station: { id: '8000105', name: 'Frankfurt(Main)Hbf' },
  end_station: { id: '8011160', name: 'Berlin Hbf' },
  legs: [
    {
      origin: { id: '8000105', name: 'Frankfurt(Main)Hbf' },
      destination: { id: '8011160', name: 'Berlin Hbf' },
      departure_time: '2026-10-16T08:13:00',
      arrival_time: '2026-10-16T12:21:00',
      train_name: 'ICE 1537',
      train_number: '1537',
      train_category: 'ICE',
      is_regional: false,
    },
  ],
  total_duration_minutes: 248,
  total_price: 42.9,
  delay_risk_score: null,
};

const plan = (status: TripPlan['status']): Pick<TripPlan, 'id' | 'title' | 'status'> => ({
  id: 'plan:1',
  title: 'Berlin',
  status,
});

// ─── finding 1: the calendar write ───────────────────────────────────────────

describe('executeCalendarAccept', () => {
  it('sends naive local wall times, no Z, exactly as the calendar parser accepts', async () => {
    mockFetch(() => json({ id: 'cal:entry:1' }, 201));
    const result = await executeCalendarAccept({
      title: 'Focus block',
      startsAt: '2026-10-15T09:30:00',
      endsAt: '2026-10-15T11:30:00',
    });
    expect(result.ok).toBe(true);
    expect(calls).toHaveLength(1);
    expect(calls[0].url).toBe('/calendar/api/entries');
    expect(calls[0].method).toBe('POST');
    const body = calls[0].body as { starts_at: string; ends_at: string; commitment: string };
    expect(body.starts_at).toBe('2026-10-15T09:30:00');
    expect(body.commitment).toBe('planned');
    expect(body.ends_at).toBe('2026-10-15T11:30:00');
  });

  it('refuses a Z-suffixed instant before sending it', async () => {
    mockFetch(() => json({}, 201));
    const result = await executeCalendarAccept({
      title: 'Focus block',
      startsAt: '2026-10-15T09:30:00Z',
      endsAt: '2026-10-15T11:30:00Z',
    });
    expect(result.ok).toBe(false);
    expect(calls).toHaveLength(0);
  });

  it('propagates a 400 as ok:false with the calendar message', async () => {
    mockFetch(() => json({ error: 'starts_at must be a date or local time' }, 400));
    const result = await executeCalendarAccept({
      title: 'Focus block',
      startsAt: '2026-10-15T09:30:00',
      endsAt: '2026-10-15T11:30:00',
    });
    expect(result.ok).toBe(false);
    expect(result.message).toContain('starts_at must be a date or local time');
  });
});

describe('settleCardAction (what a card shows)', () => {
  it('shows applied only when the executor returned ok', async () => {
    expect(await settleCardAction(async () => ({ ok: true, message: 'done' }))).toEqual({
      kind: 'applied',
      message: 'done',
    });
  });

  it('shows the failure, not applied, when the executor returned ok:false', async () => {
    mockFetch(() => json({ error: 'calendar: bad instant' }, 400));
    const status = await settleCardAction(() =>
      executeCalendarAccept({ title: 'x', startsAt: '2026-10-15T09:30:00', endsAt: '2026-10-15T10:30:00' }),
    );
    expect(status.kind).toBe('failed');
    expect(status.kind === 'failed' && status.message).toContain('bad instant');
  });

  it('shows the failure when the action throws', async () => {
    const status = await settleCardAction(async () => {
      throw new Error('boom');
    });
    expect(status).toEqual({ kind: 'failed', message: 'boom' });
  });
});

// ─── finding 5: overlap math in local time ───────────────────────────────────

describe('calendar slot math', () => {
  it('names the local date, not the UTC one, between 00:00 and 02:00 in Berlin', () => {
    const justAfterMidnight = new Date(2026, 9, 16, 0, 30); // 2026-10-15T22:30Z
    expect(justAfterMidnight.toISOString().slice(0, 10)).toBe('2026-10-15');
    expect(localDate(justAfterMidnight)).toBe('2026-10-16');
  });

  it('treats ends as exclusive', () => {
    expect(overlaps({ start: 0, end: 60 }, { start: 60, end: 120 })).toBe(false);
    expect(overlaps({ start: 0, end: 61 }, { start: 60, end: 120 })).toBe(true);
  });

  it('reads date-only and wall-time instants, and rejects offsets', () => {
    expect(instantMinutes('2026-10-15T09:30:00')).toBe(instantMinutes('2026-10-15')! + 570);
    expect(instantMinutes('2026-10-15T09:30')).toBe(instantMinutes('2026-10-15')! + 570);
    expect(instantMinutes('2026-10-15T09:30:00Z')).toBeNull();
    expect(instantMinutes('2026-02-30')).toBeNull();
  });

  it('blocks the whole day for an all-day entry', () => {
    const allDay = entry('Offsite', '2026-10-15', '2026-10-16', true);
    expect(checkSlot('2026-10-15T09:30:00', '2026-10-15T11:30:00', [allDay]).conflicts).toHaveLength(1);
    expect(checkSlot('2026-10-15T23:00:00', '2026-10-16T00:00:00', [allDay]).conflicts).toHaveLength(1);
  });

  it('does not let an all-day entry leak into the next day', () => {
    const allDay = entry('Offsite', '2026-10-15', '2026-10-16', true);
    expect(checkSlot('2026-10-16T00:00:00', '2026-10-16T01:00:00', [allDay]).conflicts).toHaveLength(0);
  });

  it('checks a slot that straddles midnight against the next day', () => {
    const earlyNext = entry('Night train', '2026-10-16T00:30:00', '2026-10-16T02:00:00');
    expect(checkSlot('2026-10-15T23:00:00', '2026-10-16T01:00:00', [earlyNext]).conflicts).toHaveLength(1);
    expect(checkSlot('2026-10-15T22:00:00', '2026-10-15T23:59:00', [earlyNext]).conflicts).toHaveLength(0);
  });

  it('finds a timed overlap at any minute, not only by hour prefix', () => {
    const standup = entry('Standup', '2026-10-15T11:15:00', '2026-10-15T11:45:00');
    expect(checkSlot('2026-10-15T09:30:00', '2026-10-15T11:30:00', [standup]).conflicts).toHaveLength(1);
  });

  it('offers only slots that were each checked free', () => {
    const entries = [
      entry('Review', '2026-10-15T09:00:00', '2026-10-15T12:00:00'),
      entry('Lunch', '2026-10-15T13:00:00', '2026-10-15T14:00:00'),
    ];
    const free = findFreeSlots(
      { date: '2026-10-15', windowStart: '09:00', windowEnd: '18:00', durationMinutes: 120 },
      entries,
    );
    expect(free.map((slot) => slot.startsAt)).toEqual(['2026-10-15T14:00:00', '2026-10-15T16:00:00']);
    for (const slot of free) {
      expect(checkSlot(slot.startsAt, slot.endsAt, entries).conflicts).toHaveLength(0);
    }
  });

  it('offers nothing when an all-day entry covers the window', () => {
    const free = findFreeSlots(
      { date: '2026-10-15', windowStart: '09:00', windowEnd: '18:00', durationMinutes: 60 },
      [entry('Holiday', '2026-10-15', '2026-10-16', true)],
    );
    expect(free).toHaveLength(0);
  });

  it('adds days without a zone', () => {
    expect(addDays('2026-10-24', 1)).toBe('2026-10-25');
    expect(addDays('2026-10-25', 1)).toBe('2026-10-26'); // DST end in Berlin
    expect(addDays('2026-12-31', 1)).toBe('2027-01-01');
  });
});

describe('engine: calendar', () => {
  it('fetches tomorrow as [tomorrow, tomorrow+1) in local time and proposes checked slots', async () => {
    mockFetch(() => json([entry('Review', '2026-10-16T09:00:00', '2026-10-16T10:30:00')]));
    const now = new Date(2026, 9, 15, 0, 30); // local 00:30, UTC still the 14th
    const reply = await assistantEngine.processQuery(
      'find a 2-hour focus block tomorrow',
      extractRouteContext('/calendar'),
      now,
    );
    expect(calls[0].url).toBe('/calendar/api/entries?from=2026-10-16&to=2026-10-17');
    const slots = reply.cards?.filter((c) => c.type === 'calendar_slot') ?? [];
    expect(slots.length).toBeGreaterThan(0);
    const first = slots[0];
    expect(first.type === 'calendar_slot' && first.data.startsAt).toBe('2026-10-16T10:30:00');
    expect(first.type === 'calendar_slot' && first.data.endsAt).toBe('2026-10-16T12:30:00');
  });

  it('proposes nothing and names calendar when it does not answer', async () => {
    mockFetch(backendDown);
    const reply = await assistantEngine.processQuery(
      'find a focus block tomorrow',
      extractRouteContext('/calendar'),
      new Date(2026, 9, 15, 12, 0),
    );
    expect(reply.cards ?? []).toHaveLength(0);
    expect(reply.content).toContain('calendar did not answer');
  });
});

// ─── finding 3: pins only into an explicit draft ─────────────────────────────

describe('executeJourneyPin', () => {
  it('writes nothing without an explicit plan', async () => {
    mockFetch(() => json([{ id: 'plan:other', status: 'draft' }]));
    const result = await executeJourneyPin({ journey }, null);
    expect(result.ok).toBe(false);
    expect(calls).toHaveLength(0);
  });

  it('refuses a plan that is not a draft and writes nothing', async () => {
    mockFetch(() => json({}));
    const result = await executeJourneyPin({ journey }, plan('saved'));
    expect(result.ok).toBe(false);
    expect(calls).toHaveLength(0);
  });

  it('writes the transit result into the chosen draft and creates no plan', async () => {
    mockFetch(() => json({ id: 'item:1' }, 201));
    const result = await executeJourneyPin({ journey }, plan('draft'));
    expect(result.ok).toBe(true);
    expect(calls).toHaveLength(1);
    expect(calls[0].url).toBe('/trips/api/plans/plan%3A1/items');
    expect(calls.some((c) => c.method === 'POST' && c.url === '/trips/api/plans')).toBe(false);
    const body = calls[0].body as { item_type: string; day: string; payload: { journey: Journey } };
    expect(body.item_type).toBe('transport');
    expect(body.day).toBe('2026-10-16');
    expect(body.payload.journey).toEqual(journey);
  });

  it('propagates a trips failure', async () => {
    mockFetch(() => json({ error: 'plan not found' }, 404));
    const result = await executeJourneyPin({ journey }, plan('draft'));
    expect(result.ok).toBe(false);
    expect(result.message).toContain('plan not found');
  });
});

// ─── finding 2: no invented card when a backend is down ──────────────────────

describe('engine: travel', () => {
  const draft = (over: Partial<{ date_start: string | null; destinations: { id: string; name: string }[] }> = {}) => ({
    draft: {
      title: 'Berlin',
      origin: { id: 'place:frankfurt', name: 'Frankfurt' },
      destinations: [{ id: 'place:berlin', name: 'Berlin' }],
      date_start: '2026-10-16',
      date_end: null,
      interests: '',
      transport_modes: [],
      travelers: [],
      ...over,
    },
    unresolved: [],
    assumptions: [],
    source_text: 'train from Frankfurt to Berlin',
  });

  it('shows no card and names trips when every backend is down', async () => {
    mockFetch(backendDown);
    const reply = await assistantEngine.processQuery('train from Frankfurt to Berlin', extractRouteContext('/travel'));
    expect(reply.cards ?? []).toHaveLength(0);
    expect(reply.content).toContain('trips did not answer');
  });

  it('shows no card and names transit when the search fails', async () => {
    mockFetch((url) => (url.startsWith('/trips/') ? json(draft()) : new Response('', { status: 502 })));
    const reply = await assistantEngine.processQuery('train from Frankfurt to Berlin', extractRouteContext('/travel'));
    expect(reply.cards ?? []).toHaveLength(0);
    expect(reply.content).toContain('transit did not answer');
  });

  it('does not search, and says so, when the date is unresolved', async () => {
    mockFetch(() => json({ ...draft({ date_start: null }), unresolved: ['dates'] }));
    const reply = await assistantEngine.processQuery('train to Berlin', extractRouteContext('/travel'));
    expect(reply.cards ?? []).toHaveLength(0);
    expect(reply.content).toContain('(unresolved)');
    expect(calls.some((c) => c.url.startsWith('/api/search'))).toBe(false);
  });

  it('puts only transit results on cards, unchanged', async () => {
    mockFetch((url) => (url.startsWith('/trips/') ? json(draft()) : json([journey])));
    const reply = await assistantEngine.processQuery('train from Frankfurt to Berlin', extractRouteContext('/travel'));
    const search = calls.find((c) => c.url.startsWith('/api/search'));
    expect(search?.url).toBe('/api/search?from=Frankfurt&to=Berlin&time=2026-10-16T08%3A00%3A00');
    expect(reply.cards).toEqual([{ type: 'journey_option', data: { journey } }]);
  });

  it('starts transit before it searches, because transit is on-demand', async () => {
    mockFetch((url) => (url.startsWith('/trips/') ? json(draft()) : json([journey])));
    await assistantEngine.processQuery('train from Frankfurt to Berlin', extractRouteContext('/travel'));
    const start = calls.findIndex((c) => c.method === 'POST' && c.url.endsWith('/capabilities/transit/start'));
    const search = calls.findIndex((c) => c.url.startsWith('/api/search'));
    expect(start).toBeGreaterThanOrEqual(0);
    expect(start).toBeLessThan(search);
  });
});

describe('engine: interior and finance', () => {
  it('names interior when it does not answer and shows nothing invented', async () => {
    mockFetch(backendDown);
    const reply = await assistantEngine.processQuery('list my layouts', extractRouteContext('/interior'));
    expect(reply.cards ?? []).toHaveLength(0);
    expect(reply.content).toContain('interior did not answer');
  });

  it('says finance cannot answer yet and calls nothing', async () => {
    mockFetch(backendDown);
    const reply = await assistantEngine.processQuery('review my spending', extractRouteContext('/finance'));
    expect(reply.content).toContain('cannot answer finance questions yet');
    expect(calls).toHaveLength(0);
  });
});

// ─── finding 7: whole-word routing ───────────────────────────────────────────

describe('local keyword router', () => {
  const general = extractRouteContext('/');

  it('matches whole words only', () => {
    expect(routeByKeywords('what is the price', general).domain).toBe('general');
    expect(routeByKeywords('show recent items', general).domain).toBe('general');
    expect(routeByKeywords('the ICE to Berlin', general).domain).toBe('travel');
  });

  it('keeps the page domain when no cue matches', () => {
    expect(routeByKeywords('review the next item', extractRouteContext('/calendar')).domain).toBe('calendar');
  });

  it('matches multi-word cues on word boundaries', () => {
    expect(routeByKeywords('measure the living room', general).domain).toBe('interior');
  });
});

describe('generative UI widgets', () => {
  const general = extractRouteContext('/');

  it('emits action_choice card for general help query', async () => {
    const reply = await assistantEngine.processQuery('help me', general);
    expect(reply.cards).toBeDefined();
    expect(reply.cards?.[0]?.type).toBe('action_choice');
  });

  it('emits telemetry_pulse card for system health query', async () => {
    mockFetch((url) => {
      if (url.includes('sjel-status/health')) {
        return json({ ok: true, capabilities: { calendar: { up: true }, transit: { up: false } } });
      }
      return json({
        temp: { cpu_temp_avg: 48.5, gpu_temp_avg: 44.0 },
        memory: { ram_usage: 12000000000, ram_total: 34359738368 },
        all_power: 14.2,
      });
    });

    const reply = await assistantEngine.processQuery('check system status', general);
    expect(reply.cards).toBeDefined();
    expect(reply.cards?.[0]?.type).toBe('telemetry_pulse');
    if (reply.cards?.[0]?.type === 'telemetry_pulse') {
      expect(reply.cards[0].data.metrics.length).toBeGreaterThanOrEqual(3);
      expect(reply.cards[0].data.capabilitiesSummary?.up).toBe(1);
    }
  });

  it('emits feed_digest card for recent feed query', async () => {
    mockFetch(() => {
      return json([
        { id: '1', title: 'Generative UI in Practice', url: 'https://example.com/gen-ui', author: 'Research' },
      ]);
    });

    const reply = await assistantEngine.processQuery('read feed articles', general);
    expect(reply.cards).toBeDefined();
    expect(reply.cards?.[0]?.type).toBe('feed_digest');
  });
});

describe('assistant model ladder (ISC-18)', () => {
  const general = extractRouteContext('/');

  it('falls back to rules rung when backends are unreachable', async () => {
    mockFetch(() => {
      throw new Error('Connection refused');
    });

    const reply = await assistantEngine.processQuery('what is Sjel', general);
    expect(reply.rung).toBe('rules');
    expect(reply.content).toContain('Sjel Assistant');
  });

  it('uses the dedicated local assistant role before the Mac rung', async () => {
    mockFetch((url) => {
      if (url.includes('/assistant/ready')) {
        return json({ status: 'ready', model: 'qwen3:8b', max_input_bytes: 8192 });
      }
      if (url.includes('/assistant/api/generate')) {
        return json({ text: 'The local role answered.' });
      }
      return json({});
    });

    const reply = await assistantEngine.processQuery('what is Sjel', general);
    expect(reply.rung).toBe('local');
    expect(reply.content).toBe('The local role answered.');
  });

  it('uses mac rung when mac foundation model answers', async () => {
    mockFetch((url) => {
      if (url.includes('/foundation-models/health')) {
        return json({ model_available: true, context_window: 4096 });
      }
      if (url.includes('/foundation-models/v1/chat/completions')) {
        return json({
          choices: [
            { message: { content: 'Sjel is a local-first personal OS running capabilities on loopback.' } },
          ],
        });
      }
      return json({});
    });

    const reply = await assistantEngine.processQuery('what is Sjel', general);
    expect(reply.rung).toBe('mac');
    expect(reply.content).toBe('Sjel is a local-first personal OS running capabilities on loopback.');
  });
});
