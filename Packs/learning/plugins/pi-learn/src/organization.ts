import { App, Modal, Notice, TFile } from 'obsidian';
import { isLearningPath } from './catalog';
import { vaultLayout } from './layout';

export interface PlacementIssue { file: TFile; message: string; destination?: string }

/** Finds structural problems without guessing a note's meaning or changing learning evidence. */
export function organizationIssues(app: App): PlacementIssue[] {
  const layout = vaultLayout(app);
  const issues: PlacementIssue[] = [];
  for (const file of app.vault.getMarkdownFiles()) {
    if (file.path.split('/').some((part) => part.startsWith('.')) || ['AGENTS.md', 'Knowledge.md'].includes(file.path)) continue;
    const frontmatter = app.metadataCache.getFileCache(file)?.frontmatter;
    const type = frontmatter?.type;
    const root = isLearningPath(app, file) || ['exercise', 'learning-path', 'log'].includes(type) ? layout.learning
      : ['concept', 'claim', 'moc'].includes(type) ? layout.knowledge
      : ['reference', 'system'].includes(type) ? layout.resources : null;
    if (root && !file.path.startsWith(`${root}/`)) {
      const destination = `${root}/${file.name}`;
      issues.push({ file, destination, message: `Move to ${destination}: ${type ?? 'learning path'}` });
    } else if (![layout.learning, layout.knowledge, layout.resources].some((folder) => file.path.startsWith(`${folder}/`))) {
      issues.push({ file, message: 'Choose Learning, Knowledge or Resources after reviewing this note.' });
    }
    const missing = ['type', 'maturity', 'summary', 'categories'].filter((key) => frontmatter?.[key] === undefined);
    if (missing.length) issues.push({ file, message: `Missing metadata: ${missing.join(', ')}` });
    for (const target of Object.keys(app.metadataCache.unresolvedLinks[file.path] ?? {}))
      issues.push({ file, message: `Unresolved link: ${target}` });
  }
  return issues;
}

/** Previews individual moves and applies them through Obsidian's link-aware file manager. */
export class OrganizationReview extends Modal {
  constructor(app: App, private readonly error = '') { super(app); }

  onOpen() {
    this.titleEl.setText('Check vault structure');
    this.contentEl.empty();
    if (this.error) { this.contentEl.createEl('p', { text: this.error }); return; }
    this.contentEl.createEl('p', { text: 'Review each move before applying it. Content, mastery and source verification stay unchanged. Semantic duplicates and merges require review of the notes.' });
    const issues = organizationIssues(this.app);
    if (!issues.length) this.contentEl.createEl('p', { text: 'No placement, metadata or unresolved-link issues found.' });
    for (const issue of issues) {
      const row = this.contentEl.createDiv();
      row.createEl('p', { text: `${issue.file.path} — ${issue.message}` });
      const open = row.createEl('button', { text: 'Open note' });
      open.onclick = () => { void this.app.workspace.openLinkText(issue.file.path, ''); };
      if (!issue.destination) continue;
      const destination = issue.destination;
      const apply = row.createEl('button', { text: 'Apply move' });
      apply.onclick = async () => {
        apply.disabled = true;
        try {
          if (this.app.vault.getAbstractFileByPath(destination)) throw new Error(`Destination exists: ${destination}`);
          const folder = destination.slice(0, destination.lastIndexOf('/'));
          if (!this.app.vault.getAbstractFileByPath(folder)) await this.app.vault.createFolder(folder);
          await this.app.fileManager.renameFile(issue.file, destination);
          this.onOpen();
        } catch (error) { new Notice(`Move failed: ${(error as Error).message}`); apply.disabled = false; }
      };
    }
  }
}
