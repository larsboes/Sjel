import type { ProjectTask } from '$lib/api';

/** The folder of a record path: `A/Tasks/x.md` → `A/Tasks`. */
export const folder = (path: string) => path.split('/').slice(0, -1).join('/');

/** The note names a property's wikilinks point at. `[[Projects/Life-Plan/Life-Plan|Life-Plan]]`
 *  → `Life-Plan`: the target's last segment, not the label, so a renamed label still matches. */
export const linkTargets = (value: string | undefined) =>
  [...(value ?? '').matchAll(/\[\[([^\]|#]+)/g)].map((m) => m[1].split('/').pop()!.trim());

/** What an embedded view shows: a live table over Sjel's records, or a base the vault still owns. */
export type Embed =
  | {
      kinds: ProjectTask['kind'][];
      label: string;
      columns: string[];
      filter: (r: ProjectTask) => boolean;
    }
  | { vault: string; label: string };

const EMBED = /^!\[\[([^\]#|]+\.base)(?:#([^\]|]+))?(?:\|([^\]]+))?\]\]\s*$/;

/** Read one line as a Bases embed, for the note at `host`. Null when the line is not an embed.
 *
 *  The views below are the ones project notes embed (counted 2026-10-18), translated from the
 *  vault's `Resources/Bases/*.base` filters. Every other base reads vault data Sjel does not
 *  own, so it stays a link into the vault. */
export function embed(line: string, host: string): Embed | null {
  const match = EMBED.exec(line.trim());
  if (!match) return null;
  const [, file, view = '', alias] = match;
  const base = file.split('/').pop()!.replace(/\.base$/, '');
  const label = alias ?? (view ? `${base} · ${view}` : base);
  const title = host.split('/').pop()!.replace(/\.md$/, '');
  const here = folder(host);
  const name = (r: ProjectTask) => r.id.split('/').pop()!.replace(/\.md$/, '');
  switch (`${base}#${view}`) {
    // `projects.contains(this.file)`
    case 'Tasks#Active Tasks (by Project)':
      return {
        kinds: ['task'],
        label,
        columns: ['status', 'due', 'priority'],
        filter: (r) => linkTargets(r.fields.projects).includes(title),
      };
    // `file.folder == this.file.folder && file.name != this.file.name && type == "project"`
    case 'Projects#📂 Candidates (same folder)':
      return {
        kinds: ['project'],
        label,
        columns: ['status', 'priority', 'outcome'],
        filter: (r) => folder(r.id) === here && name(r) !== title,
      };
    // `file.folder.split("/").slice(0, -1).join("/") == this.file.folder && file.name != this.file.name`
    case 'Project-Notes#🗂️ One Level Down':
      return {
        kinds: ['project', 'task', 'note'],
        label,
        columns: ['summary', 'type', 'status'],
        filter: (r) => folder(folder(r.id)) === here && name(r) !== title,
      };
    default:
      return { vault: file.includes('/') ? file : `Resources/Bases/${file}`, label };
  }
}

/** Where the dashboard reads one attachment (`capabilities/projects`, `GET /api/file`). */
export const fileUrl = (path: string) => `/projects/api/file?path=${encodeURIComponent(path)}`;

const IMAGE = /\.(png|jpe?g|gif|webp|svg)$/i;

/** The attachment a wikilink target names, resolved the way Obsidian resolves it: a path that
 *  ends in the target, the host's own project first. `Assets/a.png|400` drops its size. */
export function resolveFile(target: string, host: string, files: string[]): string | null {
  const name = target.split('|')[0].trim();
  const project = host.startsWith('Archive/') ? host.split('/').slice(0, 2).join('/') : host.split('/')[0];
  const matches = files.filter((f) => f === name || f.endsWith(`/${name}`));
  return matches.find((f) => f.startsWith(`${project}/`)) ?? matches[0] ?? null;
}

import type { GenUISpec } from '../genui/types';

export type Segment =
  | { markdown: string }
  | { embed: Embed }
  | { file: string; image: boolean }
  | { figure: string; label?: string }
  | { genui: GenUISpec };

const FIGURE = /^!\[\[figure:([a-zA-Z0-9_-]+)(?:\|([^\]]+))?\]\]\s*$/;

/** Split a note body into markdown runs, base embeds, figures, gen-ui widgets and attachment embeds, in order. */
export function segments(body: string, host: string, files: string[] = []): Segment[] {
  const out: Segment[] = [];
  let run: string[] = [];
  let inGenUI = false;
  let genUILines: string[] = [];

  for (const line of body.split('\n')) {
    if (inGenUI) {
      if (line.trim().startsWith('```')) {
        inGenUI = false;
        const text = genUILines.join('\n').trim();
        try {
          const spec = JSON.parse(text) as GenUISpec;
          out.push({ genui: spec });
        } catch (e) {
          out.push({
            genui: {
              type: 'raw',
              raw: text,
              error: e instanceof Error ? e.message : String(e),
            },
          });
        }
        genUILines = [];
      } else {
        genUILines.push(line);
      }
      continue;
    }

    if (line.trim().startsWith('```gen-ui')) {
      if (run.join('').trim()) out.push({ markdown: run.join('\n') });
      run = [];
      inGenUI = true;
      genUILines = [];
      continue;
    }

    const fig = FIGURE.exec(line.trim());
    if (fig) {
      if (run.join('').trim()) out.push({ markdown: run.join('\n') });
      run = [];
      out.push({ figure: fig[1], label: fig[2]?.trim() });
      continue;
    }
    const attached = /^!\[\[([^\]]+)\]\]\s*$/.exec(line.trim());
    const file = attached && !/\.(md|base)(#|\||$)/.test(attached[1]) ? resolveFile(attached[1], host, files) : null;
    if (file) {
      if (run.join('').trim()) out.push({ markdown: run.join('\n') });
      run = [];
      out.push({ file, image: IMAGE.test(file) });
      continue;
    }
    const found = embed(line, host);
    if (!found) {
      run.push(line);
      continue;
    }
    if (run.join('').trim()) out.push({ markdown: run.join('\n') });
    run = [];
    out.push({ embed: found });
  }
  if (run.join('').trim()) out.push({ markdown: run.join('\n') });
  return out;
}

/** A Markdown task-list line: `- [ ] text`, `* [x] text` or `1. [ ] text`. */
const CHECKBOX = /^(\s*(?:[-*+]|\d+\.)\s+)\[([ xX])\](?=\s)/;

/** How many task-list lines a run of markdown holds, outside code fences, in the order
 *  MarkdownDocument renders them. A segment's first checkbox is the sum of the runs before it. */
export function countCheckboxes(markdown: string): number {
  let fenced = false;
  let count = 0;
  for (const line of markdown.split('\n')) {
    if (/^(```|~~~)/.test(line)) fenced = !fenced;
    else if (!fenced && CHECKBOX.test(line)) count += 1;
  }
  return count;
}

/** The record's content with its `n`-th checkbox (from 0, after the frontmatter, outside code
 *  fences) set to `checked`, or null when there is no such checkbox. The caller saves it with
 *  the old content as `base`, so a toggle on a stale page is refused rather than lost. */
export function setCheckbox(content: string, n: number, checked: boolean): string | null {
  const lines = content.split('\n');
  let start = 0;
  if (lines[0] === '---') {
    const end = lines.indexOf('---', 1);
    if (end > 0) start = end + 1;
  }
  let fenced = false;
  let seen = 0;
  for (let i = start; i < lines.length; i++) {
    if (/^(```|~~~)/.test(lines[i])) {
      fenced = !fenced;
      continue;
    }
    if (fenced || !CHECKBOX.test(lines[i])) continue;
    if (seen++ === n) {
      lines[i] = lines[i].replace(CHECKBOX, `$1[${checked ? 'x' : ' '}]`);
      return lines.join('\n');
    }
  }
  return null;
}

/** The project a record belongs to. */
export const recordProject = (path: string): string => {
  if (path.startsWith('Archive/')) {
    return path.split('/').slice(0, 2).join('/');
  }
  return path.split('/')[0] ?? '';
};

/** The bare title of a record path. `Washing Machine/Washing Machine Electrics.md` → `Washing Machine Electrics`. */
export const recordTitle = (path: string): string =>
  path.split('/').pop()!.replace(/\.md$/, '');

/** Resolve a wikilink target against available project records. */
export function resolveRecord<T extends { path: string; title?: string }>(
  target: string,
  host: string,
  records: T[],
): T | null {
  const clean = target.split('|')[0].split('#')[0].trim().replace(/\.md$/, '');
  const targetName = clean.split('/').pop()!;
  const hostProj = recordProject(host);

  const byPath = records.find((r) => r.path === clean || r.path === `${clean}.md` || r.path.endsWith(`/${clean}.md`));
  if (byPath) return byPath;

  const matches = records.filter(
    (r) => recordTitle(r.path) === targetName || (r.title && r.title === targetName),
  );
  if (matches.length === 0) return null;
  return matches.find((r) => recordProject(r.path) === hostProj) ?? matches[0];
}

/** Find all records in `records` that contain a wikilink reference to `target`. */
export function findBacklinks<T extends { path: string; title?: string; content: string }>(
  target: { path: string; title?: string },
  records: T[],
): T[] {
  const title = target.title ?? recordTitle(target.path);
  const targetBase = recordTitle(target.path);
  return records.filter((r) => {
    if (r.path === target.path) return false;
    const body = r.content;
    return (
      body.includes(`[[${title}`) ||
      body.includes(`[[${targetBase}`) ||
      body.includes(`[[${target.path}`) ||
      body.includes(`[[Projects/${target.path}`)
    );
  });
}

/** Extract frontmatter key-value pairs from raw markdown. */
export function extractFrontmatter(content: string): Record<string, string> {
  const match = content.match(/^---\n([\s\S]*?)\n---/);
  if (!match) return {};
  const fields: Record<string, string> = {};
  for (const line of match[1].split('\n')) {
    const colon = line.indexOf(':');
    if (colon > 0) {
      const k = line.slice(0, colon).trim();
      const v = line.slice(colon + 1).trim().replace(/^['"]|['"]$/g, '');
      if (k && v) fields[k] = v;
    }
  }
  return fields;
}

/** Extract a text excerpt for hover preview cards. */
export function extractExcerpt(content: string, maxLines = 3, separator = ' '): string {
  const stripped = content.replace(/^---\n[\s\S]*?\n---\n/, '').trim();
  const lines = stripped
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => l.length > 0 && !l.startsWith('#') && !l.startsWith('!'));
  return lines.slice(0, maxLines).join(separator);
}

