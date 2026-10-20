# Projects and Tasks UI/UX Architecture

Projects, tasks, and project notes are owned by Sjel ([`capabilities/projects`](../../capabilities/projects/README.md)).
This document records the UI/UX architecture for viewing and interacting with them, the live embed
mechanisms, the generative/intelligent UI evolution, and the upstream references.

---

## 1. Storage and Rendering Pipeline

```
┌─────────────────────────────────┐
│  Sjel SQLite Database           │ (Canonical store)
│  (capabilities/projects/store)  │
└────────────────┬────────────────┘
                 │ write
                 ▼
┌─────────────────────────────────┐
│  Rendered Markdown Tree         │ (Second copy / Principle 8)
│  <overlay>/data/projects/       │ Mode 0600, temp-plus-rename
└────────────────┬────────────────┘
                 │ HTTP API
                 ▼
┌─────────────────────────────────┐
│  Web Dashboard                  │ (Svelte 5 / Runes)
│  dashboard/src/routes/projects/ │ ProjectsBoard.svelte & DatabaseTable.svelte
└─────────────────────────────────┘
```

1. **Canonical store**: SQLite stores raw note contents, frontmatter properties, and file bytes.
2. **Second copy**: Every write to a record renders its markdown to `<overlay>/data/projects/`.
3. **Security boundary**: Attachments (`GET /api/file`) serve images and PDFs inline, while all other
   mime types (including HTML and SVGs with scripts) are served with `Content-Disposition: attachment`
   and `sandbox` restrictions ([`capabilities/projects/src/server.rs:271`](../../capabilities/projects/src/server.rs#L271)).

---

## 2. Note Embed Architecture

Project notes split their content into sequential segments via [`dashboard/src/lib/projects/database.ts`](../../dashboard/src/lib/projects/database.ts):

| Syntax | Segment Kind | Behavior |
| --- | --- | --- |
| Regular text / headings | `markdown` | Rendered via `MarkdownDocument.svelte` with interactive task checkboxes. |
| `![[file.png]]` | `file` (image) | Rendered as an inline responsive image (`<img class="attachment">`). |
| `![[doc.pdf]]` | `file` (document) | Rendered as a secure attachment link. |
| `![[Tasks.base#Active Tasks]]` | `embed` (Bases) | Rendered as a live interactive [`DatabaseTable.svelte`](../../dashboard/src/lib/projects/DatabaseTable.svelte) over Sjel's SQLite records. |
| `![[figure:<id>]]` | `figure` (Registered) | Rendered as a live, interactive Svelte 5 component from the audited figure registry. |

### The Figure Embed Registry

Untrusted HTML attachments cannot run arbitrary JavaScript in the dashboard origin. Instead, interactive
figures (such as isometric system schematics produced via the `anatomy` skill) are implemented as reviewed
Svelte 5 components and registered in [`dashboard/src/lib/figures/index.ts`](../../dashboard/src/lib/figures/index.ts).

The first registered figure is `miele-w961-filter` (Miele W 961 Mains Filter), embedded in
`Washing Machine/Washing Machine Connect and Test.md` to provide interactive isometric exploded views and
live measurement checks (M1–M5) directly within the note body.

---

## 3. Generative and Intelligent UI Evolution

The step following figure embeds is **Generative UI from the typed core**
([Product Rule 6](../../CONTRIBUTING.md#product-rules): *"Screens and assistant tools come from the typed data"*).

Rather than streaming unstructured markdown or generating insecure arbitrary HTML, intelligent agents in
Sjel compose structured interfaces out of an audited, declarative component catalog.

### Architectural Principles

1. **Declarative Component Schema**: The model emits a structured AST or declarative UI language (such as OpenUI Lang 1.0) specifying layout, data binding, and state variables (`$variables`).
2. **Local Model Execution ([Product Rule 1](../../CONTRIBUTING.md#product-rules))**: UI generation runs locally on Apple Silicon (AFM / Ollama models like `qwen3.8:27b` or `gpt-oss:20b`), requiring low latency to the first token.
3. **Data Boundary & Privacy ([Product Rule 4](../../CONTRIBUTING.md#product-rules))**: Components bind to local capability endpoints (SQL, TOML, links); raw personal context does not leave the device.
4. **Action Confirmation ([Product Rule 5](../../CONTRIBUTING.md#product-rules))**: Reversible actions execute seamlessly; mutating or irreversible actions require explicit approval.

---

## 4. Upstream References and Inspirations

The following references from the user vault (`Resources/Inbox/links.md`) and external projects inform Sjel's UI/UX:

- **[Open Intelligent UI](https://github.com/thesysdev/open-intelligent-ui)**:
  Open-source implementation of ChatGPT's Intelligent UI built on [OpenUI](https://openui.com). Defines declarative primitives: Layout (`Col`, `Row`, `Box`, `Card`, `Grid`, `Carousel`), Inputs (`Button`, `Slider`, `SegmentedControl` with `$variables`), Data (`Table`, `Chart`, `Map`), and pure `@functions`.
- **[OpenAI GPT-6 Intelligent UI](https://openai.com/de-DE/index/gpt-6-for-everyone/)**:
  Paradigm shift where model responses transition from static text output to dynamic, interactive, context-aware graphic widgets.
- **[Intelligent UI for Claude Code](https://github.com/razorback16/cc-intelligent-ui)**:
  Terminal-based Intelligent UI using Typst, CeTZ, and fletcher to render typeset markdown, charts, math, and diagrams directly in terminal emulators (Ghostty, kitty, iTerm2).
- **[GPUI Kit](https://gpui-kit.com/)**:
  Pre-built component primitives for Zed Industries' GPUI framework, relevant for Sjel's native Rust desktop client.

---

## 5. Unification: Documentation and Research

Sjel maintains two distinct public content trees:
- `docs/`: How the system is designed, operated, configured, and migrated.
- `research/`: Why the system exists, empirical benchmarks, and upstream audit verdicts.

Both are intended to be hosted persistently on the dashboard (`/docs` and `/research`). Future work will unify
their shared navigation, search, and reading paths so that architecture decisions directly cite their supporting
empirical research and upstream audit entries.

---

## 6. Notion-Style Project & Note Workspace

Implemented 2026-10-20 per user directive:
- **Interactive Whole-Machine Schematic (`miele-w961-machine`)**:
  - Registered in `dashboard/src/lib/figures/index.ts`.
  - Illustrates the complete Miele Gala W 961 washing machine in isometric projection: outer cabinet, rear wall opening, 230V mains cable, mains filter 4364110, control console & electronics (EDFU/EL), drive motor & belt, honeycomb drum & suspended suds container, 2000W heater with NTC sensor, water inlet solenoid valves, drain pump with lint filter, and drain hose 1995932.
  - Interactive subsystem filters: All Systems, Power & Filter, Water Circuit, Drive & Drum, Drain & Pump, Safety & M1–M5.
  - Interactive clickable hotspot pins with live component readout card and direct navigation to project notes.
- **Notion-Like CRUD Interactions**:
  - **Projects Sidebar**: Inline `+ New` project creator with name and summary inputs; project status counts (open tasks, notes count); Archive/Unarchive and Delete Project operations.
  - **Tasks**: Inline `+ Add task` form, deletion button per task row, opened task card with deletion and backlinks.
  - **Notes**: Styled as Notion page cards with page icon, title, backlink pills, inline editing, and delete actions with confirmation prompts.
  - **Backend Deletion Route**: Added `POST /api/record/remove` to `capabilities/projects/src/server.rs` with `RemoveRecord` schema; synchronized with store transaction and filesystem cleanup.
- **Wikilink Resolution & Hover Previews**:
  - Wikilinks (`[[Target|Label]]`) resolve dynamically against attachments, local project notes, and other projects across Sjel.
  - Floating hover preview card displays record title, kind badge, due/priority metadata, and an automated excerpt when hovering over any record link.
- **Backlinks ("Referenced by")**:
  - Automatically calculates incoming references (`findBacklinks`) across all project records.
  - Renders a clean "Referenced by (N)" chip list at the footer of notes and opened tasks, interconnecting the notes workspace like a database.

---

## 7. Scrollable and Interactive Preview Popovers

Hovering any wikilink or reference chip triggers a rich, interactive preview popover:
- **Interactive & Scrollable**: Pointer-events enabled (`pointer-events: auto`), allowing the operator to glide the mouse into the preview card and scroll through long excerpts (`max-height: 13rem; overflow-y: auto`).
- **Linger & Grace Period**: Integrated timer (`hoverTimer`) prevents premature dismissal when moving between the link and the card.
- **Rich Attribute Header**: Displays record kind badge (`note`, `task`, `project`), project origin pill, priority (`P1`–`P3`), due dates, maturity badges (`seedling`, `growing`, `mature`, `evergreen`), and status tags.
- **Direct Navigation**: Clicking the popover title or the "Open note ↗" footer link directly opens the record.

---

## 8. Dynamic Database Table View & Common Note Schema

Following the pattern established for tasks, notes now offer a dual representation:
1. **Cards View (`📑 Cards`)**: Collapsible Notion-like cards displaying full rendered markdown, inline checkbox toggling, and interactive figures.
2. **Database Table View (`🗂️ Database Table`)**: A dynamic, reactive table powered by [`DatabaseTable.svelte`](../../dashboard/src/lib/projects/DatabaseTable.svelte) querying SQLite directly (`projects.records("note")`).
   - Grouping, sorting, and dynamic column visibility persisted to local storage.
   - Inline cell editing writes directly through `POST /api/record/property` to SQLite and synchronizes to the rendered markdown tree.
   - Clicking any note title displays an opened detail view directly beneath the table with the full Markdown/Preview editor and backlink references.

### Standard Note Frontmatter Schema

Consistent with Sjel's Knowledge Base doctrine ("Type at birth", `Packs/knowledge/skills/obsidian/SKILL.md`):

| Property | Type | Allowed / Suggested Values | Description |
| --- | --- | --- | --- |
| `type` | string | `note`, `hub`, `concept`, `reference`, `moc` | Identifies the architectural role of the record. Read-only in database view. |
| `maturity` | enum | `seedling`, `growing`, `mature`, `evergreen` | Development stage of the note. Editable via in-table dropdown. |
| `summary` | string | Free text (≤ 200 chars) | Brief description of the note's scope and contents. |
| `categories` | list/string | `[[MOC]]`, domain wikilinks | Categorical joins to knowledge hubs or project tracks. |
| `status` | enum (optional)| `active`, `done`, `cancelled` | Optional lifecycle status for actionable project notes. |
| `due` / `scheduled`| date (optional) | `YYYY-MM-DD` | Date tracking where applicable. |

---

## 9. Generative UI Component Engine (`gen-ui`)

Sjel implements a native declarative Generative UI rendering system ([Product Rule 6](../../CONTRIBUTING.md#product-rules): *"Screens and assistant tools come from the typed data"*), inspired by [Open Intelligent UI](https://github.com/thesysdev/open-intelligent-ui) and OpenAI's Intelligent UI paradigms.

Instead of unvetted arbitrary HTML or raw prose, models and markdown notes specify structured, declarative JSON payloads inside fenced code blocks:

````markdown
```gen-ui
{
  "type": "metric-grid" | "decision-matrix" | "simulator" | "status-timeline",
  ...
}
```
````

### Supported Declarative Widget Catalog

1. **`metric-grid`**:
   - KPI & telemetry grid with live status indicators (`nominal`, `good`, `warning`, `critical`, `idle`, `active`), unit labels, and delta markers.
   - Component: [`MetricGrid.svelte`](../../dashboard/src/lib/genui/MetricGrid.svelte).
2. **`decision-matrix`**:
   - Multi-criteria deliberation matrix for engineering trade-offs.
   - Interactive criteria weight sliders (1–5) and option score inputs (1–10).
   - Computes live weighted totals and ranks candidate options with visual proportional score bars in real time.
   - Component: [`DecisionMatrix.svelte`](../../dashboard/src/lib/genui/DecisionMatrix.svelte).
3. **`simulator`**:
   - Interactive parameter simulator with sliders and segmented controls.
   - Real-time sandboxed formula evaluation (e.g. cycle energy kWh and electricity cost €).
   - Component: [`ParameterSimulator.svelte`](../../dashboard/src/lib/genui/ParameterSimulator.svelte).
4. **`status-timeline`**:
   - Sequential milestone tracker with status markers (`done`, `current`, `pending`, `blocked`) and expandable details.
   - Component: [`StatusTimeline.svelte`](../../dashboard/src/lib/genui/StatusTimeline.svelte).

### Authoring & AI Workflow

- **Snippet Inserter**: The top note editor toolbar includes a `[ ✨ + GenUI ▾ ]` dropdown menu that inserts pre-formatted templates with one click.
- **Spec Inspector**: Every GenUI widget includes a collapsible `"View Declarative Spec ▾"` drawer for auditability and schema inspection.
- **Live Parity**: Deployed directly in `Washing Machine Connect and Test.md` for live 5-point safety telemetry, filter replacement trade-offs, and cycle energy estimation.


