import { Plugin, Notice } from 'obsidian';
import { renderMehrmaid } from './renderer';

export default class MehrmaidPlugin extends Plugin {
  async onload() {
    this.app.workspace.onLayoutReady(() => {
      this.registerMarkdownCodeBlockProcessor('mehrmaid', async (source, el, ctx) => {
        try {
          await renderMehrmaid(source, el, ctx, this.app);
        } catch (e) {
          console.error(e);
          new Notice('Mehrmaid: Error rendering diagram');
        }
      });
    });
  }
}
