<script lang="ts">
  type InlinePart =
    | { kind: "text" | "strong" | "code"; text: string }
    | { kind: "link"; text: string; href: string };

  type Block =
    | { kind: "heading"; level: number; parts: InlinePart[] }
    | { kind: "paragraph" | "quote"; parts: InlinePart[] }
    | { kind: "list"; ordered: boolean; items: InlinePart[][] }
    | { kind: "code"; language: string; text: string }
    | { kind: "table"; headers: InlinePart[][]; rows: InlinePart[][][] }
    | { kind: "rule" };

  let {
    content,
    compact = false,
    resolveLink,
  } = $props<{
    content: string;
    compact?: boolean;
    /**
     * Turns a relative link into a real one. Without it a relative link renders as its text,
     * which is right for mail and feed items: their relative links point at a site this app is
     * not. research/ passes one, because its links point at each other and at this repository.
     */
    resolveLink?: (href: string) => string;
  }>();
  const blocks = $derived(parseMarkdown(content));

  function inlineParts(value: string): InlinePart[] {
    const resolved = resolveLink
      ? value.replace(/\[([^\]]+)]\(([^)\s]+)\)/g, (_, text: string, href: string) => `[${text}](${resolveLink(href)})`)
      : value;
    const unlinked = resolveLink ? /\[([^\]]+)]\((?!https?:\/\/|\/)[^)]+\)/g : /\[([^\]]+)]\((?!https?:\/\/)[^)]+\)/g;
    const readable = resolved.replace(unlinked, "$1");
    const parts: InlinePart[] = [];
    // A root-relative href is kept only when resolveLink produced it; without a resolver,
    // `unlinked` has already turned it into text.
    const pattern = /(\[([^\]]+)]\((https?:\/\/[^)\s]+|\/[^)\s]*)\)|`([^`]+)`|\*\*([^*]+)\*\*)/g;
    let cursor = 0;
    for (const match of readable.matchAll(pattern)) {
      const start = match.index ?? 0;
      if (start > cursor) {
        parts.push({ kind: "text", text: readable.slice(cursor, start) });
      }
      if (match[2] && match[3]) {
        parts.push({ kind: "link", text: match[2], href: match[3] });
      } else if (match[4]) {
        parts.push({ kind: "code", text: match[4] });
      } else if (match[5]) {
        parts.push({ kind: "strong", text: match[5] });
      }
      cursor = start + match[0].length;
    }
    if (cursor < readable.length) {
      parts.push({ kind: "text", text: readable.slice(cursor) });
    }
    return parts.map((part) =>
      part.kind === "text"
        ? { ...part, text: part.text.replace(/(^|[\s(])[_*]([^_*]+)[_*](?=$|[\s).,])/g, "$1$2") }
        : part,
    );
  }

  function isBoundary(line: string): boolean {
    return (
      /^\s*$/.test(line) ||
      /^#{1,6}\s+/.test(line) ||
      /^(```|~~~)/.test(line) ||
      /^>\s?/.test(line) ||
      /^\s*([-*+]|\d+\.)\s+/.test(line) ||
      /^\s*([-*_])(?:\s*\1){2,}\s*$/.test(line)
    );
  }

  function tableCells(line: string): string[] {
    return line
      .trim()
      .replace(/^\|/, "")
      .replace(/\|$/, "")
      .split("|")
      .map((cell) => cell.trim());
  }

  // The stored text is already the canonical markdown: comms normalizes at
  // ingest (see capabilities/comms/src/normalize.rs), so the reader parses
  // what it is given rather than cleaning it a second time on every render.
  function parseMarkdown(source: string): Block[] {
    const lines = source.split("\n");
    const parsed: Block[] = [];
    let index = 0;

    while (index < lines.length) {
      const line = lines[index];
      if (!line.trim() || /^\s*\[[^\]]+]:\s+\S+/.test(line)) {
        index += 1;
        continue;
      }

      const fence = line.match(/^(```|~~~)\s*([\w-]*)/);
      if (fence) {
        const body: string[] = [];
        index += 1;
        while (index < lines.length && !lines[index].startsWith(fence[1])) {
          body.push(lines[index]);
          index += 1;
        }
        index += 1;
        parsed.push({ kind: "code", language: fence[2], text: body.join("\n") });
        continue;
      }

      const heading = line.match(/^(#{1,6})\s+(.+)$/);
      if (heading) {
        parsed.push({
          kind: "heading",
          level: heading[1].length,
          parts: inlineParts(heading[2].replace(/\s+#+$/, "")),
        });
        index += 1;
        continue;
      }

      if (/^\s*([-*_])(?:\s*\1){2,}\s*$/.test(line)) {
        parsed.push({ kind: "rule" });
        index += 1;
        continue;
      }

      if (
        index + 1 < lines.length &&
        line.includes("|") &&
        /^\s*\|?[\s:|-]+\|[\s:|-]*\|?\s*$/.test(lines[index + 1])
      ) {
        const headers = tableCells(line).map(inlineParts);
        const rows: InlinePart[][][] = [];
        index += 2;
        while (index < lines.length && lines[index].includes("|") && lines[index].trim()) {
          rows.push(tableCells(lines[index]).map(inlineParts));
          index += 1;
        }
        parsed.push({ kind: "table", headers, rows });
        continue;
      }

      const listItem = line.match(/^\s*([-*+]|\d+\.)\s+(.+)$/);
      if (listItem) {
        const ordered = /^\d/.test(listItem[1]);
        const items: InlinePart[][] = [];
        while (index < lines.length) {
          const item = lines[index].match(/^\s*([-*+]|\d+\.)\s+(.+)$/);
          if (!item || /^\d/.test(item[1]) !== ordered) break;
          items.push(inlineParts(item[2]));
          index += 1;
        }
        parsed.push({ kind: "list", ordered, items });
        continue;
      }

      if (/^>\s?/.test(line)) {
        const quote: string[] = [];
        while (index < lines.length && /^>\s?/.test(lines[index])) {
          quote.push(lines[index].replace(/^>\s?/, ""));
          index += 1;
        }
        parsed.push({ kind: "quote", parts: inlineParts(quote.join(" ")) });
        continue;
      }

      const paragraph = [line.trim()];
      index += 1;
      while (index < lines.length && !isBoundary(lines[index])) {
        paragraph.push(lines[index].trim());
        index += 1;
      }
      const text = paragraph.join(" ").trim();
      if (text) parsed.push({ kind: "paragraph", parts: inlineParts(text) });
    }
    return parsed;
  }
</script>

{#snippet inline(parts: InlinePart[])}
  {#each parts as part}
    {#if part.kind === "link"}
      {#if part.href.startsWith("/")}
        <a href={part.href}>{part.text}</a>
      {:else}
        <a href={part.href} target="_blank" rel="noreferrer">{part.text}</a>
      {/if}
    {:else if part.kind === "code"}
      <code>{part.text}</code>
    {:else if part.kind === "strong"}
      <strong>{part.text}</strong>
    {:else}
      {part.text}
    {/if}
  {/each}
{/snippet}

<div class="document" class:compact>
  {#each blocks as block}
    {#if block.kind === "heading"}
      {#if block.level <= 2}
        <h2>{@render inline(block.parts)}</h2>
      {:else}
        <h3>{@render inline(block.parts)}</h3>
      {/if}
    {:else if block.kind === "paragraph"}
      <p>{@render inline(block.parts)}</p>
    {:else if block.kind === "quote"}
      <blockquote>{@render inline(block.parts)}</blockquote>
    {:else if block.kind === "list"}
      {#if block.ordered}
        <ol>
          {#each block.items as item}<li>{@render inline(item)}</li>{/each}
        </ol>
      {:else}
        <ul>
          {#each block.items as item}<li>{@render inline(item)}</li>{/each}
        </ul>
      {/if}
    {:else if block.kind === "code"}
      <pre><code class:language={block.language}>{block.text}</code></pre>
    {:else if block.kind === "table"}
      <div class="table-wrap">
        <table>
          <thead>
            <tr>{#each block.headers as cell}<th scope="col">{@render inline(cell)}</th>{/each}</tr>
          </thead>
          <tbody>
            {#each block.rows as row}
              <tr>{#each row as cell}<td>{@render inline(cell)}</td>{/each}</tr>
            {/each}
          </tbody>
        </table>
      </div>
    {:else}
      <hr />
    {/if}
  {/each}
</div>

<style>
  .document {
    color: var(--text-secondary);
    font-size: 0.95rem;
    line-height: 1.72;
    overflow-wrap: anywhere;
  }

  .document :global(a) {
    color: var(--primary);
    text-decoration: underline;
    text-decoration-color: color-mix(in srgb, var(--primary) 35%, transparent);
    text-underline-offset: 0.15em;
  }

  .document :global(code) {
    border-radius: 0.25rem;
    padding: 0.1em 0.3em;
    background: var(--surface);
    color: var(--text-primary);
    font-size: 0.88em;
  }

  p,
  ul,
  ol,
  blockquote {
    margin: 0 0 1rem;
  }

  h2 {
    margin: 2.1rem 0 0.75rem;
    color: var(--text-primary);
    font-size: 1.25rem;
    line-height: 1.3;
  }

  h3 {
    margin: 1.6rem 0 0.6rem;
    color: var(--text-primary);
    font-size: 1rem;
    line-height: 1.35;
  }

  ul,
  ol {
    padding-left: 1.25rem;
  }

  li {
    margin: 0.25rem 0;
    padding-left: 0.15rem;
  }

  blockquote {
    padding: 0.1rem 0 0.1rem 1rem;
    border-left: 2px solid var(--primary);
    color: var(--text-secondary);
  }

  pre {
    max-width: 100%;
    margin: 1.25rem 0;
    padding: 1rem;
    overflow-x: auto;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    background: var(--surface);
  }

  pre code {
    padding: 0;
    background: none;
    white-space: pre;
  }

  hr {
    height: 1px;
    margin: 1.75rem 0;
    border: 0;
    background: var(--card-border);
  }

  .table-wrap {
    max-width: 100%;
    margin: 1.25rem 0;
    overflow-x: auto;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.85rem;
  }

  th,
  td {
    padding: 0.55rem 0.65rem;
    border-bottom: 1px solid var(--card-border);
    text-align: left;
    vertical-align: top;
  }

  th {
    color: var(--text-primary);
    font-weight: 600;
  }

  .compact {
    font-size: 1rem;
    line-height: 1.65;
  }

  .compact h2,
  .compact h3 {
    margin-top: 1.25rem;
  }

  .compact :global(:first-child) {
    margin-top: 0;
  }
</style>
