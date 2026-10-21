import { describe, expect, test } from 'bun:test';
import type { ProjectSummary } from '../src/lib/api';
import type { MirrorCopy, MirrorRecord } from '../src/lib/mirror';
import {
  asRecord,
  copyAge,
  nativeCopy,
  projectFromCopy,
  projectIndexFromCopy,
  readableCopy,
  type CopySource,
} from '../src/lib/projects/offline';

const g = globalThis as unknown as { window?: Record<string, unknown> };

/** `inTauri()` reads this, the same way `bridged-url.test.ts` enters the app. */
function enterTauri() {
  g.window = { __TAURI_INTERNALS__: { invoke: () => 0, transformCallback: () => 0 } };
}

function leaveTauri() {
  delete g.window;
}

function copy(overrides: Partial<MirrorCopy> = {}): MirrorCopy {
  return {
    built_at: '2026-10-21T21:21:30Z',
    built_at_ms: 1_792_617_690_000,
    age_ms: 3 * 60 * 60 * 1000,
    stale: false,
    fetched_at_ms: 1_792_628_490_000,
    bytes: 11_141_120,
    rows: 1_871,
    tables: [{ name: 'projects_records', rows: 1_221 }],
    sha256: '92d99387',
    verified: true,
    integrity: 'ok',
    foreign_key_violations: 0,
    ...overrides,
  };
}

function record(overrides: Partial<MirrorRecord> = {}): MirrorRecord {
  return {
    path: 'Alpha/Notes/Train note.md',
    project: 'Alpha',
    kind: 'note',
    title: 'Train note',
    done: false,
    due: null,
    priority: 2,
    archived: false,
    content: '# Train note\n\nwritten on a train',
    ...overrides,
  };
}

function source(overrides: Partial<CopySource> = {}): CopySource {
  return {
    status: async () => ({ copy: copy() }),
    projects: async (): Promise<ProjectSummary[]> => [{ name: 'Alpha', archived: false, open: 2, done: 1, notes: 1 }],
    notes: async () => [record({ path: 'Alpha/Notes/b.md', title: 'b' }), record({ title: 'a' })],
    projectRecords: async () => [record()],
    ...overrides,
  };
}

describe('deciding whether the copy may be read', () => {
  test('a verified copy is read', async () => {
    enterTauri();
    expect(await readableCopy(source())).not.toBeNull();
    leaveTauri();
  });

  test('an unverified copy is not read, and is not reported as a failure either', async () => {
    enterTauri();
    const unverified = source({ status: async () => ({ copy: copy({ verified: false }) }) });
    expect(await readableCopy(unverified)).toBeNull();
    leaveTauri();
  });

  test('no copy, a failing command, and a browser tab all answer null', async () => {
    enterTauri();
    expect(await readableCopy(source({ status: async () => ({ copy: null }) }))).toBeNull();
    expect(
      await readableCopy(
        source({
          status: async () => {
            throw new Error('the store is not open');
          },
        }),
      ),
    ).toBeNull();
    leaveTauri();
    // Outside the app the command does not exist at all; `nativeCopy` must not be called.
    expect(await readableCopy(nativeCopy)).toBeNull();
  });

  test('the project index carries every note, ordered by title', async () => {
    enterTauri();
    const index = await projectIndexFromCopy(source());
    const titles = index?.notes.map((n) => n.title);
    expect(index?.projects.map((p) => p.name)).toEqual(['Alpha']);
    expect(titles).toEqual(['a', 'b']);
    expect(index?.copy.rows).toBe(1_871);
    leaveTauri();
  });

  test('a project the copy does not hold is an empty answer, not a failure', async () => {
    enterTauri();
    const empty = await projectFromCopy('Not In The Copy', source({ projectRecords: async () => [] }));
    expect(empty).not.toBeNull();
    expect(empty?.records).toEqual([]);
    leaveTauri();
  });

  test('a project read that fails falls through to the ordinary error path', async () => {
    enterTauri();
    const broken = source({
      projectRecords: async () => {
        throw new Error('no such table');
      },
    });
    expect(await projectFromCopy('Alpha', broken)).toBeNull();
    leaveTauri();
  });
});

describe('the row the page renders', () => {
  test('keeps only what the copy holds, and invents nothing', () => {
    const row = asRecord(record());
    expect(row.path).toBe('Alpha/Notes/Train note.md');
    expect(row.content).toContain('written on a train');
    expect(row.kind).toBe('note');
    // The vault fields the Mac adds are absent, not empty strings or defaults.
    expect('data_class' in row).toBe(false);
    expect('fields' in row).toBe(false);
    expect('uri' in row).toBe(false);
  });
});

describe('the age the banner shows', () => {
  test('reads as a duration from the one module that phrases one', () => {
    expect(copyAge(copy({ age_ms: 30_000 }))).toBe('just now');
    expect(copyAge(copy({ age_ms: 14 * 60_000 }))).toBe('14m ago');
    expect(copyAge(copy({ age_ms: 3 * 3_600_000 }))).toBe('3h ago');
    expect(copyAge(copy({ age_ms: 50 * 3_600_000 }))).toBe('2d ago');
    expect(copyAge(copy({ age_ms: null }))).toContain('could not read');
  });
});
