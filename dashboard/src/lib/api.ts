// Every capability client, one file each under ./api. This file keeps sjel-status.
export * from './api/core';
export * from './api/travel';
export * from './api/scouting';
export * from './api/inventory';
export * from './api/interior';
export * from './api/comms';
export * from './api/calendar';
export * from './api/vault';
export * from './api/entities';
export * from './api/finance';
export * from './api/places';
import { jsonInit, request } from './api/core';

export interface AxonStatusHealth {
  ok: boolean;
  version: string;
  uptime_seconds: number;
  capabilities: Record<string, { up: boolean; url: string }>;
}

export interface CapabilityView {
  name: string;
  kind: 'process' | 'container';
  scope: 'capability' | 'spine';
  port: string;
  panel_port: string;
  panel_path: string;
  autostart: string;
  requires: string[];
  /** Id kinds this capability answers `GET /api/links` for (libs/links/ISA.md D1). Optional
   *  because a sjel-status built before 2026-10-06 does not send it. */
  links_to?: string[];
  /** null when the capability declares no health surface — unknown, not down. */
  up: boolean | null;
  health_url: string | null;
}

/** One row of another capability that references a typed id (libs/links, `Link`). */
export interface Link {
  id: string;
  kind: string;
  title: string;
  at?: string;
  meta?: string;
  /** The field that holds the reference, such as `trip_id`. */
  via: string;
}

/** The body of `GET /<capability>/api/links?to=<id>`. */
export interface Links {
  to: string;
  links: Link[];
  /** Rows that reference the id but have no linkable id of their own. */
  unlinkable: number;
}

/** How entities matched a name (libs/links, `Match`). Only `exact` links without asking. */
export type NameMatch = 'exact' | 'first' | 'ambiguous' | 'none';

/** One undecided name a capability stores as text (libs/links, `OpenName`). */
export interface OpenName {
  name: string;
  /** How many of the capability's rows one decision for this name covers. */
  rows: number;
  /** Absent when entities did not answer; `OpenNames.error` says why. */
  status?: NameMatch;
  candidates: { id: string; name: string }[];
}

/** The body of `GET /<capability>/api/people/open`. */
export interface OpenNames {
  capability: string;
  names: OpenName[];
  error?: string;
}

export const people = {
  open: (capability: string) => request<OpenNames>(`/${encodeURIComponent(capability)}/api/people/open`),
  /** `entityId: null` records "not a person" for every row of that name in that capability. */
  decide: (capability: string, name: string, entityId: string | null) =>
    request<{ ok: boolean }>(`/${encodeURIComponent(capability)}/api/people/decide`, jsonInit('POST', { name, entity_id: entityId })),
};

export const links = {
  /** Every capability is proxied at `/<name>`, and `/api` passes even where `proxy_api_only`. */
  find: (capability: string, to: string) =>
    request<Links>(`/${encodeURIComponent(capability)}/api/links?to=${encodeURIComponent(to)}`),
};

/** A capability that serves its own UI (CONTRIBUTING.md#three-architectural-nouns) declares a panel port. */
export const hasPanel = (c: CapabilityView): boolean => c.panel_port !== '';

/**
 * Where a panel lives, as seen from THIS browser.
 *
 * Composed here rather than served by sjel-status, because a panel is loaded by a
 * browser and has to be reachable on the host that browser is already on. Serving
 * `127.0.0.1:<port>` to a shell opened at `localhost` makes the two different sites:
 * Chrome then partitions the frame's storage, and a framework whose client init touches
 * storage never hydrates, so the panel renders blank while every request returns 200.
 * Over Tailscale the same absolute address would point at the phone itself.
 */
export function panelUrl(c: CapabilityView): string {
  return `${location.protocol}//${location.hostname}:${c.panel_port}${c.panel_path}`;
}

/** One unit in the committed self-model. `service` exists only where a manifest does. */
export interface SelfUnit {
  name: string;
  kind: 'capability' | 'lib' | 'spine' | 'pack' | 'unknown';
  service?: { kind: string; port?: string; requires: string[]; image?: string };
  code?: { files: number; nodes: number };
}

/** Compile-time coupling: what is pulled into what. Not `requires`, which is runtime. */
export interface SelfCoupling {
  from: string;
  to: string;
  kinds: string[];
  evidence: string[];
}

export interface SelfModel {
  schema: number;
  generator: string;
  units: SelfUnit[];
  coupling: SelfCoupling[];
  upstreams: Array<{ name: string; verdict: string }>;
  /** Fused on read by the CLI where graphify-out/ exists; self.json, which this endpoint
   *  serves as-is, never carries it (tools/sjel-cli/src/self_model/mod.rs). */
  graph?: { present: boolean; nodes: number; external: number; stale: string[]; unmatched: string[] };
}

/**
 * The self-model as served: the committed artifact plus this machine's live state.
 *
 * Two keys, not one merged object, mirroring the endpoint. `model` is a fact about the
 * repo and is identical on every machine; `live` is a fact about this machine right now.
 * `up` is `null` where a capability declares nothing to poll — unknown, not down.
 */
export interface SelfModelResponse {
  model: SelfModel;
  live: Record<string, boolean | null>;
}

/**
 * Version identity for one of the two repos this installation is made of.
 *
 * Read-only by design: the shell shows which Sjel is running and links to it, and
 * deliberately cannot tag, commit or push — a browser button that writes to git needs a
 * gate in front of it, and there is no versioning scheme to write against yet.
 *
 * `tag` is null on a repo that has never been tagged, which today is both of them.
 * `describe` then degrades to a short sha, and that is the honest answer rather than a
 * missing one. `ahead`/`behind` are null when the branch tracks no upstream — not 0,
 * which would claim it is in sync with something.
 */
/** One operator-pinned link from the overlay's links.toml. Names and URLs only. */
export interface PinnedLink {
  name: string;
  url: string;
  note?: string;
}

export interface RepoStatus {
  name: string;
  role: 'spine' | 'overlay';
  remote_url: string | null;
  branch: string | null;
  describe: string | null;
  tag: string | null;
  commits_since_tag: number | null;
  ahead: number | null;
  behind: number | null;
  dirty: boolean;
  last_commit_date: string | null;
  error: string | null;
}

// `UpstreamEntry` / `UpstreamAudit` were here until 2026-08-28, typing the payload of
// `GET /sjel-status/upstreams` for the `/upstreams` page. Both are gone with that endpoint
// and that route: their `status` field ('ok' | 'na' | 'warn' | 'fail') was
// `tools/upstream-checker`'s verdict, and PRD Q41 retired the checker.
//
// The verdict half is not lost and never came from here: `SelfModel.upstreams` below
// carries `{name, verdict}` for every manifest entry, read from `upstreams.toml`
// by `tools/self generate`, and `/self` renders it.

/** How a capability's last backup stands against its own declared thresholds. */
export type BackupState =
  /** Has a backup contract and no receipt at all. Outranks every threshold. */
  | 'never'
  | 'ok'
  /** Past `advise_days`: you could take one. */
  | 'due'
  /** Past `stale_days`: you have a problem. */
  | 'overdue'
  /** The capability declares neither threshold, so nothing knows what timely means for
   *  its data — and Sjel will not invent a cadence to fill the gap. */
  | 'unknown';

/** An in-flight or finished run, as the server remembers it. Null when sjel-status has
 *  not been asked for a backup of this capability since it started. */
export interface BackupRun {
  state: 'running' | 'succeeded' | 'failed';
  started_at: number;
  finished_at?: number;
  detail?: string;
}

export interface BackupArchiveIdentity {
  name: string;
  bytes: number;
  sha256: string;
}

export interface BackupRunAttempt {
  id: number;
  capability: string;
  target: string;
  started_at: string;
  started_epoch: number;
  finished_at: string | null;
  finished_epoch: number | null;
  exit_code: number | null;
  archive: BackupArchiveIdentity | null;
  detail: string;
  log_path: string;
}

export interface BackupTargetView {
  id: string;
  kind: string;
  path: string;
  host: string;
  present: string; // "true" | "false" | "unchecked" | "unknown"
  declared_by: string[];
  seen_at: string;
  verified_at: string | null;
  verified_verdict: string | null; // "verified" | "failed" | "unchecked" | null
  verified_detail: string | null;
  interval_hours: number | null;
}

/**
 * One capability's backup standing.
 *
 * `attempt` carries the durable record of what the last run actually did, including failures
 * that left no fresh receipt behind.
 */
export interface BackupStatus {
  capability: string;
  state: BackupState;
  /** A run stops this capability while it takes a cold copy. Say so before confirming. */
  holds_service: boolean;
  advise_days: number | null;
  stale_days: number | null;
  last_success: string | null;
  age_seconds: number | null;
  bytes: number | null;
  contents: string | null;
  run: BackupRun | null;
  attempt?: BackupRunAttempt | null;
}

/** One thing wrong with this machine, as the hourly host watch found it.
 *
 *  It filed a task until PRD Q48 (2026-08-27). A runaway process is machine state, not
 *  an action a human wrote, so the findings became host-watch's own rows and the ladder
 *  ranks them directly. There is nothing to mark done: the next run closes a finding
 *  when the condition clears, which is the only honest close for a condition. */
export interface HostWatchFinding {
  id: string;
  /** The condition, e.g. `cpu:ApplicationsStorageExtension`. Names the command, never
   *  the pid, so a reboot does not look like a new problem. */
  key: string;
  title: string;
  /** What to run to look at it, and what to run if it is stuck. Multi-line. */
  note: string;
  first_seen: string;
  last_seen: string;
}

/** One reclaimable class in the storage report, as `tools/storage` names it. */
export interface StorageClass {
  name: string;
  bytes: number;
  /** Whether the overlay's policy lets `apply` reclaim it. `false` is report-only, and
   *  the tool never runs an empty reclaim command even here. */
  applicable: boolean;
  /** Over the policy's `class_flag_gb`. Loud on this machine, not necessarily wrong: a
   *  class can be over the flag with the disk nowhere near full. */
  flagged: boolean;
  /** False when `du` returned a partial or empty answer, so `bytes` is a lower bound rather
   *  than the directory's size. Absent from a report an older build produced, where a class
   *  that could not be read was skipped silently instead. */
  measured?: boolean;
  /** False when nothing on the machine answers to the class's paths — "Declared but
   *  absent" in the report's own words. Separates that from a path that is here and measures
   *  0 bytes, the pair `protected` gained on 2026-10-11. Absent from an older report. */
  present?: boolean;
}

/** A path the policy reports and never reclaims, with the reason it is safe to leave.
 *
 *  `present` is false when nothing on the machine answers to the path. The tool measures an
 *  entry rather than trusting it, so "this is gone" and "this measures 0 MB" stopped being
 *  the same number on 2026-10-11 — four of six entries were the former, each still carrying
 *  the reason written when the path existed. Absent from a report an older build produced. */
export interface StorageProtected {
  path: string;
  bytes: number;
  reason: string;
  present?: boolean;
}

/** `sjel storage report --json`, passed through verbatim.
 *
 *  Served by sjel-status rather than by `tools/storage`, which is operator machinery with
 *  no server — it measures, prints and exits. `state` is `ok`, `warn` or `critical`, and
 *  it is the volume's state, never a class being large: a class over the flag on a machine
 *  with free space is not a fault (the tool's own rule, tested in
 *  tools/sjel-cli/src/host_watch/pure.rs). */
/** What an agent may do on one capability (ISA F10). */
export type AgentMode = 'off' | 'read-only' | 'ask' | 'auto';

/** One write that waits for the owner, or was decided. */
export interface AgentApproval {
  id: string;
  capability: string;
  method: string;
  path: string;
  preview: string;
  created_at: number;
  state: 'pending' | 'allowed' | 'denied' | 'used';
}

/** One logged agent call. Never a body or a query. */
export interface AgentCall {
  at: number;
  capability: string;
  method: string;
  path: string;
  status: number;
  decision: string;
}

export interface AgentView {
  enrolled: boolean;
  modes: AgentMode[];
  default_mode: AgentMode;
  capabilities: {
    capability: string;
    mode: AgentMode;
    confirm: string[];
    get_writes: string[];
  }[];
  pending: AgentApproval[];
  calls: AgentCall[];
}

export interface StorageReport {
  disk: { used: number; free: number; total: number; target: string };
  state: string;
  classes: StorageClass[];
  protected: StorageProtected[];
  /** Each declared service with the machine's answer. `presence` is the tool's stable key
   *  (`running` · `absent` · `unverifiable`); `running` is the same answer as a boolean, and
   *  null when the probe could not run. Absent from a report an older build produced, which
   *  printed the declaration unchecked. */
  expected_service: Array<{
    kind: string;
    name: string;
    note: string;
    presence?: 'running' | 'absent' | 'unverifiable';
    running?: boolean | null;
    reason?: string | null;
  }>;
  /** `used` minus everything a class or a protection accounts for — the tool's ceiling on
   *  what the policy does not name. Absent from a report an older build produced. */
  named_bytes?: number;
  unattributed_bytes?: number;
}

/** One hourly reading of the volume, kept by sjel-status rather than by the tool. */
export interface StorageSample {
  /** The hour this reading belongs to, canonical UTC. One row per hour. */
  hour: string;
  /** When the reading was actually taken. */
  at: string;
  used: number;
  free: number;
  total: number;
  named: number;
  unattributed: number;
  reclaimable: number;
  state: string;
}

/** One name the machine no longer answers for, and since when.
 *
 * Written by `capabilities/sjel-status/src/storage_history.rs`'s hourly sampler, one row per
 * name per hour, and reduced there to the run that is still open — so this is "missing now",
 * not "was missing once". `absent` is absent from a response an older sjel-status produced. */
export interface StorageAbsent {
  /** Which of the policy's three lists the name came from. A class name, a protected path and
   *  a service name can be the same string, so the kind is what tells them apart. */
  kind: 'class' | 'protected' | 'service';
  name: string;
  /** The oldest hour of the still-open run, canonical UTC like `StorageSample.hour`. */
  since: string;
  /** True when the run reaches the oldest kept sample: the name was already gone when the
   *  record began, so `since` is a floor and not the hour it went. */
  capped: boolean;
}

/** The disk's history, oldest sample first. `samples` is empty on a machine that has not
 *  yet completed an hour under sjel-status — that is "no history", not an error. */
export interface StorageHistory {
  retention_days: number;
  samples: StorageSample[];
  /** Names missing in the newest sample and in every sample since. Empty on a machine whose
   *  sjel-status was built before the absence table existed — which is "not known", and is
   *  why the panel prints a bare name rather than a date it does not have. */
  absent?: StorageAbsent[];
}

/** Who owns moving one class of installed software. The tool's own vocabulary, verbatim:
 *  `scheduled` a capability moves it · `manual` a verb exists and nothing schedules it ·
 *  `unowned` nothing moves it · `self` the vendor does. */
export type UpdateOwner = 'scheduled' | 'manual' | 'unowned' | 'self';

/** current · stale · unknown · n/a. `unknown` is a claim the tool refuses to make rather
 *  than a soft "current" — a `--offline` report says so on every row it did not check. */
export type UpdateStatus = 'current' | 'stale' | 'unknown' | 'n/a';

/** One class of installed software and who moves it, as `tools/sjel-cli/src/updates/report.rs`'s `SURFACES`
 *  declares it. Sent with the report so the panel renders the tool's reasoning rather
 *  than a second copy of the ownership table. */
export interface UpdateSurface {
  id: string;
  title: string;
  owner: UpdateOwner;
  ownerDetail: string;
  /** Whether `apply` moves this class. A `self` class is never actionable: that would be
   *  a second updater for a binary that already has one. */
  actionable: boolean;
  why: string;
}

/** One row: a package, a crate, an integration or a class's own receipt line. */
export interface UpdateRow {
  surface: string;
  name: string;
  owner: UpdateOwner;
  ownerDetail: string;
  installed?: string;
  latest?: string;
  status: UpdateStatus;
  /** The command that moves it. Present only on rows the tool can act on. */
  action?: string;
  note?: string;
}

/** What `apply` recorded, written by the tool and carried on every report.
 *
 *  This is how the panel learns an outcome it did not wait for: the apply route answers
 *  `202` before the work starts, because a cargo step compiles for minutes and a held
 *  request would time out while the install succeeded. */
export interface UpdateApply {
  at?: string;
  class?: string;
  steps?: number;
  failed?: number;
  stillStale?: number;
  state?: 'running' | 'done' | 'failed';
  /** tools/audit's verdict, taken immediately after the apply: `clean` · `finding(s)` ·
   *  `scanner-missing` · `could not run`. A plain string rather than a union on purpose —
   *  a fifth verdict the tool grows must be shown, not dropped. Absent on a receipt written
   *  before the field existed. */
  audit?: string;
}

/** `sjel update --json`, passed through verbatim.
 *
 *  Served by sjel-status rather than by `tools/updates`, which is operator machinery with
 *  no server — the same arrangement `storage` above has, and for the same reason. A
 *  non-zero exit is not an error: `report` exits 1 whenever anything is stale, which is
 *  the answer the panel exists to show. */
export interface UpdatesReport {
  generatedAt: string;
  offline: boolean;
  lastApply: UpdateApply | null;
  surfaces: UpdateSurface[];
  rows: UpdateRow[];
}

/** One Pack skill (or the agents/ tree) as one harness currently holds it. */
export interface PackUnitView {
  pack: string;
  skill: string;
  /** current · outdated · drifted · missing · collision · invalid · not-deployed · migration-required */
  status: string;
  detail?: string;
}

/** A directory at a harness skill root that no Pack ledger claims. */
export interface PackStrayView {
  name: string;
  /** `copy` is a promote candidate; `external` and `symlink` are owned by another installer. */
  kind: "copy" | "external" | "symlink";
  detail?: string;
}

export interface HarnessView {
  id: string;
  label: string;
  installed: boolean;
  /** `materialized` copies and can drift; `registry` reads the Pack source in place. */
  model: "materialized" | "registry";
  marker: string;
  destination: string | null;
  cli: string;
  units: PackUnitView[];
  unowned: PackStrayView[];
}

export interface PacksView {
  measuredAt: string;
  harnesses: HarnessView[];
  unsupported: { id: string; label: string; why: string }[];
}

export const axonStatus = {
  health: () => request<AxonStatusHealth>('/sjel-status/api/sjel-status/health'),
  capabilities: () => request<CapabilityView[]>('/sjel-status/api/sjel-status/capabilities'),
  backups: () =>
    request<{ backups: BackupStatus[]; attempt_error: string | null }>(
      '/sjel-status/api/sjel-status/backups',
    ),
  /** Accepts the run and returns — it does not wait for it. Poll `backups()` for the
   *  outcome, which is also what lets a slow run survive a page refresh. */
  backup: (name: string, target?: string) =>
    request<{ name: string; accepted: boolean; target?: string; run_id?: number; holds_service: boolean }>(
      `/sjel-status/api/sjel-status/capabilities/${encodeURIComponent(name)}/backup`,
      {
        method: 'POST',
        ...(target
          ? {
              body: JSON.stringify({ target }),
              headers: { 'Content-Type': 'application/json' },
            }
          : {}),
      },
    ),
  backupTargets: () =>
    request<{ targets: BackupTargetView[] }>('/sjel-status/api/sjel-status/backup/targets'),
  backupRuns: (limit = 50) =>
    request<{ runs: BackupRunAttempt[]; running: number }>(
      `/sjel-status/api/sjel-status/backup/runs?limit=${limit}`,
    ),
  setBackupPolicy: (target: string, interval_hours: number | null) =>
    request<{ target: BackupTargetView | null }>('/sjel-status/api/sjel-status/backup/policy', {
      method: 'POST',
      body: JSON.stringify({ target, interval_hours }),
      headers: { 'Content-Type': 'application/json' },
    }),
  verifyBackupTarget: (target: string) =>
    request<{ target: string; verdict: string; detail: string }>(
      '/sjel-status/api/sjel-status/backup/verify',
      {
        method: 'POST',
        body: JSON.stringify({ target }),
        headers: { 'Content-Type': 'application/json' },
      },
    ),
  self: () => request<SelfModelResponse>('/sjel-status/api/sjel-status/self'),
  repos: () => request<{ repos: RepoStatus[] }>('/sjel-status/api/sjel-status/repos'),
  links: () => request<{ links: PinnedLink[] }>('/sjel-status/api/sjel-status/links'),
  /** Open findings from the hourly host watch. Served here rather than by host-watch
   *  itself because that capability is a scheduled job with no port: it runs, writes to
   *  its own table, and exits. Same shape as `backups()`, which publishes a job's
   *  receipts for the same reason. */
  hostWatch: (signal?: AbortSignal) =>
    request<{ findings: HostWatchFinding[] }>(
      '/sjel-status/api/sjel-status/host-watch',
      signal ? { signal } : undefined,
    ).then((response) => response.findings),
  /** What fills the disk, from the tool's own `report --json`. Served here because
   *  `tools/storage` is operator machinery with no server — it measures, prints and exits
   *  — for the same reason `hostWatch()` above is. A non-zero exit is not an error: it is
   *  how `report` says free space is below the policy's critical threshold. */
  storage: (signal?: AbortSignal) =>
    request<StorageReport>('/sjel-status/api/sjel-status/storage', signal ? { signal } : undefined),
  /** The disk's own history, one sample an hour, so a trend has something to draw. Kept by
   *  sjel-status on its own timer rather than by `tools/storage`, which is a point
   *  measurement with no memory; the sampler is the only writer of the table this reads. */
  storageHistory: (signal?: AbortSignal) =>
    request<StorageHistory>(
      '/sjel-status/api/sjel-status/storage-history',
      signal ? { signal } : undefined,
    ),
  /** Software installed outside this checkout, from the tool's own `report --json`.
   *
   *  A non-zero exit is not an error here either: `report` exits 1 whenever anything is
   *  stale, which is exactly the answer this call exists to fetch. */
  updates: (signal?: AbortSignal) =>
    request<UpdatesReport>('/sjel-status/api/sjel-status/updates', signal ? { signal } : undefined),
  /** Start one class of software moving.
   *
   *  `202` means STARTED, not finished: the handler spawns the tool detached because a
   *  cargo step compiles for minutes, and the outcome arrives on the next `updates()` call
   *  as `lastApply`. Polling the report is the whole progress mechanism — there is no
   *  second endpoint and no state held in sjel-status. */
  updatesApply: (className: string) =>
    request<{ started: boolean; class: string }>('/sjel-status/api/sjel-status/updates/apply', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ class: className }),
    }),
  /** The agent's reach (ISA F10): each capability's mode, writes waiting, latest calls. */
  agent: (signal?: AbortSignal) =>
    request<AgentView>('/sjel-status/api/sjel-status/agent', signal ? { signal } : undefined),
  setAgentMode: (capability: string, mode: AgentMode) =>
    request<{ capability: string; mode: AgentMode }>('/sjel-status/api/sjel-status/agent/mode', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ capability, mode }),
    }),
  decideAgentWrite: (id: string, allow: boolean) =>
    request<AgentApproval>(`/sjel-status/api/sjel-status/agent/approvals/${encodeURIComponent(id)}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ allow }),
    }),
  /** Every Pack skill against every agent harness. Served here rather than by `packs`
   *  itself because that capability is `kind = "data"`: it owns the deployment ledgers and
   *  nothing starts, so it has no port. Same reason as `hostWatch()` above. */
  packs: (signal?: AbortSignal) =>
    request<PacksView>('/sjel-status/api/sjel-status/packs', signal ? { signal } : undefined),
  /** Append a `watch` row to upstreams.toml. A 400 carries the tool's one-line refusal. */
  watchUpstream: (url: string, summary: string, name?: string) =>
    request<{ name: string; url: string; verdict: string; license: string; summary: string }>(
      '/sjel-status/api/sjel-status/upstreams/watch',
      {
        method: 'POST',
        body: JSON.stringify({ url, summary, ...(name ? { name } : {}) }),
        headers: { 'Content-Type': 'application/json' },
      },
    ),
  start: (name: string, signal?: AbortSignal) =>
    request<{ name: string; up: boolean; detail: string }>(
      `/sjel-status/api/sjel-status/capabilities/${encodeURIComponent(name)}/start`,
      { method: 'POST', signal },
    ),
  stop: (name: string) =>
    request<{ name: string; up: boolean; detail: string }>(
      `/sjel-status/api/sjel-status/capabilities/${encodeURIComponent(name)}/stop`,
      { method: 'POST' },
    ),
};

/** One graphify node inside a unit: a file or a symbol the extractor found. */
export interface UnitGraphNode {
  id: string;
  label: string;
  source_file: string;
  file_type: string;
  community: number;
}

/**
 * One unit's slice of the graphify graph, capped by the server.
 *
 * `total` and `truncated` are the honest part of the contract: the full graph
 * is thousands of nodes, a drill-down returns the busiest few hundred, and a
 * reader has to be able to tell a complete unit from a sampled one.
 */
export interface UnitGraph {
  unit: string;
  prefixes: string[];
  total: number;
  returned: number;
  truncated: boolean;
  cap: number;
  nodes: UnitGraphNode[];
  edges: Array<{ from: string; to: string; label?: string }>;
}

export const knowledgeGraph = {
  unit: (name: string) =>
    request<UnitGraph>(`/knowledge-graph/api/graph/unit/${encodeURIComponent(name)}`),
};

/** One sample from macmon pipe/serve. Every field present on Apple Silicon. */
export interface MacmonSample {
  all_power: number;
  ane_power: number;
  cpu_power: number;
  cpu_usage_pct: number;
  ecpu_usage: [number, number];  // [frequency_mhz, utilisation_0_1]
  gpu_power: number;
  gpu_ram_power: number;
  gpu_usage: [number, number];   // [frequency_mhz, utilisation_0_1]
  memory: {
    ram_total: number;
    ram_usage: number;
    swap_total: number;
    swap_usage: number;
  };
  pcpu_usage: [number, number];  // [frequency_mhz, utilisation_0_1]
  ram_power: number;
  sys_power: number;
  temp: {
    cpu_temp_avg: number;
    gpu_temp_avg: number;
  };
  timestamp: string;
}

// macmon — sudoless Apple Silicon performance monitor. Not an Sjel capability.
// Proxied at /macmon → http://localhost:9911 (see vite.config.ts).
export const macmon = {
  json: () => request<MacmonSample>('/macmon/json'),
};
