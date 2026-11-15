# Object catalog

A read-only Svelte catalog derived from Rust object declarations. Projects is the first
registry; the catalog has no capability-specific pages. `projects-api::registry`
owns discovery, identity, classification inheritance, links and governed action metadata.
The internal read bindings are not a saved or adaptive view-document format.

## Build and verify

```sh
bun install --frozen-lockfile --ignore-scripts
bun pm scan
bun run generate
bun run check
bun run build
bun generate.ts --check
bun run fixture
bun run browser-check
```

Generation calls `sjel cargo` and requires the configured Rust build path. The node serves
`apps/catalog/dist` at `/catalog/` behind the existing gateway access and origin checks.
`bun run fixture` starts a loopback preview with fictional records, independent of live services.
All production data fetches use approved registry reads through that gateway; credentials never enter
the bundle. The bundle loads no external scripts, fonts or styles. A standalone development
server has no data proxy; use the Rust node to exercise authenticated reads.

`bun run browser-check` launches a Rust test fixture with the production static-asset handler
and fictional classified reads. It uses an isolated headless Chrome profile. An installed Chrome
channel is the default; `SJEL_BROWSER_EXECUTABLE` selects an existing compatible executable.
It never installs a browser. Screenshots, when the browser starts, stay in
`dist/acceptance/`. The fixture shuts down on completion and has a bounded lifetime.
Normal output cleanup can fail with `EPERM` on restricted hosts; `bun run build --emptyOutDir false`
verifies compilation without removing generated directories. That option can retain obsolete assets.

## Why this shape

The list uses identity keys, a declared inactive-property role, pagination and native buttons. Detail properties derive from the
wire schema, inherit the object's class and render as escaped text. Linked lists request
`content=false`; the single-record detail fetches content as plain text. Omitted properties stay omitted. Project membership links filter the aggregate
read by archived state because the existing read selects by name alone. Inventory reference
links are not projected by these read contracts and are not fabricated by the catalog.

Svelte, its official Vite adapter, Vite, TypeScript, svelte-check and Playwright are pinned in `package.json`
and resolved in `bun.lock`; adoption scope is recorded in `../../upstreams.toml`. Install
scripts remain disabled. The schema-to-TypeScript lowering deliberately rejects unsupported
schema forms instead of weakening them to `any`.
