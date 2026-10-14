# Sjel agent bootstrap

For Sjel operations, run `sjel help` or `sjel search <task>` before browsing files. For Sjel
repository changes, follow this file's data-boundary, repository, and GitHub workflow rules. Use
Graphify only as optional symbol-level drill-down after bounded Sjel context is available.

Before editing, run `git status --short --branch` and `tools/doctor`. Preserve unrelated worktree
changes. Derive capabilities, ports, health, open claims and architecture from Sjel tools, the owning
`ISA.md` and live metadata rather than remembered prose.

## Branch discipline and multi-session concurrency

- **Current branch only**: Work strictly in the currently checked-out branch.
- **Direct to `main` until official release**: For a long time until the application is officially released, work directly on the `main` branch. Do not create separate branches or switch branches unless explicitly requested.
- **Post-release branching**: Only after the app is officially released will the repository transition to feature branches and a dev version (`dev`).
- **Parallel agent sessions**: Be aware that other agent sessions frequently work in parallel in this repository. Always inspect `git status --short --branch` before editing, never stash, revert, or overwrite changes made by concurrent sessions, and touch only the files belonging to your task.


Host requirements are scoped to what a machine actually does. After changing a machine's enabled
capabilities, re-run `tools/toolchain-check` — the requirement set is derived from that list, so it
moves with it. Before invoking a backup, restore, audit or build, run
`tools/toolchain-check --workflow <name>` first: a missing dependency found afterwards is one found
with a service already held down.

Keep public code and doctrine in Sjel. Keep private values, machine state and secrets in the active
overlay or Vaultwarden. Never generate, expose or change a secret without explicit authorization.

## Code, documentation, and planning doctrine (Vector & Rust Reference standard)

- **Code is the source of truth**: Code, tests, and AST knowledge graphs (`graphify`) are the ground truth for humans and AI agents. Avoid sprawling markdown decision logs or PR retrospectives in the tree.
- **Technical documentation structure**:
  - Technical manuals live under `docs/capabilities/<name>.md` (interface contracts, data flows, Mermaid diagrams, operational guarantees).
  - Capability `README.md` files are lean entrypoints containing Why-blocks and point to `docs/capabilities/<name>.md`.
- **UI component modularity**:
  - Route views in `dashboard/src/routes/` are thin orchestrators. Avoid multi-thousand-line page monoliths.
  - Extract self-contained boards, search columns, solver panels, cards, and modal sheets into `dashboard/src/lib/<domain>/<Component>.svelte`.
- **Short, precise planning & claims**: Issues and open claims in `PLAN.md` or `ISA.md` must be short, falsifiable, and precise specifications, never narrative essays.
- **Self-explanatory code over prose comments**:
  - Write expressive, strongly typed code (Newtypes, type-state pattern, explicit enums).
  - If code needs a paragraph to explain *what* it does, refactor the code and variable names instead.
- **Interface contracts over historical narrative**:
  - **Zero history in code**: Never put dates, PR numbers, ISC tickets, or past migration stories into comments. Git log owns history.
  - **Document public boundaries**: Use standard rustdoc (`///`) or JSDoc (`/** ... */`) for every public type, struct field, function, parameter, and error condition (`# Errors`).
  - **Internal comments**: Maximum 1–2 lines strictly explaining non-obvious hardware, kernel, or concurrency invariants.
- **Visual diagrams for architecture**:
  - Prefer concise Mermaid or ASCII diagrams in module READMEs to illustrate data flow, state transitions, or component pipelines.
- **Opportunistic trimming**:
  - When editing a module, trim sprawling narrative comments to this rule. Do not run a separate sweep.

## Interaction and output style (ADHD-friendly / low cognitive load)

Format responses to minimize cognitive load, following `attention-control` doctrine:

- **Lead with the bottom line**: State the overall outcome and verification state in the first line.
- **Structured and chunked**: Use numbered headings and bullet points with **bold lead-ins**. Avoid narrative paragraphs.
- **Hard metrics and paths**: State exact measurements, test counts (`17/17 gates`, `0 fail`), commit SHAs, and clickable file links.
- **Isolate human-only actions**: Put manual tasks the operator must perform into their own dedicated block with a concrete time estimate (e.g. `2 minutes, manual`).
- **Zero conversational fluff**: Cut pleasantries, apologetic hedging, recaps, and meta-commentary. State results and facts flatly.

## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

When the user types `/graphify`, use the installed graphify skill or instructions before doing anything else.

Rules:
- For codebase questions, first run `graphify query "<question>"` when graphify-out/graph.json exists. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- Dirty graphify-out/ files are expected after hooks or incremental updates; dirty graph files are not a reason to skip graphify. Only skip graphify if the task is about stale or incorrect graph output, or the user explicitly says not to use it.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).
