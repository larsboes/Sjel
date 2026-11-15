# json-render evaluation admission

## Scope and verdict

Evaluate @json-render/core 0.22.0 and @json-render/svelte 0.22.0 with Zod 4.3.6 and the catalog's
existing Svelte 5.57.2. The evaluation uses fictional data, disables lifecycle scripts and
excludes chat helpers, the experimental gateway evaluator and upstream component catalogs.
Production adoption remains subject to the central upstream register and Rust document admission.

The audited source is release v0.22.0 at commit
[`1819412`](https://github.com/vercel-labs/json-render/tree/18194121c8f087afacd9a8ef65fbda18374ee039).
The shallow checkout was inspected without running its build, tests or installation scripts.
The npm archives for the two packages and Zod were unpacked separately and inspected as text.
This review covers the selected renderer path, not every package in the upstream monorepo.

## Findings

| Severity | Evidence | Evaluation condition |
| --- | --- | --- |
| Boundary | `packages/svelte/src/contexts/ActionProvider.svelte:185` handles `resolved.action === "setState"` before custom handlers | The Sjel store guard refuses writes outside local namespaces; documents cannot supply actions |
| Boundary | `packages/svelte/src/streaming.svelte.ts:248` and `:436` call `fetch(api, ...)` | Neither chat helper is imported by the evaluation entry point |
| Boundary | `packages/core/src/experimental-evaluator.ts:66` names `https://ai-gateway.vercel.sh/v4/ai/evaluation-model` | The evaluator is excluded; the client CSP refuses connections |
| Resource | `packages/svelte/src/ElementRenderer.svelte` recursively follows component references | Admission bounds bytes, nodes and depth and rejects repeated references before rendering |
| Supply chain | `.github/workflows/release.yml` uses actions such as `actions/checkout@v4` and publishes through CI | Exact registry versions and archive integrity are pinned; action tags are not immutable provenance |

Line references are relative to the pinned release checkout. The searched npm distribution paths
are `core/package/dist`, `svelte/package/dist` and `zod/package`. A search for `postinstall`,
`preinstall`, `eval(`, `new Function`, `child_process`, `sendBeacon`, `localStorage`,
`sessionStorage`, `indexedDB`, `posthog` and `sentry` found no occurrences in runtime code after
excluding maps and Markdown. The fetch occurrences correspond to the three optional paths above.
This bounded search found no author-only telemetry or downloaded-code execution path.

## Dependency and execution surface

The published Svelte package depends exactly on core 0.22.0 and declares Svelte and Zod peers.
Core depends on Zod ^4.3.6. Zod 4.3.6 declares no runtime dependencies. The evaluation pins Zod
and Svelte directly, so no new UI kit, model SDK, native extension or postinstall binary is
required. Package manifests declare build/development commands but no install lifecycle hook.
Installation uses `--ignore-scripts` regardless.

Registry metadata advertises SLSA provenance for both json-render packages. The core npm
archive's SHA-512 was independently recomputed and matched registry integrity metadata.
Provenance signatures, branch protections and maintainer account security were not verified.

The selected renderer uses process memory for spec, snapshot and draft state. No filesystem or
credential API was found in the selected runtime path. Installation creates a local lockfile
and node_modules; builds create ignored dist files. The fixture server serves only static local
assets and accepts GET requests. It has no capability proxy or credentials.

## Limits

- The shallow checkout does not establish maintainer history or account security.
- The registry archive comparison does not establish reproducible source-to-package builds.
- Existing approved Svelte/Vite tooling is reused; this review does not repeat its audit.
- Device isolation, WKWebView behavior and third-party host enforcement require separate checks.
- This is evaluation admission. It does not approve a production view format or private-data use.
