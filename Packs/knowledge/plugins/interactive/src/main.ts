import { Plugin, MarkdownRenderChild, MarkdownPostProcessorContext, TFile, normalizePath } from 'obsidian';
import { parseBlock, escapeHtml } from './parser';
import { collectThemeVars } from './theme';
import { buildSrcdoc } from './iframe-renderer';
import { disposeNativeElement, renderNativeElement, updateNativeTheme, NativeRenderEntry } from './native-renderer';
import { MESSAGE_TYPE, MAX_HEIGHT } from './types';

interface FrameEntry {
  iframe: HTMLIFrameElement;
  fixedHeight: boolean;
  render: () => Promise<void>;
}

export default class HtmlEmbedPlugin extends Plugin {
  private frames = new Map<string, FrameEntry>();
  private nativeEntries = new Set<NativeRenderEntry>();
  private counter = 0;

  async onload() {
    this.registerMarkdownCodeBlockProcessor('html-embed', (source, el, ctx) => {
      const child = new MarkdownRenderChild(el);
      ctx.addChild(child);
      this.renderIframeBlock(source, el, ctx, child);
    });

    this.registerMarkdownCodeBlockProcessor('interactive', (source, el, ctx) => {
      const child = new MarkdownRenderChild(el);
      ctx.addChild(child);
      this.renderNativeBlock(source, el, ctx, child);
    });

    // Listen for iframe height reports
    this.registerDomEvent(window, 'message', (event) => {
      const data = event && event.data;
      if (!data || data.type !== MESSAGE_TYPE || typeof data.height !== 'number') return;
      const entry = this.frames.get(data.id);
      if (!entry || entry.iframe.contentWindow !== event.source) return;
      if (entry.fixedHeight) return;
      const h = Math.max(24, Math.min(MAX_HEIGHT, Math.ceil(data.height)));
      entry.iframe.style.height = `${h}px`;
    });

    // Immediate theme-change observer on document.body class
    const themeObserver = new MutationObserver((mutations) => {
      for (const m of mutations) {
        if (m.type === 'attributes' && m.attributeName === 'class') {
          for (const entry of this.nativeEntries.values()) entry.updateTheme();
          break;
        }
      }
    });
    themeObserver.observe(document.body, { attributes: true, attributeFilter: ['class'] });
    this.register(() => themeObserver.disconnect());

    // Rebuild frames and re-theme native elements when Obsidian CSS changes
    this.registerEvent(
      this.app.workspace.on('css-change', () => {
        for (const entry of this.frames.values()) entry.render();
        for (const entry of this.nativeEntries.values()) entry.updateTheme();
      })
    );
  }

  onunload() {
    for (const entry of this.nativeEntries) entry.destroy?.();
    this.frames.clear();
    this.nativeEntries.clear();
  }

  themeCss(): string {
    try {
      return collectThemeVars(getComputedStyle(document.body));
    } catch (e) {
      return '';
    }
  }

  async resolveFile(path: string, sourcePath?: string): Promise<TFile | null> {
    const clean = normalizePath(String(path).replace(/^\[\[|\]\]$/g, ''));
    let file = this.app.vault.getAbstractFileByPath(clean);
    if (!(file instanceof TFile)) {
      file = this.app.metadataCache.getFirstLinkpathDest(clean, sourcePath || '');
    }
    if (!(file instanceof TFile)) return null;
    return file;
  }

  async renderNativeBlock(
    source: string,
    el: HTMLElement,
    ctx: MarkdownPostProcessorContext,
    child: MarkdownRenderChild
  ) {
    const { opts, content } = parseBlock(source);
    el.empty();
    const wrap = el.createDiv({ cls: 'interactive-native iso' });

    let file: TFile | null = null;
    let rawContent = content;
    const plugin = this;
    let destroyed = false;
    let renderVersion = 0;

    const entry: NativeRenderEntry = {
      render: async function () {
        if (destroyed) return;
        const version = ++renderVersion;
        let nextContent = rawContent;
        try {
          if (file) nextContent = await plugin.app.vault.cachedRead(file);
        } catch (e) {
          nextContent = `<pre>interactive: could not read ${escapeHtml(String(opts.file))}</pre>`;
        }
        if (destroyed || version !== renderVersion) return;
        rawContent = nextContent;
        renderNativeElement(
          wrap,
          rawContent,
          opts,
          file ? file.name : undefined,
          file ? () => plugin.app.workspace.openLinkText(file!.path, ctx.sourcePath, 'tab') : undefined
        );
      },
      updateTheme: function () {
        if (!destroyed) updateNativeTheme(wrap);
      },
      destroy: function () {
        if (destroyed) return;
        destroyed = true;
        renderVersion++;
        disposeNativeElement(wrap);
      }
    };

    if (opts.file) {
      file = await this.resolveFile(opts.file, ctx.sourcePath);
      if (!file) {
        wrap.empty();
        wrap.createDiv({ cls: 'html-embed-error', text: `interactive: file not found — ${opts.file}` });
        return;
      }
      child.registerEvent(
        this.app.vault.on('modify', (f) => {
          if (f === file) entry.render();
        })
      );
    }

    this.nativeEntries.add(entry);
    child.register(() => {
      entry.destroy?.();
      plugin.nativeEntries.delete(entry);
    });

    await entry.render();
  }

  async renderIframeBlock(
    source: string,
    el: HTMLElement,
    ctx: MarkdownPostProcessorContext,
    child: MarkdownRenderChild
  ) {
    const { opts, content } = parseBlock(source);
    const id = `he-${++this.counter}-${Math.random().toString(36).slice(2, 8)}`;

    el.empty();
    const wrap = el.createDiv({ cls: 'html-embed' });
    const iframe = wrap.createEl('iframe', { cls: 'html-embed-frame' });
    iframe.setAttribute('sandbox', 'allow-scripts allow-same-origin allow-forms allow-popups');
    iframe.setAttribute('referrerpolicy', 'no-referrer');
    iframe.setAttribute('loading', 'lazy');
    iframe.setAttribute('title', opts.title ? String(opts.title) : opts.file ? `HTML embed: ${opts.file}` : 'HTML embed');

    const fixedHeight = typeof opts.height === 'number' ? opts.height : parseInt(String(opts.height), 10);
    const entry: FrameEntry = {
      iframe,
      fixedHeight: !isNaN(fixedHeight) && fixedHeight > 0,
      render: null as any
    };

    if (entry.fixedHeight) iframe.style.height = `${fixedHeight}px`;
    else iframe.style.height = '120px';

    const plugin = this;
    let rawContent = content;
    let file: TFile | null = null;

    if (opts.file) {
      file = await this.resolveFile(opts.file, ctx.sourcePath);
      if (!file) {
        wrap.empty();
        wrap.createDiv({ cls: 'html-embed-error', text: `html-embed: file not found — ${opts.file}` });
        return;
      }
      child.registerEvent(
        this.app.vault.on('modify', (f) => {
          if (f === file) entry.render();
        })
      );
    }

    entry.render = async function () {
      try {
        if (file) rawContent = await plugin.app.vault.cachedRead(file);
      } catch (e) {
        rawContent = `<pre>html-embed: could not read ${escapeHtml(String(opts.file))}</pre>`;
      }
      if (!rawContent || !rawContent.trim()) {
        rawContent = '<div style="opacity:.6;font-size:13px">html-embed: empty block</div>';
      }
      iframe.srcdoc = buildSrcdoc(rawContent, plugin.themeCss(), id, opts);
    };

    this.frames.set(id, entry);
    child.register(() => {
      plugin.frames.delete(id);
    });

    if (file) {
      const link = wrap.createDiv({ cls: 'html-embed-caption' });
      const a = link.createEl('a', { text: file.name, cls: 'internal-link' });
      a.addEventListener('click', (e) => {
        e.preventDefault();
        plugin.app.workspace.openLinkText(file!.path, ctx.sourcePath, 'tab');
      });
    }

    await entry.render();
  }
}
