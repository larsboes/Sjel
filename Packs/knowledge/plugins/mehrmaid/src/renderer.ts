import { App, MarkdownRenderer, MarkdownRenderChild, MarkdownPostProcessorContext, loadMermaid } from 'obsidian';
import { getThemeConfig } from './theme';

export async function renderMarkdown(
  str: string,
  el: HTMLElement,
  ctx: MarkdownPostProcessorContext | undefined,
  app: App
): Promise<HTMLDivElement> {
  const child = new MarkdownRenderChild(el);
  const container = el.createDiv({ cls: 'mehrmaid-markdown-container' });
  if (ctx && typeof ctx !== 'string') {
    ctx.addChild(child);
  }
  await MarkdownRenderer.render(app, str, container, (ctx && ctx.sourcePath) || '', child);
  return container;
}

export async function renderMehrmaid(
  source: string,
  el: HTMLElement,
  ctx: MarkdownPostProcessorContext | undefined,
  app: App
): Promise<void> {
  const mermaid = await loadMermaid();
  const isDark = document.body.classList.contains('theme-dark');
  const config = getThemeConfig(isDark);

  mermaid.initialize(config);
  if (mermaid.mermaidAPI && mermaid.mermaidAPI.setConfig) {
    mermaid.mermaidAPI.setConfig(config);
  }

  const matches = source.match(/"([^"]*?)"/g);
  if (matches && matches.length > 0) {
    const promises = matches.map((m) => {
      const inner = m.substring(1, m.length - 1);
      return renderMarkdown(inner, el, ctx, app);
    });
    const markdownEls = await Promise.all(promises);

    const widths = markdownEls.map((m) => Math.max(m.offsetWidth, 10));
    const heights = markdownEls.map((m) => Math.max(m.offsetHeight, 10));

    el.empty();
    for (let i = 0; i < matches.length; i++) {
      const match = matches[i];
      const id = 'mm-' + match.replace(/[^a-zA-Z0-9]/g, '') + i;
      source = source.replace(
        match,
        `<div class="${id} cm-sizer" style="width:${widths[i]}px;height:${heights[i] - 4}px;display:inline-block;"></div>`
      );
    }

    const lineStart = (ctx && ctx.getSectionInfo && ctx.getSectionInfo(el)) ? ctx.getSectionInfo(el)!.lineStart : 0;
    const graphId = `mehrmaid-${lineStart}-${Math.random().toString(36).slice(2, 7)}`;
    const { svg } = await mermaid.render(graphId, source);
    el.insertAdjacentHTML('beforeend', svg);

    for (let i = 0; i < markdownEls.length; i++) {
      const match = matches[i];
      const id = 'mm-' + match.replace(/[^a-zA-Z0-9]/g, '') + i;
      const htmlEl = el.getElementsByClassName(id)[0];
      if (htmlEl) {
        htmlEl.appendChild(markdownEls[i]);
      }
    }
  } else {
    const lineStart = (ctx && ctx.getSectionInfo && ctx.getSectionInfo(el)) ? ctx.getSectionInfo(el)!.lineStart : 0;
    const graphId = `mehrmaid-${lineStart}-${Math.random().toString(36).slice(2, 7)}`;
    const { svg } = await mermaid.render(graphId, source);
    el.empty();
    el.insertAdjacentHTML('beforeend', svg);
  }
}
