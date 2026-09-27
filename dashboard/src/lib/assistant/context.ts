import type { RouteContext } from './types';

/**
 * The domain the drawer biases towards on the current page (PRD §8.1, Q111: the drawer
 * reads the path so the operator does not have to restate it).
 *
 * `quickPrompts` lists only questions the engine answers from a capability. A chip that
 * leads to "cannot answer yet" is a promise the drawer does not keep.
 */
export function extractRouteContext(pathname: string): RouteContext {
  if (pathname.startsWith('/travel')) {
    return {
      pathname,
      domain: 'travel',
      label: pathname.includes('/connections') ? 'Travel · Connections' : 'Travel · Plans',
      contextSummary: 'Travel: connection search through trips (intent) and transit (search).',
      quickPrompts: ['Train from Frankfurt to Berlin tomorrow'],
    };
  }

  if (pathname.startsWith('/interior')) {
    return {
      pathname,
      domain: 'interior',
      label: 'Interior',
      contextSummary: 'Interior: lists the stored layouts and their check results.',
      quickPrompts: ['List my layouts'],
    };
  }

  if (pathname.startsWith('/calendar')) {
    return {
      pathname,
      domain: 'calendar',
      label: 'Calendar',
      contextSummary: 'Calendar: finds a free block against the stored entries.',
      quickPrompts: ['Find a 2-hour focus block tomorrow'],
    };
  }

  if (pathname.startsWith('/finance')) {
    return {
      pathname,
      domain: 'finance',
      label: 'Finance',
      contextSummary: 'Finance: the drawer cannot answer finance questions yet.',
      quickPrompts: [],
    };
  }

  if (pathname.startsWith('/systems')) {
    return {
      pathname,
      domain: 'system',
      label: 'Systems',
      contextSummary: 'Systems: health and telemetry from axon-status and macmon.',
      quickPrompts: ['System health', 'Hardware status'],
    };
  }

  if (pathname.startsWith('/people')) {
    return {
      pathname,
      domain: 'people',
      label: 'People',
      contextSummary: 'People: capabilities/entities, synced from Obsidian and Google Contacts.',
      quickPrompts: ['Find duplicates'],
    };
  }

  if (pathname.startsWith('/feed')) {
    return {
      pathname,
      domain: 'feed',
      label: 'Feed',
      contextSummary: 'Feed: reading and link ingestion through comms.',
      quickPrompts: ['Recent unread feed'],
    };
  }

  return {
    pathname,
    domain: 'general',
    label: 'Sjel',
    contextSummary: 'No capability bias on this page.',
    quickPrompts: ['Find a 2-hour focus block tomorrow', 'Train from Frankfurt to Berlin tomorrow', 'System health'],
  };
}
