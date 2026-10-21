/**
 * Reading the projects page from the device's copy when the Mac cannot answer.
 *
 * `src-tauri/src/mirror.rs` fetches and checks the copy; the rules here decide when a page may use
 * it, and they are separated from the component so they can be tested without a Tauri host
 * (`vite/offline-copy.test.ts`, the same fake-invoke pattern as `bridged-url.test.ts`).
 *
 * Three rules, and each exists because of a way the page could otherwise lie:
 *
 * 1. **Only a verified copy is read.** `status.copy.verified` is false when the file on disk stopped
 *    matching its manifest, and an unverified copy is exactly the "partial database that looks
 *    complete" this feature exists to refuse.
 * 2. **An empty answer is still an answer.** The copy may simply not hold the project that was
 *    asked for, and that is different from the Mac being unreachable: the page shows "not in the
 *    copy", not a network error.
 * 3. **A row keeps only what the copy holds.** No `data_class`, no frontmatter map, no `uri` — those
 *    are read from the vault by the Mac, and filling them in here would make the copy claim more
 *    than it has.
 */
import type { ProjectRecord, ProjectSummary } from '$lib/api';
import { ageLabel } from '$lib/ui/age';
import { inTauri } from '$lib/mac-bridge';
import {
  mirrorProjectRecords,
  mirrorProjects,
  mirrorRecords,
  mirrorStatus,
  type MirrorCopy,
  type MirrorRecord,
} from '$lib/mirror';

/** The copy's commands, injectable so the rules below run against a fake in a test. */
export interface CopySource {
  status(): Promise<{ copy: MirrorCopy | null }>;
  projects(): Promise<ProjectSummary[]>;
  notes(): Promise<MirrorRecord[]>;
  projectRecords(name: string): Promise<MirrorRecord[]>;
}

export const nativeCopy: CopySource = {
  status: mirrorStatus,
  projects: mirrorProjects,
  notes: () => mirrorRecords('note'),
  projectRecords: mirrorProjectRecords,
};

/** The copy this device holds, if it holds one that verifies. Null outside the app, when there is
 *  none, and when reading it failed — all three take the ordinary "the Mac did not answer" path. */
export async function readableCopy(source: CopySource = nativeCopy): Promise<MirrorCopy | null> {
  if (!inTauri()) return null;
  try {
    const status = await source.status();
    return status.copy?.verified ? status.copy : null;
  } catch {
    return null;
  }
}

/** The project list and every note title, for the index a phone shows with no project selected. */
export async function projectIndexFromCopy(
  source: CopySource = nativeCopy,
): Promise<{ projects: ProjectSummary[]; notes: ProjectRecord[]; copy: MirrorCopy } | null> {
  const copy = await readableCopy(source);
  if (!copy) return null;
  try {
    const [projects, notes] = await Promise.all([source.projects(), source.notes()]);
    return {
      projects,
      notes: notes.map(asRecord).toSorted((a, b) => a.title.localeCompare(b.title)),
      copy,
    };
  } catch {
    return null;
  }
}

/** One project's records from the copy. An empty list is a real answer: the copy may not hold it. */
export async function projectFromCopy(
  name: string,
  source: CopySource = nativeCopy,
): Promise<{ records: ProjectRecord[]; copy: MirrorCopy } | null> {
  const copy = await readableCopy(source);
  if (!copy) return null;
  try {
    const records = await source.projectRecords(name);
    return { records: records.map(asRecord), copy };
  } catch {
    return null;
  }
}

/** A row of the copy, in the shape the projects page already renders. */
export function asRecord(r: MirrorRecord): ProjectRecord {
  return {
    path: r.path,
    kind: r.kind,
    title: r.title,
    done: r.done,
    due: r.due,
    priority: r.priority,
    archived: r.archived,
    content: r.content,
  };
}

/**
 * How old the copy is, in words.
 *
 * The wording is `lib/ui/age.ts`'s (`tools/dashboard-age.test.ts` ratchets it to one module); the
 * policy is this page's, and it is the honest one: a copy the device could not date is not reported
 * as fresh, and not reported as a duration either.
 */
export function copyAge(copy: MirrorCopy): string {
  if (copy.age_ms === null) return 'at a build time this device could not read';
  return ageLabel(copy.age_ms / 1000);
}
