import type { DuplicateCandidate, Journey } from '$lib/api';
import type { Rung, Skip } from '$lib/intelligence/ladder';

export type IntentDomain = 'travel' | 'interior' | 'calendar' | 'finance' | 'system' | 'feed' | 'people' | 'general';

/** How the local keyword router picked a domain. In-page word matching, no model. */
export interface KeywordRouting {
  domain: IntentDomain;
  /** The whole-word cues that matched, for the tooltip. Empty when the route decided. */
  matched: string[];
  reason: string;
}

/** A connection exactly as transit's `/api/search` returned it. Nothing on this card is
 *  derived here except the display strings. */
export interface JourneyOptionCardData {
  journey: Journey;
  price_history?: Array<{ day: string; prices: number[] }>;
}

/** A proposed calendar block. `startsAt`/`endsAt` are naive local wall times
 *  ("YYYY-MM-DDTHH:MM:00"), the format `capabilities/calendar/src/date.rs::parse_instant`
 *  accepts; an offset or a trailing `Z` is a 400 there. */
export interface CalendarSlotCardData {
  title: string;
  startsAt: string;
  endsAt: string;
}

export interface TelemetryMetricItem {
  label: string;
  value: string;
  subvalue?: string;
  tone?: 'normal' | 'good' | 'warn' | 'alarm';
  percent?: number;
}

export interface TelemetryPulseWidgetData {
  title: string;
  subtitle?: string;
  overallOk: boolean;
  metrics: TelemetryMetricItem[];
  capabilitiesSummary?: {
    up: number;
    total: number;
  };
  actions?: Array<{
    label: string;
    route?: string;
    actionKey?: string;
    icon?: string;
  }>;
}

export interface FeedDigestItem {
  id?: string;
  title: string;
  url: string;
  author?: string;
  domain?: string;
  age?: string;
}

export interface FeedDigestWidgetData {
  title: string;
  items: FeedDigestItem[];
  moreCount?: number;
}

export interface SpatialRoomItem {
  name: string;
  areaSqMeters?: number;
  objectsCount?: number;
}

export interface SpatialSummaryWidgetData {
  title: string;
  roomsCount: number;
  furnitureCount: number;
  rooms: SpatialRoomItem[];
  modelUrl?: string;
}

export interface GenerativeMetric {
  label: string;
  value: string;
  change?: string;
  tone?: 'normal' | 'good' | 'warn' | 'alarm';
}

export interface GenerativeCustomCardData {
  title: string;
  subtitle?: string;
  tone?: 'good' | 'warn' | 'alarm' | 'primary' | 'neutral';
  kicker?: string;
  chips?: string[];
  metrics?: GenerativeMetric[];
  actions?: Array<{
    label: string;
    prompt?: string;
    route?: string;
    icon?: string;
  }>;
}

export interface ActionChoiceOption {
  id: string;
  label: string;
  prompt: string;
  icon?: string;
  description?: string;
}

export interface ActionChoiceWidgetData {
  title: string;
  description?: string;
  choices: ActionChoiceOption[];
}

export type ActionCard =
  | { type: 'journey_option'; data: JourneyOptionCardData }
  | { type: 'calendar_slot'; data: CalendarSlotCardData }
  | { type: 'telemetry_pulse'; data: TelemetryPulseWidgetData }
  | { type: 'feed_digest'; data: FeedDigestWidgetData }
  | { type: 'spatial_summary'; data: SpatialSummaryWidgetData }
  | { type: 'generative_card'; data: GenerativeCustomCardData }
  | { type: 'action_choice'; data: ActionChoiceWidgetData }
  | { type: 'merge_candidate'; data: DuplicateCandidate };

export interface AssistantMessage {
  id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  timestamp: string;
  routing?: KeywordRouting;
  cards?: ActionCard[];
  rung?: Rung;
  skippedRungs?: Skip[];
}

export interface ActionResult {
  ok: boolean;
  message: string;
}

export interface RouteContext {
  pathname: string;
  domain: IntentDomain;
  label: string;
  contextSummary: string;
  quickPrompts: string[];
}
