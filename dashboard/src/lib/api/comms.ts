import type { CalendarCommitment } from './calendar';
import { jsonInit, request } from './core';

export type FeedStream = 'news' | 'media';

export type FeedKind =
  | 'youtube'
  | 'instagram'
  | 'podcast'
  | 'article'
  | 'mail'
  | 'github'
  | 'arxiv'
  | 'reddit';

export type FeedStatus = 'new' | 'keeper' | 'dismissed';

export interface FeedRelevance {
  profile_key: string;
  profile_label: string;
  score: number;
  rationale: string;
  mode: 'reranked' | 'semantic' | 'lexical';
  profile_revision: string;
}

export interface FeedEvaluationFactor {
  key: string;
  label: string;
  score: number;
  weight: number;
  rationale: string;
  context: {
    kind: 'trip' | string;
    id: string;
    label: string;
    date_start: string | null;
    date_end: string | null;
    matched_terms: string[];
  } | null;
}

export interface FeedEvaluation {
  overall_score: number;
  explanation: string;
  mode: 'reranked' | 'semantic' | 'lexical' | 'unscored';
  item_revision: string;
  context_revision: string;
  evaluator_revision: string;
  evaluated_at: string;
  factors: FeedEvaluationFactor[];
}

export interface CommsEvaluationStatus {
  evaluator_revision: string;
  context_revision: string;
  ledger: {
    evaluated: number;
    reranked: number;
    semantic: number;
    lexical: number;
    unscored: number;
  };
  summarizer: {
    // The unattended rung: the small local model every drain runs on. Named
    // here rather than the strong one because that is what an empty feed is a
    // question about.
    provider: string;
    model: string;
    configured: boolean;
    reachable: boolean;
    // The rung only an explicit press engages. Declared even though no
    // component reads it yet: a field on the wire that the interface does not
    // know about is how five provider fields went missing before (#C11).
    strong: {
      provider: string;
      model: string;
      configured: boolean;
      reachable: boolean;
    };
    // The durable half of the capacity alert, on the wire since the drains
    // started counting streaks and undeclared here until now.
    capacity: {
      alert_after: number;
      consecutive_aborts: number;
      alerting: boolean;
      last_abort_at: string | null;
      last_success_at: string | null;
    };
  };
  relevance: {
    provider: string;
    model: string;
    configured: boolean;
    reachable: boolean;
    profile_count: number;
    active_mode: 'reranked' | 'semantic' | 'lexical';
  };
  reranker: {
    provider: string;
    model: string;
    configured: boolean;
    reachable: boolean;
  };
  travel_context: {
    enabled: boolean;
    source: string;
    upcoming_count: number;
    reachable: boolean;
    from_cache: boolean;
    refreshed_at: string;
    plans: Array<{
      id: string;
      label: string;
      date_start: string;
      date_end: string;
    }>;
  };
}

export interface FeedOrigin {
  source_id: string;
  source_ref: string;
  label: string | null;
}

export interface FeedStageProvenance {
  stage: "extraction" | "normalization" | "summary" | "ranking";
  tier: "legacy" | "deterministic" | "model" | "human";
  revision: string;
  completed_at: string;
}

export interface FeedQualityFlag {
  feed_id: string;
  title: string | null;
  url: string;
  status: FeedStatus;
  content_status: "full" | "thin" | "none" | "unknown";
  signal:
    | "content_status"
    | "extraction_path"
    | "retention"
    | "boilerplate_leakage"
    | "summary_attempts"
    | "ranking_basis"
    | string;
  reason: string;
  evidence: string;
  derived_at: string;
}

export interface FeedQualityRefresh {
  reviewed: number;
  flagged_items: number;
  flag_count: number;
  bounded_to: number;
  days: number;
  provider_calls: 0;
}

interface FeedEntryBase {
  id: string;
  stream: FeedStream;
  kind: FeedKind;
  title: string | null;
  url: string;
  author: string | null;
  summary: string | null;
  /** The opening of this item's digest, sent only when it has no summary of its own.
   *
   *  Separate from `summary` because the two have different producers: the enrichment drain
   *  writes summaries on the light rung and has no cloud door, so an item past the 4,096-token
   *  window gets a digest and never a summary. Collapsing them server-side would make the card
   *  claim a summary nothing produced. */
  digest_preview: string | null;
  day: string; // YYYY-MM-DD grouping key
  created_at: string;
  status: FeedStatus;
}

export interface FeedEntry extends FeedEntryBase {
  /** What this item is worth protecting, as `GET /comms/feed` states it since 2026-09-06.
   *  An unclassified row answers `c1`, the undeclared default, so the LIST never omits it.
   *
   *  Optional all the same, and the `?` is the contract gap rather than caution: comms'
   *  `FeedFullItem` -- what `POST /ingest` and `GET /feed/:id` answer with -- carries no
   *  class, so `toListEntry` in `routes/feed/+page.svelte` builds a list row from a detail
   *  that has none. A reader must therefore treat `undefined` as "not stated" and fail
   *  closed, the way finance's `item_is_quotable` already does. It stops being optional the
   *  day the detail contract states one too.
   *
   *  On `FeedEntry` and not on `FeedEntryBase` for the same reason, and because the shared
   *  reader (`ContentItemDetail`) carries the richer `ContentDataClass` shape under this
   *  exact name -- one name for two shapes is how a component comes to read the wrong one. */
  data_class?: DataClass;
  relevance: FeedRelevance | null;
  evaluation: FeedEvaluation | null;
}

export interface FeedEntryDetail extends FeedEntryBase {
  transcript: string | null;
  relevance: FeedRelevance[];
  evaluation: FeedEvaluation | null;
  content_status: "full" | "thin" | "none" | "unknown";
  /** Which client handed the content over; null when the server fetched it. */
  captured_via: string | null;
  processing: FeedStageProvenance[];
  origins: FeedOrigin[];
}

export type ContentSource = 'feed' | 'mail' | 'calendar';

/**
 * Which capability owns each content source.
 *
 * The contract is shared; the data is not. A source is served by the capability
 * that stores it, and every one of them exposes the same `/content/:source/:id`
 * shape — so the reader resolves an item from its source alone and no caller
 * carries a per-capability special case. Adding a source is one line here.
 */
const CONTENT_BASE: Record<ContentSource, string> = {
  feed: '/comms',
  mail: '/comms',
  calendar: '/calendar/api',
};

/** Any content item, from whichever capability owns that source. */
export function contentItem(source: ContentSource, id: string, signal?: AbortSignal) {
  return request<ContentItemDetail>(
    `${CONTENT_BASE[source]}/content/${source}/${encodeURIComponent(id)}`,
    signal ? { signal } : undefined,
  );
}

export type DataClass = 'c0' | 'c1' | 'c2' | 'c3';

/** Who decided, ascending. `legacy` means nobody did — a row from before its
 * table had a class, or an item no collector declared anything about. */
export type ClassificationMethod = 'legacy' | 'deterministic' | 'model' | 'human';

export interface ContentDataClass {
  value: DataClass;
  label: 'Public' | 'Mine' | 'Others' | 'Secret';
  rationale: string;
  method: ClassificationMethod;
  version: string;
}

export interface ContentProcessingPolicy {
  local_processing: 'allowed' | 'blocked';
  cloud_handling: 'eligible' | 'pseudonymization_required' | 'blocked';
  pseudonymization_required: boolean;
  rationale: string;
}

export interface CloudProcessingState {
  status: 'not_prepared' | 'staged' | 'stale';
  preview_hash: string | null;
  approved_at: string | null;
  dispatch_status: 'not_queued' | 'queued' | 'running' | 'succeeded' | 'failed';
  job_id: string | null;
  provider_role: string | null;
  queued_at: string | null;
  provider_calls: number;
  task: 'content-analysis-v1' | null;
  started_at: string | null;
  completed_at: string | null;
  last_error: string | null;
  result: CloudContentAnalysis | null;
}

export interface CloudContentAnalysis {
  schema_version: 'cloud-content-analysis-v1';
  summary: string;
  importance: 'low' | 'medium' | 'high';
  importance_rationale: string;
  important_dates: Array<{ label: string; date: string | null; source_text: string }>;
  action_items: Array<{ text: string; due_date: string | null }>;
  topics: string[];
}

/// Why a configured cloud role cannot be picked right now. Kept as a union
/// rather than a bare string so a new server reason fails the build here
/// instead of rendering as an unlabelled token in the roster.
export type CloudProviderUnavailableReason =
  | 'missing_credential'
  | 'billing_expired_or_unknown'
  | 'budget_unavailable'
  | 'daily_request_limit_reached';

/// Every field `CloudProviderOut` serializes, in the order the server writes
/// them. The interface used to declare eight of the fifteen, so the budget,
/// the context ceiling, the credit expiry and the reason a role was refused
/// existed on the wire and nowhere in the UI.
export interface CloudProvider {
  role: string;
  name: string;
  model: string;
  provider_label: string;
  location: 'cloud';
  data_tier: 'public' | 'pseudonymized_personal';
  billing_mode: 'free_only' | 'prepaid_credit';
  failover_priority: number;
  max_requests_per_day: number;
  /// Null when the call ledger could not be read; that is a distinct state
  /// from zero calls made, and `budget_unavailable` is the reason it carries.
  requests_used_today: number | null;
  requests_remaining_today: number | null;
  max_input_tokens: number;
  credit_expires_on: string | null;
  available: boolean;
  unavailable_reason: CloudProviderUnavailableReason | null;
}

export interface RedactionFinding {
  entity_type: string;
  marker: string;
  count: number;
}

export interface CloudDerivativePreview {
  schema_version: 'cloud-derivative-preview-v1';
  source: ContentSource;
  id: string;
  source_revision: string;
  preview_hash: string;
  original_data_class: DataClass;
  derivative_data_class: 'c0' | 'c1';
  transformation: 'bounded-public-v1' | 'deterministic-entity-redaction-v3';
  document: string;
  redaction_count: number;
  redactions: RedactionFinding[];
  /** PRD Q9b's receipt: the sentence a human reads on a call that was reduced,
   *  or null when nothing was removed. Composed by comms, not here, so every
   *  surface says the same thing about the same call. */
  redaction_receipt: string | null;
  entity_detection: 'not-required' | 'local-deterministic-v3';
  truncated: boolean;
  approval_required: true;
  provider_calls: 0;
  limitations: string[];
}

export interface MailContentExtension {
  category: MailCategory;
  rationale: string;
  classification_method: ClassificationMethod;
  classification_version: string;
  gmail_action: 'archive' | 'trash' | 'restore' | null;
  gmail_action_at: string | null;
  purge_after: string | null;
  gmail_location: 'inbox' | 'archive' | 'trash' | 'missing' | null;
  gmail_observed_at: string | null;
  gmail_sync_status: 'synced' | 'queued' | 'retrying' | 'attention' | null;
  gmail_sync_action: 'archive' | 'trash' | 'restore' | null;
  gmail_sync_error: string | null;
  /** The doctrine's one state label, mirrored from Gmail. Separate from `status`
   *  on purpose: status is what Sjel decided about a proposal, waiting is what you
   *  decided about the conversation. */
  waiting: boolean;
  waiting_since: string | null;
}

/** Versioned reader contract shared by Feed sources and mail proposals. */
export interface ContentItemDetail {
  schema_version: 'content-item-v2';
  source: ContentSource;
  id: string;
  /** The source's own type discriminator, not a shared enum — a feed article is
   *  an `article`, a calendar entry is a `nightlife` or `work_onsite`. Open
   *  string, exactly as the schema types it: a source may add a kind without a
   *  dashboard release, and every reader already falls back for unknown ones. */
  kind: string;
  title: string | null;
  url: string;
  author: string | null;
  summary: string | null;
  content: string | null;
  content_label: string;
  day: string;
  created_at: string;
  /** Each source's triage axis, in one field: feed keeps or dismisses, mail
   *  moves through Gmail states, calendar commits. */
  status: FeedStatus | TriageStatus | CalendarCommitment;
  content_status: 'full' | 'thin' | 'none' | 'unknown';
  data_class: ContentDataClass;
  processing_policy: ContentProcessingPolicy;
  cloud_processing: CloudProcessingState;
  relevance: FeedRelevance[];
  evaluation: FeedEvaluation | null;
  processing: FeedStageProvenance[];
  origins: FeedOrigin[];
  links: ContentLink[];
  /** What the local model wrote about this item. Deliberately not `summary`,
   *  which is what the *source* said it is — calendar reads that from the
   *  entry's own description, and a generated paragraph written over it would
   *  destroy the only verbatim text an entry has. A reader wanting the short
   *  version prefers `digest.text` and falls back to `summary`. */
  digest: ContentDigest | null;
  mail: MailContentExtension | null;
  calendar: CalendarContentExtension | null;
}

/** Why there is no digest text. `skipped_short` is a verdict about the source —
 *  it is already shorter than any honest digest of it — not a failure. */
export type DigestState =
  | 'generated'
  | 'skipped_short'
  | 'remote_refused'
  /** The class enters no prompt at all — `c3`, or a class outside the
   *  vocabulary. Stricter than `remote_refused`, which is about where the
   *  endpoint is: no model was asked, local included. */
  | 'local_refused'
  | 'unconfigured'
  | 'http_error'
  | 'model_error'
  /** The server took the request and then ran out of room for it. A fact about
   *  the machine rather than the request, so it is worth retrying. */
  | 'capacity_aborted'
  | 'empty_response'
  | 'timeout';

/** The rung the length ladder landed on. Derived from `source_chars`, never
 *  chosen directly — see libs/summarize/README.md. */
export type DigestShape = 'none' | 'brief' | 'standard' | 'sectioned';

/** `detailed` moves the shape exactly one rung up that same ladder. It is not a
 *  separate instruction to the model, which is why it stays inspectable. */
export type DigestDepth = 'standard' | 'detailed';

export interface ContentDigest {
  text: string | null;
  state: DigestState;
  shape: DigestShape;
  depth: DigestDepth;
  /** The operator's focus terms, as typed. Shown back so a differently-shaped
   *  digest is explained rather than mysterious. */
  focus: string[];
  producer: string;
  source_chars: number;
  /** Entities the deterministic redactor removed before this text was stored.
   *  Non-zero only for Private content. */
  redactions: number;
  attempts: number;
  last_error: string | null;
  /** Mermaid source, validated against the known diagram headers before it was
   *  stored — a string the renderer cannot draw never gets here. */
  diagram: string | null;
  diagram_state: string | null;
  diagram_error: string | null;
  /** The table pulled out of the source, or null. Deliberately data rather than
   *  a chart specification: the reader compiles one, so a model never reaches
   *  the rendering layer. Every value appeared verbatim in the source text
   *  before it was admitted. */
  chart: ContentChartData | null;
  /** `generated`, or `skipped_short` when the source holds no comparable
   *  numbers — the answer for most prose, and not a failure. */
  chart_state: string | null;
  chart_error: string | null;
  generated_at: string;
}

/** One measure over a handful of categories. One series is the maximum by
 *  design: the figure palette is a low-chroma print palette that cannot carry
 *  categorical identity, so a chart drawn from it must not need to. */
export interface ContentChartData {
  title: string;
  category_label: string;
  measure_label: string;
  unit: string | null;
  /** Derived from the categories, never chosen by a model: an ordered run of
   *  three or more gets a line, everything else bars. */
  mark: "bar" | "line";
  note: string;
  rows: { category: string; value: number }[];
}

/** A named way out of an item: source page, the mail that carried the ticket,
 *  a map, a vault note. `kind` is a presentation hint and stays open — an
 *  unknown one still renders a working link. */
export interface ContentLink {
  label: string;
  kind: string;
  url: string;
}

/** What only a calendar entry has. No score on purpose: a decided item is not
 *  ranked, so `commitment` is its triage axis. */
export interface CalendarContentExtension {
  starts_at: string;
  /** Exclusive, as everywhere in calendar. */
  ends_at: string;
  all_day: boolean;
  commitment: 'possible' | 'planned' | 'committed';
  location: string | null;
  /** The operator's own note — why they care, not what the thing is. */
  notes: string | null;
  /** Which adapter contributed the row (`manual`, `luma`, `google`) — not the
   *  item's `source`, which is always `calendar`. Decides which actions are
   *  honest: an entry imported from Google must not offer to export back. */
  entry_source: string;
  /** Set when materialized from a rhythm. Such an instance is not exported
   *  individually, and any patch detaches it from its rhythm. */
  rhythm_id: string | null;
}

// One item's place in a collector run. Derived server-side per request, so a
// run never becomes stale state on the entry itself.
export interface FeedRun {
  feed_id: string;
  source_id: string;
  label: string | null;
  run_key: string;
  run_started: string | null;
}

export interface VaultLinkCandidate {
  id: string;
  source_id: string;
  source_ref: string;
  label: string | null;
  url: string;
  imported: boolean;
}

export interface FeedSource {
  id: string;
  adapter: 'github-trending' | 'arxiv' | string;
  enabled: boolean;
  source_url: string;
  query_configured: boolean;
  limit: number;
  last_run_at: string | null;
}

export interface FeedSourceScan {
  fetched: number;
  new_count: number;
  sources: Array<{
    source_id: string;
    adapter: string;
    fetched: number;
    new_count: number;
    known_count: number;
  }>;
}

export type MailCategory =
  | 'aktiv'
  | 'issue'
  | 'feed'
  | 'werbung'
  | 'belege'
  | 'steuern'
  | 'sonstiges';

export type TriageStatus =
  | 'proposed'
  | 'approved'
  | 'executed'
  | 'archived'
  | 'trashed'
  | 'missing'
  | 'dismissed';

export interface TriageItem {
  id: string;
  from_addr: string | null;
  subject: string | null;
  snippet: string | null;
  stream: MailCategory;
  rationale: string;
  classification_method: ClassificationMethod;
  classification_version: string;
  data_class: DataClass;
  data_class_rationale: string;
  data_classification_method: ClassificationMethod;
  data_classification_version: string;
  status: TriageStatus;
  gmail_action: 'archive' | 'trash' | 'restore' | null;
  gmail_action_at: string | null;
  purge_after: string | null;
  gmail_location: 'inbox' | 'archive' | 'trash' | 'missing' | null;
  gmail_observed_at: string | null;
  gmail_sync_status: 'synced' | 'queued' | 'retrying' | 'attention' | null;
  gmail_sync_action: 'archive' | 'trash' | 'restore' | null;
  gmail_sync_error: string | null;
  /** The doctrine's one state label, mirrored from Gmail. Separate from `status`
   *  on purpose: status is what Sjel decided about a proposal, waiting is what you
   *  decided about the conversation. */
  waiting: boolean;
  waiting_since: string | null;
  internal_date: string | null;
  relevance: FeedRelevance[];
  /** The local model rung's verdict, when a shadow pass has stored one. Null for
   *  a thread the deterministic rules decided, and for every thread until a pass
   *  has run. Declared here rather than in `lib/mail/api.ts` because TriageItem
   *  is the reader contract for GET /triage and this file owns it; the client
   *  FUNCTIONS all live in the stream's own module. */
  model?: TriageModelVerdict | null;
}

/** One stored verdict from the local mail classification rung.
 *
 *  `urgency_validated` is false until the frozen corpus carries a measured
 *  urgency band error. While it is false, urgency is shown and ranks nothing:
 *  a number that would reorder the ladder passes the same door the stream does. */
export interface TriageModelVerdict {
  mode: 'shadow' | 'applied' | 'held';
  /** `generated` when the model answered. `local_refused` when the class refuses
   *  every prompt, `skipped_over_window` when the source is too long for the
   *  light local model, and the transport states otherwise. */
  state: string;
  rule_stream: MailCategory;
  model_stream: MailCategory | null;
  confidence_bp: number | null;
  urgency_bp: number | null;
  urgency_validated: boolean;
  rationale: string | null;
  urgency_rationale: string | null;
  data_class: DataClass;
  /** Set when apply refused this proposal because it would raise the data class.
   *  Names the class it would raise to. */
  held_reason: string | null;
  classification_version: string;
  applied_at: string | null;
}

export interface TriageSweepResult {
  fetched: number;
  new_count: number;
  skipped: number;
  /** Threads whose subject or snippet was redacted before being stored. */
  redacted: number;
  events: { considered: number; proposals: number; no_event: number; refused: number; failed: number };
  total_stored: number;
  next_cursor: string | null;
  exhausted: boolean;
}

/** Freshness of the unattended inbox sweep. Counts, times and an error class;
 *  no mail reaches this shape. `last_success_at` is deliberately separate from
 *  `last_run_at` — a failing run still ran, and "when did collection last
 *  actually work" is the question a red schedule raises. */
export interface TriageSweepStatus {
  enabled: boolean;
  every_minutes: number;
  max_threads: number;
  quiet_hours: { start: number; end: number } | null;
  last_run_at: string | null;
  last_success_at: string | null;
  last_failure_at: string | null;
  last_error: 'auth' | 'quota' | 'network' | 'unknown' | null;
  considered_count: number;
  new_count: number;
  consecutive_failures: number;
}

/** What a redaction pass removed, by kind and count — never the values. */
export interface TriageRedactResult {
  reviewed: number;
  in_scope: number;
  changed: number;
  dry_run: boolean;
  entity_types: Record<string, number>;
  audit: { id: string; digest: string }[];
  transformation: string;
  provider_calls: number;
}

export interface TriageRelevanceResult {
  scored: number;
  profile_count: number;
  mode: 'reranked' | 'semantic' | 'lexical' | null;
  local_only: true;
}

export interface TriageDataClassRefreshResult {
  reviewed: number;
  updated: number;
  preserved_human: number;
  classifier_version: string;
  content_inputs: ['sender', 'subject', 'category'];
  provider_calls: 0;
}

export interface TriageBulkResult {
  succeeded: string[];
  failures: Array<{ id: string; error: string }>;
  gmail_changed: boolean;
  /** Rows whose stored subject and snippet this batch permanently redacted,
   *  because the category it set on them raises the data class. Zero for every
   *  action except `categorize` into `belege` or `steuern`. */
  narrowed: number;
}

export interface GmailMaintenanceResult {
  retried: number;
  recovered: number;
  retry_failures: number;
  reconciled: number;
  changed: number;
  read_failures: number;
  missing: number;
  content_fetched: false;
}

export const comms = {
  feed: (opts: { stream?: FeedStream; days?: number; includeDismissed?: boolean } = {}) => {
    const params = new URLSearchParams();
    if (opts.stream) params.set('stream', opts.stream);
    if (opts.days != null) params.set('days', String(opts.days));
    if (opts.includeDismissed) params.set('include_dismissed', 'true');
    const qs = params.toString();
    return request<FeedEntry[]>(`/comms/feed${qs ? `?${qs}` : ''}`);
  },
  entry: (id: string, signal?: AbortSignal) =>
    request<FeedEntryDetail>(
      `/comms/feed/${encodeURIComponent(id)}`,
      signal ? { signal } : undefined,
    ),
  /** Kept as the comms-shaped alias so existing comms callers read naturally;
   *  the routing itself lives in `contentItem` so there is one map, not two. */
  content: (source: ContentSource, id: string, signal?: AbortSignal) =>
    contentItem(source, id, signal),
  /** Generate or refine an item's digest. Synchronous on the server: this is a
   *  button the operator is watching, and answering early would show them the
   *  previous digest. */
  digest: (
    source: ContentSource,
    id: string,
    body: { depth?: DigestDepth; focus?: string[] } = {},
  ) =>
    request<ContentDigest>(
      `/comms/content/${source}/${encodeURIComponent(id)}/digest`,
      jsonInit('POST', body),
    ),
  chart: (source: ContentSource, id: string) =>
    request<ContentDigest>(
      `/comms/content/${source}/${encodeURIComponent(id)}/chart`,
      { method: 'POST' },
    ),
  diagram: (source: ContentSource, id: string) =>
    request<ContentDigest>(
      `/comms/content/${source}/${encodeURIComponent(id)}/diagram`,
      { method: 'POST' },
    ),
  refreshDigests: (source: ContentSource, limit?: number) =>
    request<{ source: string; digested: number }>(
      '/comms/content/digests/refresh',
      jsonInit('POST', limit === undefined ? { source } : { source, limit }),
    ),
  prepareCloudPreview: (source: ContentSource, id: string) =>
    request<CloudDerivativePreview>(
      `/comms/content/${source}/${encodeURIComponent(id)}/cloud-preview`,
      { method: 'POST' },
    ),
  approveCloudPreview: (source: ContentSource, id: string, preview_hash: string) =>
    request<CloudProcessingState>(
      `/comms/content/${source}/${encodeURIComponent(id)}/cloud-approval`,
      jsonInit('POST', { preview_hash }),
    ),
  cloudProviders: () => request<CloudProvider[]>('/comms/content/cloud-providers'),
  queueCloudDerivative: (
    source: ContentSource,
    id: string,
    preview_hash: string,
    provider_role: string,
  ) =>
    request<CloudProcessingState>(
      `/comms/content/${source}/${encodeURIComponent(id)}/cloud-queue`,
      jsonInit('POST', { preview_hash, provider_role }),
    ),
  runCloudJob: (jobId: string) =>
    request<CloudProcessingState>(
      `/comms/content/cloud-jobs/${encodeURIComponent(jobId)}/run`,
      jsonInit('POST', {}),
    ),
  runs: (days = 7) => request<FeedRun[]>(`/comms/feed/runs?days=${days}`),
  evaluationStatus: () =>
    request<CommsEvaluationStatus>('/comms/feed/evaluation/status'),
  qualityFlags: (limit = 500) =>
    request<FeedQualityFlag[]>(`/comms/feed/quality?limit=${limit}`),
  refreshQualityFlags: (days = 3650) =>
    request<FeedQualityRefresh>(
      '/comms/feed/quality/refresh',
      jsonInit('POST', { days }),
    ),
  // Answers once the item is stored; the summary is written behind the response, so a
  // freshly ingested entry legitimately comes back with `summary: null`.
  ingest: (url: string) => request<FeedEntryDetail>('/comms/ingest', jsonInit('POST', { url })),
  refreshRelevance: (days = 90) =>
    request<{
      scored: number;
      evaluated: number;
      considered: number;
      skipped_current: number;
      profile_count: number;
      mode: 'reranked' | 'semantic' | 'lexical' | null;
      evaluator_revision: string;
      travel_context: {
        upcoming_count: number;
        reachable: boolean;
        from_cache: boolean;
        refreshed_at: string;
      };
    }>(
      '/comms/feed/relevance/refresh',
      jsonInit('POST', { days }),
    ),
  scanVaultLinks: () =>
    request<VaultLinkCandidate[]>('/comms/vault-links/scan', { method: 'POST' }),
  sources: () => request<{ sources: FeedSource[] }>('/comms/sources'),
  scanSources: (source_id?: string) =>
    request<FeedSourceScan>(
      '/comms/sources/scan',
      jsonInit('POST', { source_id: source_id ?? null }),
    ),
  importVaultLink: (source_id: string, url: string) =>
    request<FeedEntryDetail>(
      '/comms/vault-links/import',
      jsonInit('POST', { source_id, url }),
    ),
  setStatus: (id: string, status: FeedStatus) =>
    request<void>(`/comms/feed/${encodeURIComponent(id)}/status`, jsonInit('POST', { status })),
  triage: (status?: TriageStatus) =>
    request<TriageItem[]>(
      `/comms/triage${status ? `?status=${encodeURIComponent(status)}` : ''}`,
    ),
  setTriageStatus: (id: string, status: 'proposed' | 'approved' | 'dismissed') =>
    request<void>(
      `/comms/triage/${encodeURIComponent(id)}/status`,
      jsonInit('POST', { status }),
    ),
  setTriageCategory: (id: string, stream: MailCategory) =>
    request<{ ok: boolean; narrowed: boolean }>(
      `/comms/triage/${encodeURIComponent(id)}/stream`,
      jsonInit('POST', { stream }),
    ),
  setTriageDataClass: (id: string, data_class: DataClass) =>
    request<void>(
      `/comms/triage/${encodeURIComponent(id)}/data-class`,
      jsonInit('POST', { data_class }),
    ),
  applyGmailAction: (id: string, action: 'archive' | 'trash' | 'restore') =>
    request<{ ok: true; action: 'archive' | 'trash' | 'restore'; gmail_changed: boolean; gmail_confirmed: true }>(
      `/comms/triage/${encodeURIComponent(id)}/gmail`,
      jsonInit('POST', { action }),
    ),
  decideGmailJob: (id: string, decision: 'retry' | 'cancel') =>
    request<{ ok: true; state: 'completed' | 'canceled'; gmail_changed?: boolean }>(
      `/comms/triage/${encodeURIComponent(id)}/gmail-job`,
      jsonInit('POST', { decision }),
    ),
  reconcileGmail: () =>
    request<GmailMaintenanceResult>('/comms/triage/reconcile', jsonInit('POST', {})),
  triageSweepStatus: (signal?: AbortSignal) =>
    request<TriageSweepStatus>('/comms/triage/sweep/status', signal ? { signal } : undefined),
  sweepTriage: (limit = 100, cursor?: string | null) =>
    request<TriageSweepResult>(
      '/comms/triage/sweep',
      jsonInit('POST', { limit, cursor: cursor ?? null }),
    ),
  refreshTriageRelevance: (limit = 200) =>
    request<TriageRelevanceResult>(
      '/comms/triage/relevance/refresh',
      jsonInit('POST', { limit }),
    ),
  refreshTriageDataClasses: (limit = 500) =>
    request<TriageDataClassRefreshResult>(
      '/comms/triage/data-class/refresh',
      jsonInit('POST', { limit }),
    ),
  /** Remediate rows stored before the sweep redacted Private mail in place.
   *  Idempotent: a second run reports `changed: 0`. */
  redactTriage: (limit = 500, dryRun = false) =>
    request<TriageRedactResult>(
      '/comms/triage/redact',
      jsonInit('POST', { limit, dry_run: dryRun }),
    ),
  bulkTriage: (
    ids: string[],
    action:
      | 'dismiss'
      | 'categorize'
      | 'set-data-class'
      | 'archive'
      | 'trash'
      // The doctrine's one state label. It only labels — it does not archive,
      // and archiving does not set it.
      | 'waiting'
      | 'clear-waiting',
    stream?: MailCategory,
    data_class?: DataClass,
  ) =>
    request<TriageBulkResult>(
      '/comms/triage/bulk',
      jsonInit('POST', { ids, action, stream: stream ?? null, data_class: data_class ?? null }),
    ),
};
