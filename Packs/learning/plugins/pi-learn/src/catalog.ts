import type { App, TFile } from 'obsidian';

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
  return file.path.startsWith('Knowledge/') && (type === 'note' || type === 'claim');
}
