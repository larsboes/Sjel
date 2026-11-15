import type { App, TFile } from 'obsidian';
import { vaultLayout } from './layout';

/** Vault content eligible for learning, excluding hidden folders and reference material. */
export function isLearningPath(app: App, file: TFile): boolean {
  return !file.path.split('/').some((part) => part.startsWith('.')) &&
    (app.metadataCache.getFileCache(file)?.headings ?? []).some((h) => h.level === 2 && h.heading === 'Learning Path');
}

export function learningPaths(app: App): TFile[] {
  return app.vault.getMarkdownFiles().filter((f) => isLearningPath(app, f)).sort((a, b) => a.path.localeCompare(b.path));
}

export function isLearningNote(app: App, file: TFile): boolean {
  const type = app.metadataCache.getFileCache(file)?.frontmatter?.type;
  return file.path.startsWith(`${vaultLayout(app).knowledge}/`) && (type === 'note' || type === 'concept' || type === 'claim');
}

/** Content eligible for curation; vault rules, logs and hidden archives are excluded. */
export function isCuratableNote(app: App, file: TFile): boolean {
  const layout = vaultLayout(app);
  return file.extension === 'md' && !file.path.split('/').some((part) => part.startsWith('.')) &&
    [layout.learning, layout.knowledge, layout.resources].some((folder) => file.path.startsWith(`${folder}/`)) &&
    file.path !== layout.log && app.metadataCache.getFileCache(file)?.frontmatter?.type !== 'system';
}

/** One-hop outgoing and incoming links, without recursive scope expansion. */
export function curationNeighbors(app: App, file: TFile): TFile[] {
  const links = app.metadataCache.resolvedLinks;
  return app.vault.getMarkdownFiles().filter((other) => other.path !== file.path && isCuratableNote(app, other) &&
    (links[file.path]?.[other.path] || links[other.path]?.[file.path])).sort((a, b) => a.path.localeCompare(b.path));
}
