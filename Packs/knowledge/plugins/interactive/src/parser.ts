import { BlockOptions, ParsedBlock } from './types';

/** Parse the code block: leading `key: value` lines are options, the rest is HTML/script. */
export function parseBlock(source: string): ParsedBlock {
  const lines = source.split(/\r?\n/);
  const opts: BlockOptions = {};
  let i = 0;

  for (; i < lines.length; i++) {
    const m = /^\s*(file|height|title)\s*:\s*(.*?)\s*$/i.exec(lines[i]);
    if (!m) break;
    const key = m[1].toLowerCase();
    const val = m[2];
    if (key === 'height') {
      const parsed = parseInt(val, 10);
      opts.height = !isNaN(parsed) && parsed > 0 ? parsed : undefined;
    } else {
      opts[key] = val;
    }
  }

  // Skip blank separator lines
  while (i < lines.length && lines[i].trim() === '') i++;

  return { opts, content: lines.slice(i).join('\n') };
}

export function escapeHtml(s: string): string {
  return String(s).replace(/[&<>"']/g, (c) => {
    return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c] || c;
  });
}
