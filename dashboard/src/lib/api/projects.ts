import { jsonInit, request } from './core';
import type { Task, TaskStatus } from './vault';

/** One project, as `capabilities/projects` counts it. */
export interface ProjectSummary {
  name: string;
  archived: boolean;
  open: number;
  done: number;
  notes: number;
}

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
    request<{ tasks: Task[] }>(
      `/projects/api/tasks${status ? `?status=${status}` : ''}`,
      signal ? { signal } : undefined,
    ).then((response) => response.tasks),
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
  /** Replace a record's content. `base` is the content the edit started from: a stale base
   *  is refused with 409 and nothing is written. */
  save: (path: string, content: string, base: string) =>
    request<{ path: string }>('/projects/api/record', jsonInit('POST', { path, content, base })),
  addNote: (project: string, title: string, summary = '') =>
    request<{ path: string }>('/projects/api/notes', jsonInit('POST', { project, title, summary })),
  addProject: (name: string, summary = '') =>
    request<{ project: string }>('/projects/api/projects', jsonInit('POST', { name, summary })),
};
