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

export type Segment = { markdown: string } | { embed: Embed } | { file: string; image: boolean };

/** Split a note body into markdown runs, base embeds and attachment embeds, in order. */
export function segments(body: string, host: string, files: string[] = []): Segment[] {
  const out: Segment[] = [];
  let run: string[] = [];
  for (const line of body.split('\n')) {
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
