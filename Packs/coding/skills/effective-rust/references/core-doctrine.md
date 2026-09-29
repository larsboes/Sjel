# Core Doctrine & Placement

## Placement Rules
Sjel separates backend code across three rungs:
1. **`capabilities/<name>`**: Self-contained business domains (e.g. `calendar`, `finance`, `trips`, `vault`). Each capability exposes a loopback HTTP API through `sjel-server`, manages its own persistence via `sjel-store`, and maintains isolated domain logic.
2. **`libs/<name>`**: Shared, domain-agnostic infrastructure. Libs must not know about individual capabilities. Examples:
   - `sjel-store`: SQLite connection pooling, migrations, table prefixes.
   - `sjel-server`: Loopback bind enforcement, auth gates, route manifests.
   - `sjel-http`: Canonical outbound HTTP client with shared User-Agent and timeout rules.
   - `sjel-config`: Tilde expansion, environment variables, overlay paths.
3. **`tools/<name>`**: Operator machinery and build helpers (`storage`, `fda-launcher`).

## Workspace Dependencies
- Declare shared dependency versions in the root `Cargo.toml` `[workspace.dependencies]` table.
- Workspace members inherit versions via `dep = { workspace = true }`.
- Feature flags stay with the specific crate member needing them (e.g. `reqwest = { workspace = true, features = ["json"] }`), unless universal across all consumers.

## Safety & Invariants
- The workspace enforces `#![deny(unsafe_code)]` via workspace lints (`[workspace.lints.rust]`).
- Do not introduce `unsafe` blocks without:
  1. A reproducible benchmark proving that safe Rust is an unacceptable bottleneck (`ISA.md`).
  2. Explicit authorization from the repo maintainer.
  3. A formal `// SAFETY:` block documenting why the invariants hold and how undefined behavior is impossible.
- FFI anchors (such as iOS dead-strip prevention in `dashboard/src-tauri`) remain strictly confined to platform-specific boundary files with minimal footprint.
