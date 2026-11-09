# Projects and Tasks UI/UX Architecture

Projects, tasks, and project notes are owned by Sjel ([`crates/projects`](../../../crates/projects/README.md)).
This document records how they are stored and rendered, the direction for generative UI, and the
upstream references.

---

## 1. Storage and Rendering Pipeline

```
┌─────────────────────────────────┐
│  Sjel SQLite Database           │ (Canonical store)
│  (crates/projects/src/store.rs)  │
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
│  Native client                  │ GPUI on macOS, NSH on iOS
│  apps/gpui                      │ Projects view & database tables
└─────────────────────────────────┘
```

1. **Canonical store**: SQLite stores raw note contents and frontmatter properties. Attachments stay files in
   the rendered tree, not rows ([`crates/projects/src/store.rs`](../../../crates/projects/src/store.rs)).
2. **Second copy**: Every write to a record renders its markdown to `<overlay>/data/projects/`.
3. **Security boundary**: Attachments (`GET /api/file`) serve images and PDFs inline, while all other
   mime types (including HTML and SVGs with scripts) are served with `Content-Disposition: attachment`
   and `sandbox` restrictions ([`crates/projects/src/server.rs`](../../../crates/projects/src/server.rs)).

---

## 2. Generative and Intelligent UI Evolution

The step following figure embeds is **Generative UI from the typed core**
([Product Rule 6](../../../CONTRIBUTING.md#product-rules): *"Screens and assistant tools come from the typed data"*).

Rather than streaming unstructured markdown or generating insecure arbitrary HTML, intelligent agents in
Sjel compose structured interfaces out of an audited, declarative component catalog. The build
plan and its done-when are in
[`docs/plans/gpui.md`, Generated views](../../../docs/plans/gpui.md#generated-views-priority-3)
(decided 2026-11-09).

### Architectural Principles

1. **Declarative Component Schema**: The model emits a structured AST or declarative UI language (such as OpenUI Lang 1.0) specifying layout, data binding, and state variables (`$variables`).
2. **Local Model Execution ([Product Rule 1](../../../CONTRIBUTING.md#product-rules))**: UI generation runs locally on Apple Silicon (AFM / Ollama models like `qwen3.8:27b` or `gpt-oss:20b`), requiring low latency to the first token.
3. **Data Boundary & Privacy ([Product Rule 4](../../../CONTRIBUTING.md#product-rules))**: Components bind to local capability endpoints (SQL, TOML, links); raw personal context does not leave the device.
4. **Action Confirmation ([Product Rule 5](../../../CONTRIBUTING.md#product-rules))**: Reversible actions execute seamlessly; mutating or irreversible actions require explicit approval.

---

## 3. Upstream References and Inspirations

The following external projects inform Sjel's UI/UX:

- **[Open Intelligent UI](https://github.com/thesysdev/open-intelligent-ui)**:
  Open-source implementation of ChatGPT's Intelligent UI built on [OpenUI](https://openui.com). Defines declarative primitives: Layout (`Col`, `Row`, `Box`, `Card`, `Grid`, `Carousel`), Inputs (`Button`, `Slider`, `SegmentedControl` with `$variables`), Data (`Table`, `Chart`, `Map`), and pure `@functions`.
- **[OpenAI GPT-6 Intelligent UI](https://openai.com/de-DE/index/gpt-6-for-everyone/)**:
  Describes model responses as dynamic, interactive, context-aware graphic widgets rather than static text.
- **[Intelligent UI for Claude Code](https://github.com/razorback16/cc-intelligent-ui)**:
  Terminal-based Intelligent UI using Typst, CeTZ, and fletcher to render typeset markdown, charts, math, and diagrams directly in terminal emulators (Ghostty, kitty, iTerm2).
- **[GPUI Kit](https://gpui-kit.com/)**:
  Pre-built component primitives for Zed Industries' GPUI framework, relevant for Sjel's native Rust desktop client.

---

## 4. Unification: Documentation and Research

Sjel maintains two distinct public content trees:
- `docs/`: How the system is designed, operated, configured, and migrated.
- `research/`: Why the system exists, empirical benchmarks, and upstream audit verdicts.

Both are hosted persistently on the documentation site (`website/content`). Future work will unify
their shared navigation, search, and reading paths so that architecture decisions directly cite their supporting
empirical research and upstream audit entries.
