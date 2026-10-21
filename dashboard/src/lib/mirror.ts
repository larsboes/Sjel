/**
 * The device's copy of the Mac's data, for when the Mac is not reachable.
 *
 * `src-tauri/src/mirror.rs` states the design; this is the typed seam the pages use. Nothing here
 * renders anything yet — the copy is fetched, verified and read, and no route reads it in place of
 * the Mac, which is deliberate: what the copy holds is the Mac's decision (`PHONE_SET` in
 * `tools/sjel-cli/src/mirror.rs`), and a page that switched source would hide that.
 *
 * Two things this is not:
 *
 * - not the snapshot cache (`./mac-bridge.ts`, `src-tauri/src/sync.rs`), which keeps the last answer
 *   to each GET the phone has already made, keyed by URL. That reopens visited pages; this answers
 *   questions the phone never asked.
 * - not a write path. The copy is opened read-only, and an edit made offline still goes to the
 *   outbox and needs the Mac.
 */
import { invoke } from '@tauri-apps/api/core';

/** One table's contribution, as the Mac's manifest reports it. */
export interface MirrorTable {
  name: string;
  rows: number;
}

/** What the copy holds, for a status line. Mirrors `CopyStatus` in `src-tauri/src/mirror.rs`. */
export interface MirrorCopy {
  /** The Mac's own RFC 3339 build time. */
  built_at: string;
  /** That time as unix milliseconds, `null` when it does not parse. */
  built_at_ms: number | null;
  age_ms: number | null;
  /** True when older than `REBUILD_AFTER_MS`, so the next sync rebuilds on the Mac first. */
  stale: boolean;
  fetched_at_ms: number;
  bytes: number;
  rows: number;
  tables: MirrorTable[];
  sha256: string;
  /** False is not a slow copy: the file on disk does not match its manifest and is not readable. */
  verified: boolean;
  integrity: string;
  foreign_key_violations: number;
}

export interface MirrorStatus {
  copy: MirrorCopy | null;
  /** Set when a copy exists and cannot be trusted; empty when there is simply none yet. */
  problem: string | null;
}

/** What one sync did. Mirrors `SyncOutcome` in `src-tauri/src/mirror.rs`. */
export interface MirrorOutcome {
  action: 'unchanged' | 'fetched' | 'rebuilt and fetched';
  /** Bytes that crossed the network for the copy itself. Zero for `unchanged`. */
  transferred: number;
  /** Why a rebuild that was due did not happen, when it did not. The fetch still ran. */
  rebuild_error: string | null;
  status: MirrorStatus;
}

/**
 * One record from the copy. Reduced on purpose: `/projects/api/tasks` also answers `uri`,
 * `projects`, `data_class` and `fields`, which it reads from vault frontmatter the copy does not
 * carry. Read those from the Mac, or show them as absent.
 */
export interface MirrorRecord {
  path: string;
  project: string;
  kind: 'project' | 'task' | 'note';
  title: string;
  done: boolean;
  due: string | null;
  priority: number;
  archived: boolean;
  content: string;
}

export interface MirrorProject {
  name: string;
  archived: boolean;
  open: number;
  done: number;
  notes: number;
}

/** How old a copy may be before `mirrorSync` rebuilds it on the Mac first. */
export const REBUILD_AFTER_MS = 12 * 60 * 60 * 1000;

/** What this device holds, without going near the network. */
export function mirrorStatus(): Promise<MirrorStatus> {
  return invoke<MirrorStatus>('mirror_status');
}

/**
 * Fetch the copy. `rebuild` (default true) asks the Mac to build a fresh one first when what is
 * stored is older than `REBUILD_AFTER_MS`, or older than the device can tell. Rejects with the
 * native error string; whatever was stored before is untouched.
 */
export function mirrorSync(rebuild = true): Promise<MirrorOutcome> {
  return invoke<MirrorOutcome>('mirror_sync', { rebuild });
}

/** Open tasks, or done ones, ordered by due date then priority. The Mac is never consulted. */
export function mirrorTasks(status?: 'open' | 'done'): Promise<MirrorRecord[]> {
  return invoke<MirrorRecord[]>('mirror_tasks', { status: status ?? null });
}

/** Every record of one kind, ordered by path — the copy of `/projects/api/records?kind=`. */
export function mirrorRecords(kind: 'project' | 'task' | 'note'): Promise<MirrorRecord[]> {
  return invoke<MirrorRecord[]>('mirror_records', { kind });
}

/** Every project the copy knows, counted like `capabilities/projects` counts it. */
export function mirrorProjects(): Promise<MirrorProject[]> {
  return invoke<MirrorProject[]>('mirror_projects');
}

/** One project's records, ordered by path. */
export function mirrorProjectRecords(name: string): Promise<MirrorRecord[]> {
  return invoke<MirrorRecord[]>('mirror_project_records', { name });
}
