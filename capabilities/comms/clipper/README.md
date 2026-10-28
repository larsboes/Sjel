# Sjel Clipper

A Chrome extension that saves web pages into Sjel. It is [Obsidian Web Clipper](https://github.com/obsidianmd/obsidian-clipper)
1.7.1, vendored at `6d56d618` and detached, with one change of substance: a save goes to the
`comms` clip store on this Mac instead of to an Obsidian vault. Reader, persistent highlights,
templates, variables, filters, logic, the template editor and Interpreter are upstream code.
The audit and the full list of deltas are in [`upstreams.toml`](../../../upstreams.toml)
(`[obsidian-web-clipper]`).

`../axon-clip` remains in service until this extension has replaced it in daily use.

## What a save does

The main button is **Add to Sjel**. Copy to clipboard and Save file (Markdown) stay as secondary
actions, as upstream has them.

1. The popup compiles the template as upstream does: frontmatter properties plus the note body.
2. `src/utils/sjel-destination.ts` builds a clip: the Markdown, the template's save behavior and
   destination, the page URL and metadata, and the page HTML, selection and highlights when the
   page provides them. Each clip carries a new retry key.
3. Chrome passes it through native messaging to `comms-clip-host`, which relays it to
   `comms-server` over a Unix socket at `<overlay>/data/comms/clip.sock`
   ([`web_clip_socket.rs`](../src/web_clip_socket.rs)). The socket has owner-only permissions. There is no
   HTTP route and no token in browser storage, so `sjel-status` has nothing to proxy.
4. The store keeps the clip as `c2` and starts no enrichment or cloud call.

When Sjel does not answer, the clip stays in the extension's local storage and the popup says so.
The background worker retries the queue at browser start and every five minutes. A clip that the
store refuses (invalid, conflicting or too large) moves to a failed list instead of blocking the
queue. If a page's HTML would take a clip over the store's 2 MiB bound, the HTML is dropped first
and the clip records `full_html_omitted`.

## Save behaviors

Upstream templates write to Obsidian notes. In Sjel each save is an immutable version of a clip
document, and the document shows the versions folded together:

| Template behavior | Clip document | Effect on its current Markdown |
|---|---|---|
| Create new note | `clip:<retry key>`, a document of its own | The clip |
| Overwrite note | `note:<path>/<note name>` | Replaces it; earlier versions stay stored |
| Append / prepend to specific note | `note:<path>/<note name>` | Adds after / before it |
| Append / prepend to daily note | `daily:<YYYY-MM-DD>`, the browser's local date | Adds after / before it |

Read them back without a browser: `comms clips` lists documents, and `comms clips show <key>`
prints one.

## Install

1. `tools/clipper build` builds the extension into `dist/`.
2. `tools/clipper install` builds `comms-clip-host`, writes its wrapper into the overlay and
   registers it with Chrome for this extension's fixed id (`extension-id`). It ends with
   `tools/clipper check`, which pings `comms-server` through the host.
3. In `chrome://extensions`, turn on Developer mode, choose **Load unpacked** and select `dist/`.

To bring your setup over from Obsidian Web Clipper, use its settings page: **Export all settings**
(templates and properties) and **Export highlights**. Then use **Import all settings** and the
highlights import in the Sjel Clipper's settings. Keep the exported files until the Sjel Clipper
shows your templates and highlights.

## Develop

```sh
bun install --frozen-lockfile --ignore-scripts
bun run check    # tsc and the upstream vitest suite
bun run build    # dist/ for Chrome
```

The `youtube` template fixture is skipped: it fails at `6d56d618` with upstream's own lockfile.
`defuddle`, `linkedom`, `highlight.js`, TypeScript and the `@types` packages are held at upstream's
lock versions so the extraction fixtures keep matching.

## Not yet

- Safari on Mac and iPhone, which needs an app wrapper and signing.
- Firefox. Its manifest is vendored but not built or tested.
- Interpreter through Sjel inference. Upstream's Interpreter calls the provider configured in
  the extension directly; point it at a local model until it routes through Sjel.
- Review and routing of clips to their owning capabilities.
