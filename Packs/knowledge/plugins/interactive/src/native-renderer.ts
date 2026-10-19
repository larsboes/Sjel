import { BlockOptions } from './types';
import { escapeHtml } from './parser';

export interface NativeRenderEntry {
  render: () => Promise<void>;
  updateTheme: () => void;
  destroy?: () => void;
}

export function updateNativeTheme(wrap: HTMLElement): void {
  const isDark = document.body.classList.contains('theme-dark');
  const theme = isDark ? 'dark' : 'light';
  wrap.setAttribute('data-theme', theme);
  wrap.classList.toggle('theme-dark', isDark);
  wrap.classList.toggle('theme-light', !isDark);

  // Notify any child widgets listening for theme changes
  wrap.dispatchEvent(
    new CustomEvent('obsidian-theme-change', {
      detail: { theme, isDark },
      bubbles: true
    })
  );
  window.dispatchEvent(
    new CustomEvent('obsidian-theme-change', {
      detail: { theme, isDark },
      bubbles: true
    })
  );
}

export function renderNativeElement(
  wrap: HTMLElement,
  content: string,
  opts: BlockOptions,
  fileName?: string,
  onOpenFile?: () => void
): void {
  wrap.empty();
  updateNativeTheme(wrap);

  // Height handling: fixed or auto
  if (opts.height !== undefined && opts.height !== null && opts.height !== 'auto') {
    const num = typeof opts.height === 'number' ? opts.height : parseInt(String(opts.height), 10);
    if (!isNaN(num) && num > 0) {
      wrap.style.height = `${num}px`;
      wrap.style.overflow = 'hidden';
    } else {
      wrap.style.height = 'auto';
      wrap.style.overflow = 'visible';
    }
  } else {
    wrap.style.height = 'auto';
    wrap.style.overflow = 'visible';
  }

  const parser = new DOMParser();
  const doc = parser.parseFromString(content, 'text/html');

  // 1. Append styles
  doc.querySelectorAll('style').forEach((s) => {
    wrap.appendChild(document.importNode(s, true));
  });

  // 2. Append non-script body content
  if (doc.body) {
    Array.from(doc.body.childNodes).forEach((node) => {
      if (node.nodeName !== 'SCRIPT' && node.nodeName !== 'STYLE') {
        wrap.appendChild(document.importNode(node, true));
      }
    });
  }

  // 3. Execute scripts natively in Obsidian's context
  doc.querySelectorAll('script').forEach((s) => {
    const scriptEl = document.createElement('script');
    if (s.type) scriptEl.type = s.type;
    scriptEl.textContent = s.textContent;
    wrap.appendChild(scriptEl);
  });

  // 4. File link caption
  if (fileName && onOpenFile) {
    const link = wrap.createDiv({ cls: 'html-embed-caption' });
    const a = link.createEl('a', { text: fileName, cls: 'internal-link' });
    a.addEventListener('click', (e) => {
      e.preventDefault();
      onOpenFile();
    });
  }
}
