import { axonStatus, calendar, comms, entities, interior, macmon, transit, trips, type IntentDraft, type Journey } from '$lib/api';
import { addDays, findFreeSlots, localDate, unreadableEntries } from './calendar-slots';
import { matchedCues, routeByKeywords } from './keyword-router';
import type { ActionCard, AssistantMessage, IntentDomain, RouteContext, SpatialRoomItem, TelemetryMetricItem } from './types';

/**
 * Answers a drawer prompt from the capabilities on this machine, and from nothing else.
 *
 * Every number, time and name in a reply or on a card comes from a capability response.
 * A field the capability did not determine is shown as unresolved; a capability that did
 * not answer is named, and the reply stops there. There is no offline fallback with
 * sample data: the drawer says it cannot answer instead (operator ruling 2026-09-24).
 *
 * Nothing here leaves loopback: trips, transit, calendar and interior are local services.
 * The prompt goes to trips' intent parser as typed; privacy boundaries for anything that
 * does leave the machine live in Rust (`libs/pseudonymize`, PRD Q112), not in the page.
 */

interface Reply {
  content: string;
  cards?: ActionCard[];
}

/** Departure time sent to transit when the sentence names none. Stated in the reply. */
export const DEFAULT_DEPARTURE = '08:00';
/** The window a free block is searched in. Stated in the reply. */
export const FOCUS_WINDOW = { start: '09:00', end: '18:00' } as const;
/** Block length when the sentence names none. Stated in the reply. */
export const DEFAULT_BLOCK_MINUTES = 60;
const MAX_JOURNEY_CARDS = 3;

function reason(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

function didNotAnswer(capability: string, err: unknown, consequence: string): Reply {
  return { content: `${capability} did not answer: ${reason(err)}. ${consequence}` };
}

export class AssistantEngine {
  async processQuery(
    prompt: string,
    context: RouteContext,
    now: Date = new Date(),
  ): Promise<AssistantMessage> {
    const routing = routeByKeywords(prompt, context);
    const { content, cards } = await this.answer(prompt, routing.domain, now);
    return {
      id: `msg-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
      role: 'assistant',
      content,
      timestamp: new Date().toISOString(),
      routing,
      cards,
    };
  }

  private answer(prompt: string, domain: IntentDomain, now: Date): Promise<Reply> {
    switch (domain) {
      case 'travel':
        return this.travel(prompt);
      case 'calendar':
        return this.calendar(prompt, now);
      case 'interior':
        return this.interior();
      case 'finance':
        return Promise.resolve({
          content: 'The drawer cannot answer finance questions yet. Open Finance for balances and the review queue.',
          cards: [{
            type: 'generative_card',
            data: {
              title: 'Finance & Ledger',
              kicker: 'CAPABILITY OVERVIEW',
              tone: 'primary',
              subtitle: 'Bank accounts, balance reconciliation, and expense review queue.',
              chips: ['Multi-currency', 'Ledger', 'Review Queue'],
              actions: [
                { label: 'Open Finance', route: '/finance', icon: 'wallet' },
              ],
            },
          }],
        });
      case 'system':
        return this.system();
      case 'feed':
        return this.feed(prompt);
      case 'people':
        return this.people();
      default:
        return Promise.resolve({
          content: 'Sjel Assistant can interact with live capabilities across travel, calendar, systems, feed, and room interior.',
          cards: [{
            type: 'action_choice',
            data: {
              title: 'Suggested Quick Actions',
              description: 'Tap a prompt or ask your own question:',
              choices: [
                {
                  id: 'c1',
                  label: 'Check System Telemetry',
                  prompt: 'System health',
                  description: 'Live CPU, RAM, power, and capability health check',
                  icon: 'cpu',
                },
                {
                  id: 'c2',
                  label: 'Find Calendar Focus Slot',
                  prompt: 'Find a 2-hour focus block tomorrow',
                  description: 'Search free calendar blocks without overlapping commitments',
                  icon: 'calendar',
                },
                {
                  id: 'c3',
                  label: 'Explore Reading Feed',
                  prompt: 'Recent reading feed',
                  description: 'Recent articles ingested into comms',
                  icon: 'feed',
                },
                {
                  id: 'c4',
                  label: 'Inspect 3D Interior',
                  prompt: 'Show interior layout',
                  description: 'RoomPlan spatial rooms and furniture inventory',
                  icon: 'layout',
                },
              ],
            },
          }],
        });
    }
  }

  private async travel(prompt: string): Promise<Reply> {
    let drafted: IntentDraft;
    try {
      drafted = await trips.draftIntent(prompt);
    } catch (err) {
      return didNotAnswer('trips', err, 'No connection was searched.');
    }

    const draft = drafted.draft;
    const origin = draft.origin?.name ?? null;
    const destination = draft.destinations[0]?.name ?? null;
    const date = draft.date_start;
    const lines = [
      `trips read: from ${origin ?? '(not stated)'} to ${destination ?? '(unresolved)'} on ${date ?? '(unresolved)'}.`,
    ];
    if (drafted.unresolved.length > 0) lines.push(`Unresolved: ${drafted.unresolved.join(', ')}.`);
    if (drafted.assumptions.length > 0) lines.push(`trips assumed: ${drafted.assumptions.join('; ')}.`);

    if (!destination || !date) {
      lines.push('Not searched: a destination and a date are both needed.');
      return { content: lines.join('\n') };
    }
    const modes = draft.transport_modes;
    if (modes.length > 0 && !modes.includes('train')) {
      lines.push(`Not searched: transit searches rail only, and the sentence asked for ${modes.join(', ')}.`);
      return { content: lines.join('\n') };
    }

    // transit is on-demand. Start it before the search, as /travel/connections does on
    // mount (operator ruling 2026-09-24). A failed start is not fatal here: the search
    // below reports the real error.
    await axonStatus.start('transit').catch(() => undefined);

    let journeys: Journey[];
    try {
      journeys = await transit.search(origin, destination, `${date}T${DEFAULT_DEPARTURE}:00`);
    } catch (err) {
      lines.push(`transit did not answer: ${reason(err)}. No connection is shown.`);
      return { content: lines.join('\n') };
    }

    const when = `departures from ${DEFAULT_DEPARTURE} on ${date} (the sentence named no time)`;
    if (origin === null) lines.push("No origin stated: transit started from the profile's first home station.");
    if (journeys.length === 0) {
      lines.push(`transit found no connection for ${when}.`);
      return { content: lines.join('\n') };
    }
    const shown = journeys.slice(0, MAX_JOURNEY_CARDS);
    lines.push(
      `transit found ${journeys.length} connection${journeys.length === 1 ? '' : 's'} for ${when}; the first ${shown.length} follow.`,
    );
    return {
      content: lines.join('\n'),
      cards: shown.map((journey) => ({ type: 'journey_option', data: { journey } })),
    };
  }

  private async calendar(prompt: string, now: Date): Promise<Reply> {
    if (matchedCues(prompt, ['focus', 'slot', 'block', 'free', 'time']).length === 0) {
      return {
        content: 'The drawer can find a free block in the calendar ("find a 2-hour focus block tomorrow"). It cannot answer other calendar questions yet.',
      };
    }

    const today = localDate(now);
    const explicit = /\b(\d{4}-\d{2}-\d{2})\b/.exec(prompt)?.[1];
    const date = explicit
      ? explicit
      : matchedCues(prompt, ['today']).length > 0
        ? today
        : matchedCues(prompt, ['tomorrow']).length > 0
          ? addDays(today, 1)
          : null;
    if (!date) {
      return { content: 'Which day? Say "today", "tomorrow" or a date as YYYY-MM-DD. No slot was searched.' };
    }

    const hours = /(\d+(?:[.,]\d+)?)\s*-?\s*(?:hours?|h)\b/i.exec(prompt);
    const minutes = /(\d+)\s*-?\s*(?:minutes?|mins?)\b/i.exec(prompt);
    const stated = hours
      ? Math.round(Number(hours[1].replace(',', '.')) * 60)
      : minutes
        ? Number(minutes[1])
        : null;
    const duration = stated ?? DEFAULT_BLOCK_MINUTES;

    let entries;
    try {
      // `to` is exclusive in the calendar's day window
      // (`capabilities/calendar/src/store.rs::day_window_bounds`), so one day is [date, date+1).
      entries = await calendar.entries.list(date, addDays(date, 1));
    } catch (err) {
      return didNotAnswer('calendar', err, 'No slot was proposed.');
    }

    const lines: string[] = [];
    if (stated === null) lines.push(`No length stated; searched for ${duration} minutes.`);
    const unreadable = unreadableEntries(entries);
    if (unreadable.length > 0) {
      lines.push(
        `${unreadable.length} entr${unreadable.length === 1 ? 'y has' : 'ies have'} times the drawer cannot read and ${unreadable.length === 1 ? 'was' : 'were'} not checked: ${unreadable.map((e) => `"${e.title}"`).join(', ')}.`,
      );
    }

    const free = findFreeSlots(
      { date, windowStart: FOCUS_WINDOW.start, windowEnd: FOCUS_WINDOW.end, durationMinutes: duration },
      entries,
    );
    const window = `between ${FOCUS_WINDOW.start} and ${FOCUS_WINDOW.end} on ${date}`;
    if (free.length === 0) {
      lines.unshift(`No free ${duration}-minute block ${window}. The calendar holds ${entries.length} entr${entries.length === 1 ? 'y' : 'ies'} that day.`);
      return { content: lines.join('\n') };
    }
    lines.unshift(
      `Checked ${entries.length} calendar entr${entries.length === 1 ? 'y' : 'ies'} ${window}. Free ${duration}-minute block${free.length === 1 ? '' : 's'}:`,
    );
    return {
      content: lines.join('\n'),
      cards: free.map((slot) => ({
        type: 'calendar_slot',
        data: { title: 'Focus block', startsAt: slot.startsAt, endsAt: slot.endsAt },
      })),
    };
  }

  private async interior(): Promise<Reply> {
    let layouts: Array<{ name: string; occupied_m2: number; corridors: { from: string; to: string; width_cm: number | null }[] }>;
    try {
      layouts = await interior.layouts();
    } catch (err) {
      return didNotAnswer('interior', err, 'No layout is shown.');
    }

    if (layouts.length === 0) {
      return { content: 'interior holds no layouts yet. The drawer cannot evaluate or change layouts.' };
    }

    const rooms: SpatialRoomItem[] = layouts.map((l) => ({
      name: l.name,
      areaSqMeters: l.occupied_m2,
      objectsCount: l.corridors.length,
    }));

    const card: ActionCard = {
      type: 'spatial_summary',
      data: {
        title: 'RoomPlan Spatial Interior',
        roomsCount: layouts.length,
        furnitureCount: 0,
        rooms,
      },
    };

    return {
      content: `Interior holds ${layouts.length} layout${layouts.length === 1 ? '' : 's'}. Open Interior for 3D layout.`,
      cards: [card],
    };
  }

  /**
   * Probable duplicate people, as merge cards. Rules find the pairs; the on-device model is
   * asked only about first-name-only pairs that share a field, and only advises. Every
   * merge and every "not the same" is a tap (capabilities/entities/src/duplicates.rs).
   */
  private async people(): Promise<Reply> {
    try {
      const { total, candidates } = await entities.duplicates(10, true);
      if (total === 0) return { content: 'No probable duplicates among your people.' };
      const shown = candidates.length;
      return {
        content:
          `${total} possible duplicate pair${total === 1 ? '' : 's'}` +
          (shown < total ? `; here are the ${shown} strongest.` : '.') +
          ' Merging keeps the record with a note (or more details) and the longer name.',
        cards: candidates.map((data) => ({ type: 'merge_candidate' as const, data })),
      };
    } catch (err) {
      return didNotAnswer('entities', err, 'No duplicates are shown.');
    }
  }

  private async system(): Promise<Reply> {
    const [healthResult, macmonSample] = await Promise.allSettled([
      axonStatus.health(),
      macmon.json(),
    ]);

    const isHealthy = healthResult.status === 'fulfilled' && healthResult.value.ok;
    const caps = healthResult.status === 'fulfilled' ? Object.values(healthResult.value.capabilities ?? {}) : [];
    const upCount = caps.filter((c) => c.up).length;

    const metrics: TelemetryMetricItem[] = [];

    let cpuVal = '--';
    let gpuVal = '--';
    let ramVal = '--';
    let totalRamVal = '--';
    let powerVal = '--';

    if (macmonSample.status === 'fulfilled') {
      const m = macmonSample.value;
      const cpuTemp = m.temp?.cpu_temp_avg;
      const gpuTemp = m.temp?.gpu_temp_avg;
      if (cpuTemp != null) {
        cpuVal = `${cpuTemp.toFixed(0)}°C`;
        metrics.push({
          label: 'CPU Temp',
          value: cpuVal,
          tone: cpuTemp > 80 ? 'alarm' : cpuTemp > 65 ? 'warn' : 'good',
          percent: Math.min(100, (cpuTemp / 100) * 100),
        });
      }
      if (gpuTemp != null) {
        gpuVal = `${gpuTemp.toFixed(0)}°C`;
        metrics.push({
          label: 'GPU Temp',
          value: gpuVal,
          tone: gpuTemp > 80 ? 'alarm' : gpuTemp > 65 ? 'warn' : 'good',
          percent: Math.min(100, (gpuTemp / 100) * 100),
        });
      }
      if (m.memory?.ram_usage != null && m.memory?.ram_total != null) {
        const usedGb = m.memory.ram_usage / 1073741824;
        const totalGb = m.memory.ram_total / 1073741824;
        ramVal = `${usedGb.toFixed(1)} GB`;
        totalRamVal = `${totalGb.toFixed(0)} GB`;
        const ramPct = Math.round((usedGb / totalGb) * 100);
        metrics.push({
          label: 'RAM Usage',
          value: ramVal,
          subvalue: `of ${totalRamVal}`,
          tone: ramPct > 85 ? 'warn' : 'normal',
          percent: ramPct,
        });
      }
      if (m.all_power != null) {
        powerVal = `${m.all_power.toFixed(1)} W`;
        metrics.push({
          label: 'Total Power',
          value: powerVal,
          tone: 'normal',
        });
      }
    }

    const card: ActionCard = {
      type: 'telemetry_pulse',
      data: {
        title: 'System Telemetry & Status',
        subtitle: isHealthy ? 'All probed services reporting operational' : 'One or more services need attention',
        overallOk: isHealthy,
        capabilitiesSummary: caps.length > 0 ? { up: upCount, total: caps.length } : undefined,
        metrics,
        actions: [
          { label: 'Systems Dashboard', route: '/systems', icon: 'server' },
          { label: 'Capabilities Map', route: '/capabilities', icon: 'boxes' },
        ],
      },
    };

    const prose = isHealthy
      ? `All systems operational. ${upCount} of ${caps.length} capabilities running. CPU ${cpuVal}, RAM ${ramVal} / ${totalRamVal}, Power ${powerVal}.`
      : `System attention required. ${upCount} of ${caps.length} capabilities running.`;

    return {
      content: prose,
      cards: [card],
    };
  }

  private async feed(prompt: string): Promise<Reply> {
    const urlMatch = prompt.match(/https?:\/\/[^\s]+/i);
    if (urlMatch) {
      const url = urlMatch[0];
      try {
        const entry = await comms.ingest(url);
        return {
          content: `Ingested into Comms feed: ${entry.title ?? url}.`,
          cards: [{
            type: 'generative_card',
            data: {
              title: entry.title ?? url,
              kicker: 'INGESTED LINK',
              tone: 'good',
              subtitle: entry.author ? `by ${entry.author}` : undefined,
              chips: ['Saved to Comms', 'Reading List'],
              actions: [
                { label: 'Open Feed', route: '/feed', icon: 'feed' },
              ],
            },
          }],
        };
      } catch (err) {
        return didNotAnswer('comms', err, 'The URL was not ingested.');
      }
    }

    try {
      const entries = await comms.feed({ days: 7 });
      if (!entries || entries.length === 0) {
        return { content: 'Your feed has no recent items in the last 7 days.' };
      }

      const items = entries.slice(0, 3).map((e) => {
        let domain: string | undefined;
        try {
          domain = new URL(e.url).hostname.replace(/^www\./, '');
        } catch {
          domain = undefined;
        }
        return {
          id: e.id,
          title: e.title ?? e.url,
          url: e.url,
          author: e.author ?? undefined,
          domain,
        };
      });

      return {
        content: `Comms feed has ${entries.length} recent item${entries.length === 1 ? '' : 's'}:`,
        cards: [{
          type: 'feed_digest',
          data: {
            title: 'Recent Reading Feed',
            items,
            moreCount: entries.length > 3 ? entries.length - 3 : undefined,
          },
        }],
      };
    } catch (err) {
      return didNotAnswer('comms', err, 'Could not read feed.');
    }
  }
}

export const assistantEngine = new AssistantEngine();
