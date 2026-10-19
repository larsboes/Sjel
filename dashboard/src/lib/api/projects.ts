import type { DataClass } from './comms';
import { jsonInit, request } from './core';

/** One task as band 620 and the tables read it. Sjel owns tasks (ISA.md F14); the vault's task
 *  reader retired on 2026-10-18 once the vault's `Projects/` moved to its `Archive/`. */
export interface Task {
  /** The record path under the projects root. */
  id: string;
  title: string;
  done: boolean;
  due: string | null;
  /** 1 high · 2 medium · 3 low. A blank key reads as 2. */
  priority: number;
  summary: string | null;
  /** Display names of the note's `projects:` links. */
  projects: string[];
  /** The dashboard page that shows the task. */
  uri: string;
  /** What the note is worth protecting, decided by `content-item`'s vault classifier on the
   *  path the note had in the vault (PRD Q9a). */
  data_class: DataClass;
  /** Why that class, in the classifier's words. */
  data_class_rationale: string;
}

export type TaskStatus = 'open' | 'done';

/** One project, as `capabilities/projects` counts it. */
export interface ProjectSummary {
  name: string;
  archived: boolean;
  open: number;
  done: number;
  notes: number;
}

/** A task as the tasks table reads it: the vault shape plus its project folder and every
 *  frontmatter key, lists flattened to `a, b`. */
export interface ProjectTask extends Task {
  kind: 'project' | 'task' | 'note';
  project: string;
  fields: Record<string, string>;
}

/** The states a task can be in; `done` follows `status` (capabilities/projects/src/ops.rs). */
export const TASK_STATUSES = ['open', 'doing', 'waiting', 'done'] as const;

/** The states a project can be in; archiving is a folder move (capabilities/projects/src/ops.rs). */
export const PROJECT_STATUSES = ['active', 'done', 'cancelled'] as const;

/** One file of a project: hub, task or note. `content` is the markdown, frontmatter included. */
export interface ProjectRecord {
  path: string;
  kind: 'project' | 'task' | 'note';
  title: string;
  done: boolean;
  due: string | null;
  priority: number;
  archived: boolean;
  content: string;
}

/** The shared store owns projects and tasks (ISA.md F14). Tasks keep the vault's `Task` shape,
 *  so band 620 reads both sources while the import runs. Every write renders its markdown file
 *  before it answers. */
export const projects = {
  tasks: (status?: TaskStatus, signal?: AbortSignal) =>
    request<{ tasks: ProjectTask[] }>(
      `/projects/api/tasks${status ? `?status=${status}` : ''}`,
      signal ? { signal } : undefined,
    ).then((response) => response.tasks),
  /** Every live record of one kind, in the task shape with all its properties. */
  records: (kind: ProjectTask['kind'], signal?: AbortSignal) =>
    request<{ records: ProjectTask[] }>(`/projects/api/records?kind=${kind}`, signal ? { signal } : undefined).then(
      (response) => response.records,
    ),
  /** Every attachment path: the images, PDFs and other files of projects. */
  files: (signal?: AbortSignal) =>
    request<{ files: string[] }>('/projects/api/files', signal ? { signal } : undefined).then((response) => response.files),
  list: (signal?: AbortSignal) =>
    request<{ projects: ProjectSummary[] }>('/projects/api/projects', signal ? { signal } : undefined).then(
      (response) => response.projects,
    ),
  get: (name: string, signal?: AbortSignal) =>
    request<{ records: ProjectRecord[] }>(
      `/projects/api/project?name=${encodeURIComponent(name)}`,
      signal ? { signal } : undefined,
    ).then((response) => response.records),
  addTask: (body: { project: string; title: string; summary?: string; due?: string; priority?: number }) =>
    request<{ task: Task }>('/projects/api/tasks', jsonInit('POST', body)).then((response) => response.task),
  setDone: (id: string, done: boolean) =>
    request<{ task: Task }>('/projects/api/tasks/done', jsonInit('POST', { id, done })).then(
      (response) => response.task,
    ),
  /** Set one frontmatter property; an empty value clears it. The server checks the schema keys. */
  setProperty: (path: string, key: string, value: string) =>
    request<{ task: ProjectTask }>('/projects/api/record/property', jsonInit('POST', { path, key, value })).then(
      (response) => response.task,
    ),
  /** Replace a record's content. `base` is the content the edit started from: a stale base
   *  is refused with 409 and nothing is written. */
  save: (path: string, content: string, base: string) =>
    request<{ path: string }>('/projects/api/record', jsonInit('POST', { path, content, base })),
  addNote: (project: string, title: string, summary = '') =>
    request<{ path: string }>('/projects/api/notes', jsonInit('POST', { project, title, summary })),
  addProject: (name: string, summary = '') =>
    request<{ project: string }>('/projects/api/projects', jsonInit('POST', { name, summary })),
};
