# Sjel Clipper

A Chrome extension that saves web pages into Sjel. It is [Obsidian Web Clipper](https://github.com/obsidianmd/obsidian-clipper)
1.7.1, vendored at `6d56d618` and detached, with one change of substance: a save goes to the
`comms` clip store on this Mac instead of to an Obsidian vault. Reader, persistent highlights,
templates, variables, filters, logic, the template editor and Interpreter are upstream code.
The audit and the full list of deltas are in [`upstreams.toml`](../../../upstreams.toml)
(`[obsidian-web-clipper]`).

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
prints one. `comms clips delete <key>` removes every version of one document; `comms clips delete-all
--yes` removes all stored clips. In **Feed › Clips**, open a document to delete it, or confirm
**Remove all** from the list.

Google AI Studio chat captures collapse exact repeated Markdown blocks of at least 200 characters
within each prompt and response. Fenced code and tables are preserved; an intentional repeated
paragraph is kept only once in the saved clip.

Large documents in **Feed › Clips** are parsed once and shown in pages of at most 80 Markdown
blocks. Citation auditing applies to the visible page, keeping scrolling and navigation bounded
without truncating the stored clip.

## YouTube clips

The popup and direct Markdown exports use asynchronous extraction for YouTube, allowing up to
30 seconds for captions and the transcript panel. The default content includes the video,
full description and timestamped transcript when extraction succeeds. An unavailable transcript
is marked in the captured content; opening YouTube's transcript panel and clipping again retries
the capture. This status describes the capture, not whether the video has captions.

Templates can select individual sections:

| Variable | Content |
|---|---|
| `{{transcript}}` | Timestamped transcript, or empty when unavailable |
| `{{transcriptStatus}}` | `available` or `unavailable` |
| `{{videoDescription}}` | Full extracted description in Markdown |
| `{{comments}}` | Up to 50 loaded top-level comments, with author attribution |
| `{{commentsCount}}` | Number of captured comments |
| `{{videoId}}` | YouTube video identifier |

Comments are optional: a template includes them with `{{comments}}`. Capture uses the threads
already loaded on the page, without scrolling, fetching more comments or expanding replies.
The limit applies to the first 50 loaded threads; exact duplicate comments are omitted.

## Settings Sjel owns

The templates and the Interpreter model belong to Sjel. Edit them in the Sjel app under
**Settings › Clipper**; saved clips are under **Feed › Clips**.

- `src/utils/sjel-settings.ts` asks comms for its settings at browser start, on install, every
  five minutes and whenever the settings page opens. A new revision replaces the browser's
  templates.
- The first time comms holds no templates, the browser hands over its own, once. After that the
  browser's template editor and Interpreter settings are read-only (`src/utils/sjel-lock.ts`).
- The Interpreter sends its prompt to comms, which answers on the light local role of Sjel's
  inference config, refused when that role is not on hardware the operator owns. The browser
  keeps no provider key. Reader and highlighter appearance stay per-browser settings.

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
- Review and routing of clips to their owning capabilities.
