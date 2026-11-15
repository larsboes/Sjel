# Adaptive views for the Svelte catalog

## Recommendation

Evaluate json-render behind a Sjel-owned adapter before building a renderer. Keep A2UI as the
alternative when protocol interoperability is required. Defer OpenUI Lang for this slice because
its Svelte runtime lacks general expressions and collection bindings. This recommendation does
not select a production document format or authorize upstream code to access capabilities.

The stopping rung in the build ladder is composition: reuse a renderer, retain Sjel's component
catalog, and implement the host boundary. A new renderer is justified only if measured adapter
work requires replacing the upstream runtime.

The source inspection baseline is 2026-11-13. Claude documentation was checked again on
2026-11-15. Package versions below identify inspected source or releases, not an instruction to
install a floating latest version. No upstream packages were installed or executed for this
comparison. A dependency audit, device acceptance and bundle measurements have not run.

## Sjel constraints and existing work

[Product Views](../website/content/product.md#views) defines views as documents composed from
approved components. [The catalog plan](../docs/plans/gpui.md#views-from-the-web-catalog) owns
the adoption sequence: derived, phone, saved, then live views. Rust owns contracts, data classes,
read authorization and governed writes. A renderer receives authorized data and emits proposal
intents. It receives no arbitrary URL, credential or write-route authority.

[The existing catalog](../apps/catalog/README.md) contains registry-derived lists and details.
Its internal bindings are not a saved-view format. Its current browser gateway reads provide
a browser implementation, not evidence that the same bundle works offline or through MCP Apps.
The adaptive adapter must reuse these components rather than introduce a second object UI.

Model-authored documents and developer-authored executable components have different trust
boundaries. Neither a catalog schema nor an iframe grants permission to read an object or perform
an action. Mail and feed text remains escaped text inside any permitted view.

## Source snapshots

| Project | Inspected source | Version | Direct package license |
| --- | --- | --- | --- |
| json-render | [`5e30a6f`](https://github.com/vercel-labs/json-render/tree/5e30a6f9425cd5ffb5038ba44352ab4b1feb3fc7) | core and Svelte 0.22.0; [release](https://github.com/vercel-labs/json-render/releases/tag/v0.22.0) | Apache-2.0 |
| Community A2UI Svelte renderer | [`d0345ee`](https://github.com/ChaliceForAuri/a2ui-svelte/tree/d0345eeb52c4c84871c82c4f9c0e43f078ac3ffc) | svelte-a2ui 0.2.0; [release](https://github.com/ChaliceForAuri/a2ui-svelte/releases/tag/v0.2.0) | Apache-2.0 |
| Thesys OpenUI Lang | [`4d24e02`](https://github.com/thesysdev/openui/tree/4d24e0205a0110c641d02b9ace168e45ccc58d9c) | Svelte 0.4.2, depending on Lang Core ^0.4.1 | MIT |

These licenses describe the named packages. They do not constitute a transitive license audit.
Thesys OpenUI Lang is distinct from Weights & Biases OpenUI and the Open UI standards group.

## Comparison

| Concern | json-render | A2UI through svelte-a2ui | OpenUI Lang for Svelte |
| --- | --- | --- | --- |
| Ownership | Svelte renderer in the main project | Community renderer separate from the A2UI team | First-party Svelte port with documented feature gaps |
| Document | JSON element map and root, with separate state | Versioned surface lifecycle and component/data update messages | Line-oriented component language parsed into nodes |
| Components | Register existing Svelte components; optional shadcn-svelte catalog | Custom catalogs plus 18 Basic Catalog components | Bring an application component library |
| Binding | JSON Pointer state, repeats, visibility and two-way bindings | Surface data model, collection templates and two-way bindings | Form fields; no general expressions, Each, references or variable state |
| Streaming | Document patches; independent controlled-store updates | Independent component and data-model messages; forward references | Accumulated text is reparsed while streaming |
| Validation | Catalog schemas and structural issues identifying elements | Structural protocol errors and some paths; custom props need validation | Parser metadata through onParseResult; no dedicated onError prop |
| Failure display | Unknown nodes, missing children and render exceptions can disappear | Unknown/pending nodes use placeholders; depth limit bounds recursion | Parse or component failures can produce empty output |
| Model-free use | Supplied specs and state require no model | Host can supply messages without a model | Saved text can render without a model |
| Offline and WKWebView | Plausible with host state; unverified on device | Plausible with custom transport; unverified on device | Plausible without generation service; unverified on device |

Sources: json-render [Svelte API](https://json-render.dev/docs/api/svelte),
[element renderer](https://github.com/vercel-labs/json-render/blob/5e30a6f9425cd5ffb5038ba44352ab4b1feb3fc7/packages/svelte/src/ElementRenderer.svelte),
[structural validator](https://github.com/vercel-labs/json-render/blob/5e30a6f9425cd5ffb5038ba44352ab4b1feb3fc7/packages/core/src/spec-validator.ts);
A2UI [catalog interfaces](https://github.com/ChaliceForAuri/a2ui-svelte/blob/d0345eeb52c4c84871c82c4f9c0e43f078ac3ffc/src/lib/catalog/types.ts),
[node renderer](https://github.com/ChaliceForAuri/a2ui-svelte/blob/d0345eeb52c4c84871c82c4f9c0e43f078ac3ffc/src/lib/render/Node.svelte),
[reducer](https://github.com/ChaliceForAuri/a2ui-svelte/blob/d0345eeb52c4c84871c82c4f9c0e43f078ac3ffc/src/lib/protocol/reducer.ts);
OpenUI [Svelte API](https://www.openui.com/docs/api-reference/svelte-lang) and
[renderer](https://github.com/thesysdev/openui/blob/4d24e0205a0110c641d02b9ace168e45ccc58d9c/packages/svelte-lang/src/lib/Renderer.svelte).

## Why json-render leads

The Svelte renderer accepts a supplied spec and registry. Its
[state provider](https://github.com/vercel-labs/json-render/blob/5e30a6f9425cd5ffb5038ba44352ab4b1feb3fc7/packages/svelte/src/contexts/StateProvider.svelte)
accepts an external store. This is the useful integration point: Rust supplies authorized
snapshots while a constrained client store holds local presentation state and form drafts.
The renderer's network-generating chat helpers are optional and do not belong in the offline
bundle. The inspected core manifest depends on Zod; the Svelte package depends on core and
declares Svelte and Zod peers. There is no need to adopt an AI SDK to render supplied documents.

The optional shadcn-svelte package adds Tailwind and several UI dependencies. Its component
count is not an adoption argument. Sjel can register its existing components, including its own
tables and charts, without adopting that visual system or dependency set.

The default
[action provider](https://github.com/vercel-labs/json-render/blob/5e30a6f9425cd5ffb5038ba44352ab4b1feb3fc7/packages/svelte/src/contexts/ActionProvider.svelte)
handles built-in state actions before registered handlers. An empty action-handler map does not
disable setState, pushState, removeState, navigation-state actions or form-validation writes.
Sjel must restrict document actions and writable state paths. Root replacement and writes to
host snapshots must fail. Proposal submission remains an explicitly registered host operation.

The structural validator is useful but does not provide Sjel authorization, a complete catalog
policy, or an accepted resource budget. The inspected element renderer recursively follows child
references without an explicit depth guard. Rust must reject cycles, excessive depth, oversized
documents, unknown components and invalid props before the client renders them. Saved documents
must not silently drop invalid nodes or automatically apply lossy repairs.

## Where A2UI wins

A2UI is preferable when Sjel must receive documents from independently implemented A2UI agents.
The community Svelte runtime already implements protocol lifecycle, separate data updates,
forward references, function calls and a rendering-depth limit. Sjel can register domain-specific
components for tables and charts while reusing the protocol engine.

Accessibility needs component-level evidence. The inspected
[modal](https://github.com/ChaliceForAuri/a2ui-svelte/blob/d0345eeb52c4c84871c82c4f9c0e43f078ac3ffc/src/lib/catalog/basic/Modal.svelte)
uses native dialog behavior. The
[tabs](https://github.com/ChaliceForAuri/a2ui-svelte/blob/d0345eeb52c4c84871c82c4f9c0e43f078ac3ffc/src/lib/catalog/basic/Tabs.svelte)
lack arrow-key handling, roving tabindex and explicit tab/panel associations. Existing
[browser tests](https://github.com/ChaliceForAuri/a2ui-svelte/blob/d0345eeb52c4c84871c82c4f9c0e43f078ac3ffc/tests/browser/containers.test.ts)
exercise switching and opening. Screen-reader acceptance still needs a device check. Runtime
reuse leaves Sjel free to supply its own controls.

The svelte-a2ui manifest declares no runtime dependency on the official web_core package.
Security advisories about web_core cannot be transferred to this separate implementation
without tracing the affected code. No vulnerability verdict is established by this comparison.

## OpenUI and interoperability corrections

OpenUI's documented Svelte gaps concern the required runtime behavior. A compact syntax alone
does not outweigh missing reactive expressions and collection support. Token and latency gains
remain unmeasured on Sjel documents.

Current OpenUI
[telemetry documentation](https://github.com/thesysdev/openui/blob/4d24e0205a0110c641d02b9ace168e45ccc58d9c/docs/content/docs/openui-lang/telemetry.mdx)
distinguishes default-on installation telemetry from opt-in server runtime telemetry; browser
calls do not send runtime telemetry. Applicability to the older Lang Core version selected by
the Svelte package needs an exact-version check. Telemetry is not evidence that the browser
renderer requires a cloud service.

json-render's
[A2UI integration page](https://github.com/vercel-labs/json-render/blob/5e30a6f9425cd5ffb5038ba44352ab4b1feb3fc7/apps/web/content/docs/a2ui.mdx)
labels its example illustrative. It shows a custom React renderer and older A2UI message shapes.
A Svelte adapter would require additional implementation and verification. Thesys'
[A2UI adapter](https://www.openui.com/docs/api-reference/a2ui) deliberately changes component
payloads to OpenUI Lang statements. Neither establishes drop-in wire compatibility.

## How Claude web and Desktop host UI

Claude has several UI paths; they must not be treated as one renderer architecture.

| Surface | Publicly documented mechanism | Consequence for Sjel |
| --- | --- | --- |
| Interactive connectors | Developer-supplied MCP Apps with host-mediated communication | Reuse this embedding boundary for a developer-built Svelte bundle |
| Custom chat visuals | Model-generated HTML and SVG on web and Desktop | This follows an executable-output policy that Sjel does not permit for adaptive views |
| Claude Design previews | Sandboxed iframe on a separate content domain | Isolation is useful prior art; it does not replace Sjel contract authorization |

Anthropic's [MCP Apps guide](https://claude.com/docs/connectors/building/mcp-apps/getting-started)
documents local examples in Claude Desktop. Its expanded app tools, entry points and embedded
websites require a remote connector; directory features can also depend on listing status.
Local Desktop support does not make a private loopback service reachable from Claude web.
Exposing Sjel through a tunnel requires a separate deployment decision.

The guide documents App.connect and a host-specific sandbox origin through ui.domain. Public
documentation does not identify A2UI, json-render or OpenUI Lang as Claude's general runtime.
The implementation framework and reuse between Claude's closed-source web and Desktop hosts
remain unverified.

Sources: Anthropic's [interactive-connector announcement](https://claude.com/resources/articles/interactive-connectors-full-tool),
[HTML/SVG visuals explanation](https://support.claude.com/en/articles/9002504-can-claude-produce-images),
and [Design isolation guide](https://support.claude.com/en/articles/14604406-claude-design-admin-guide-for-team-and-enterprise-plans).

MCP Apps supplies the packaging and host protocol. Sjel can ship one
developer-authored bundle whose input remains a validated view document. The
[ext-apps license file](https://github.com/modelcontextprotocol/ext-apps/blob/main/LICENSE)
explains an MIT-to-Apache transition; packaging requires the notices for the exact adopted
revision. json-render's MCP app integration is React-oriented, so a direct Svelte integration
with the standard SDK needs evaluation rather than assumed parity.

## Prepared store boundary

[client-state.ts](../apps/catalog/adapter/client-state.ts) provides a dependency-free store
wrapper matching the inspected StateStore method surface. Client writes are limited to /draft
and /ui JSON Pointers. It validates every path before delegating a batch, rejects prototype
traversal, detaches caller-owned write values, and freezes returned records and arrays. The
host retains the underlying immutable store and can publish replacement snapshots.

This wrapper requires a JSON-only immutable host store. Freezing shared reads makes mutation
invalid for host code too; host updates must replace objects. It is not a document validator,
Rust authorization layer, renderer, or proposal gateway. It does not filter model context or
grant network access. The default json-render navigation and validation destinations remain
unavailable unless document validation explicitly maps them into local UI state.

The wrapper imports no upstream runtime. Source-level interface matching is not a successful
integration test. Production entry points, package manifests and lockfiles do not import it.

Run the isolated checks from the repository root:

```sh
bun --no-env-file test apps/catalog/adapter/client-state.test.ts
apps/catalog/node_modules/.bin/tsc --noEmit --strict --target ES2022 --module ESNext --moduleResolution bundler --lib ES2022,DOM --skipLibCheck apps/catalog/adapter/client-state.ts
```

The acceptance tests use fictional records. They cover local writes, refused snapshot/root
writes, malformed and prototype pointers, mixed-batch refusal, read immutability, input aliasing,
host refresh, subscriptions and optional server snapshots. They do not run a real renderer.

## Adoption gate

Evaluate json-render and A2UI against the same synthetic list/detail/form scenario:

1. Derive and save a view without a model. Reload it without generation or network access.
2. Refresh authorized data independently of the document while retaining local form drafts.
3. Reject root replacement, snapshot mutation, write-route bindings, unknown components,
   invalid props, cycles and oversized inputs with errors naming the failing node.
4. Submit only a typed proposal. The Rust capability decides authorization and write mode;
   labels, confirmation controls and client validation confer no additional authority.
5. Render hostile source strings as text, then check keyboard navigation, focus after updates,
   VoiceOver and offline behavior on the target iPhone WKWebView and an MCP Apps host.

Before installing a candidate, record its scoped verdict in upstreams.toml, inspect the exact
dependency graph and execution surface, pin versions and preserve license notices. Reuse the
Rust registry as the catalog authority rather than maintain a second handwritten field schema.
Measure bundle size, large-list behavior and update cost after both candidates pass correctness.

Choose A2UI if required interoperability outweighs its extra protocol surface. Reconsider
OpenUI when its Svelte runtime supplies the necessary binding model and measurable generation
savings. Build a renderer only if the bounded acceptance experiment demonstrates that adapting
the existing runtimes replaces their core. Device tests, host interoperation, dependency audit,
upstream-store integration and a production document profile remain unverified.
