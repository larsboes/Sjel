import type { DataClass } from './comms';
import { request } from './core';

/** One action, read out of a note under the vault's `Projects/**\/Tasks/`.
 *
 *  This replaced a database row on 2026-08-27 (PRD Q48): the `tasks`
 *  capability retired and the Action kind went back to the vault, which is
 *  where the vault contract §5.1b had assigned it all along.
 *
 *  Every field here has a reader below — `due` and `priority` rank the row,
 *  `title` and `summary` render it, `projects` labels it, `done` decides
 *  whether it is a decision at all. The note carries six more frontmatter
 *  keys that nothing on this page reads, so the server does not serve them. */
export interface Task {
  /** Vault-relative path. The identity that survives a machine. */
  id: string;
  title: string;
  done: boolean;
  due: string | null;
  /** 1 high · 2 medium · 3 low. The server defaults a blank key to 2, the same
   *  fallback the vault's own Tasks.base applies. */
  priority: number;
  summary: string | null;
  /** Display names of the note's `projects:` links. */
  projects: string[];
  /** `obsidian://open?…` — where the operator goes to act on it. */
  uri: string;
  /** What the note is worth protecting, decided by `content-item`'s vault classifier:
   *  the folder sets it, the note's own `class:` key overrides it (PRD Q9a).
   *
   *  Not optional, unlike the feed's and the calendar's: `/api/tasks` is the only route
   *  the vault server serves, so there is no second path a task can arrive by without
   *  one. Never `c0` — publishing is an act, and no folder means "already public". */
  data_class: DataClass;
  /** Why that class, in the classifier's words. The one branch the value cannot show is a
   *  note whose `class:` key is not a class at all: it is refused rather than honoured or
   *  escalated, so it reads as its folder's default and only this sentence says so. */
  data_class_rationale: string;
}

export type TaskStatus = 'open' | 'done';

/** Read-only, and the whole namespace is one call.
 *
 *  There is no create and no patch because the vault server has no write
 *  route: a task is written, edited and marked done in Obsidian, in a note a
 *  human owns. Sjel reads the vault and does not write to it (PRD §5.5). */
export const vault = {
  tasks: (status?: TaskStatus, signal?: AbortSignal) =>
    request<{ tasks: Task[] }>(
      `/vault/api/tasks${status ? `?status=${status}` : ''}`,
      signal ? { signal } : undefined,
    ).then((response) => response.tasks),
};
