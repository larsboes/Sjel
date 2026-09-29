---
name: svelte-core-bestpractices
description: Guidance on writing fast, robust, modern Svelte 5 code including runes, snippets, event handling, and stores migration. Use when authoring, editing, or auditing Svelte 5 components (.svelte) or reactive modules (.svelte.ts). Do not use for backend Rust APIs, non-Svelte scripts, or prose writing.
---

## `$state`

Only use the `$state` rune for variables that should be _reactive_ — in other words, variables that cause an `$effect`, `$derived` or template expression to update. Everything else can be a normal variable.

Objects and arrays (`$state({...})` or `$state([...])`) are made deeply reactive, meaning mutation will trigger updates. This has a trade-off: in exchange for fine-grained reactivity, the objects must be proxied, which has performance overhead. In cases where you're dealing with large objects that are only ever reassigned (rather than mutated), use `$state.raw` instead. This is often the case with API responses, for example.

## `$derived`

To compute something from state, use `$derived` rather than `$effect`:

```js
// do this
let square = $derived(num * num);

// don't do this
let square;

$effect(() => {
	square = num * num;
});
```

> [!NOTE] `$derived` is given an expression, _not_ a function. If you need to use a function (because the expression is complex, for example) use `$derived.by`.

Deriveds are writable — you can assign to them, just like `$state`, except that they will re-evaluate when their expression changes.

If the derived expression is an object or array, it will be returned as-is — it is _not_ made deeply reactive. You can, however, use `$state` inside `$derived.by` in the rare cases that you need this.

## `$effect`

Effects are an escape hatch and should mostly be avoided. In particular, avoid updating state inside effects.

- If you need to sync state to an external library such as D3, it is often neater to use [`{@attach ...}`](references/attach.md)
- If you need to run some code in response to user interaction, put the code directly in an event handler or use a [function binding](references/bind.md) as appropriate
- If you need to log values for debugging purposes, use [`$inspect`](references/inspect.md)
- If you need to observe something external to Svelte, use [`createSubscriber`](references/svelte-reactivity.md)

Never wrap the contents of an effect in `if (browser) {...}` or similar — effects do not run on the server.

## `$props`

Treat props as though they will change. For example, values that depend on props should usually use `$derived`:

```js
// @errors: 2451
let { type } = $props();

// do this
let color = $derived(type === 'danger' ? 'red' : 'green');

// don't do this — `color` will not update if `type` changes
let color = type === 'danger' ? 'red' : 'green';
```

## `$inspect.trace`

`$inspect.trace` is a debugging tool for reactivity. If something is not updating properly or running more than it should you can add `$inspect.trace(label)` as the first line of an `$effect` or `$derived.by` (or any function they call) to trace their dependencies and discover which one triggered an update.

## Events

Any element attribute starting with `on` is treated as an event listener:

```svelte
<button onclick={() => {...}}>click me</button>

<!-- attribute shorthand also works -->
<button {onclick}>...</button>

<!-- so do spread attributes -->
<button {...props}>...</button>
```

If you need to attach listeners to `window` or `document` you can use `<svelte:window>` and `<svelte:document>`:

```svelte
<svelte:window onkeydown={...} />
<svelte:document onvisibilitychange={...} />
```

Avoid using `onMount` or `$effect` for this.

## Snippets

[Snippets](references/snippet.md) are a way to define reusable chunks of markup that can be instantiated with the [`{@render ...}`](references/render.md) tag, or passed to components as props. They must be declared within the template.

```svelte
{#snippet greeting(name)}
	<p>hello {name}!</p>
{/snippet}

{@render greeting('world')}
```

> [!NOTE] Snippets declared at the top level of a component (i.e. not inside elements or blocks) can be referenced inside `<script>`. A snippet that doesn't reference component state is also available in a `<script module>`, in which case it can be exported for use by other components.

## Each blocks

Prefer to use [keyed each blocks](references/each.md) — this improves performance by allowing Svelte to surgically insert or remove items rather than updating the DOM belonging to existing items.

> [!NOTE] The key _must_ uniquely identify the object. Do not use the index as a key.

Avoid destructuring if you need to mutate the item (with something like `bind:value={item.count}`, for example).

## Using JavaScript variables in CSS

If you have a JS variable that you want to use inside CSS you can set a CSS custom property with the `style:` directive.

```svelte
<div style:--columns={columns}>...</div>
```

You can then reference `var(--columns)` inside the component's `<style>`.

## Styling child components

The CSS in a component's `<style>` is scoped to that component. If a parent component needs to control the child's styles, the preferred way is to use CSS custom properties:

```svelte
<!-- Parent.svelte -->
<Child --color="red" />

<!-- Child.svelte -->
<h1>Hello</h1>

<style>
	h1 {
		color: var(--color);
	}
</style>
```

If this is impossible (for example, the child component comes from a library) you can use `:global` to override styles:

```svelte
<div>
	<Child />
</div>

<style>
	div :global {
		h1 {
			color: red;
		}
	}
</style>
```

## Context

Consider using context instead of declaring state in a shared module. This will scope the state to the part of the app that needs it, and eliminate the possibility of it leaking between users when server-side rendering.

Use `createContext` rather than `setContext` and `getContext`, as it provides type safety.

## Async Svelte

If using version 5.36 or higher, you can use [await expressions](references/await-expressions.md) and [hydratable](references/hydratable.md) to use promises directly inside components. Note that these require the `experimental.async` option to be enabled in `svelte.config.js` as they are not yet considered fully stable.

## Avoid legacy features

Always use runes mode for new code, and avoid features that have more modern replacements:

- use `$state` instead of implicit reactivity (e.g. `let count = 0; count += 1`)
- use `$derived` and `$effect` instead of `$:` assignments and statements (but only use effects when there is no better solution)
- use `$props` instead of `export let`, `$$props` and `$$restProps`
- use `onclick={...}` instead of `on:click={...}`
- use `{#snippet ...}` and `{@render ...}` instead of `<slot>` and `$$slots` and `<svelte:fragment>`
- use `<DynamicComponent>` instead of `<svelte:component this={DynamicComponent}>`
- use `import Self from './ThisComponent.svelte'` and `<Self>` instead of `<svelte:self>`
- use classes with `$state` fields to share reactivity between components, instead of using stores (see [stores-to-runes migration](references/stores-to-runes-migration.md))
- use `{@attach ...}` instead of `use:action`
- use clsx-style arrays and objects in `class` attributes, instead of the `class:` directive

## Stores-to-Runes Migration & `.svelte.ts` Singletons

Avoid legacy Svelte stores (`writable`, `readable`, `derived`, `get(store)`) in new code. Define reactive singleton classes in `.svelte.ts` modules with public `$state` fields and getters for derived state. For resource-intensive sources (polling, SSE), implement reference-counted lifecycle tracking (`#subscribers`).

See [stores-to-runes-migration.md](references/stores-to-runes-migration.md) for full patterns, legacy interop adapters, and `createSubscriber` examples.

## SvelteKit 2 & Full-Stack TypeScript

When working in SvelteKit applications:
- Use `import { page } from '$app/state'` instead of the legacy `$app/stores` store. Access `page.url`, `page.data`, and `page.params` directly as reactive properties.
- Type page load contracts strictly using `./$types` (`PageServerLoad`, `PageData`).
- Implement form actions with progressive enhancement using `use:enhance` and `fail()` error returns.
- Model generic components with `<script lang="ts" generics="T, K extends keyof T">` and type snippet props using `Snippet<[T]>`.

See [sveltekit-and-typescript.md](references/sveltekit-and-typescript.md) for complete routing, form action, and generic component examples.

## API & State Orchestration

For dashboards communicating with local daemons (e.g. Axum HTTP loopback, Tauri IPC):
- **Last-known-good retention**: Never clear state arrays when a poll or network request fails; set an `offline = true` flag and preserve cached items to prevent UI flashing.
- **`$state.raw` for high-frequency feeds**: Avoid deep proxy overhead on rapid telemetry, log streams, or large analytical collections.
- **Optimistic updates**: Apply user mutations to `$state` immediately, with try/catch rollback if the backend call fails.

See [api-and-state-orchestration.md](references/api-and-state-orchestration.md) for SSE reconnection, Tauri IPC abstraction, and optimistic mutation patterns.

---

## Architectural References

- [attach.md](references/attach.md) - Using `{@attach ...}` instead of `use:action`
- [await-expressions.md](references/await-expressions.md) - Async await expressions in templates
- [bind.md](references/bind.md) - Function bindings
- [each.md](references/each.md) - Keyed each blocks
- [hydratable.md](references/hydratable.md) - SSR hydration boundaries
- [inspect.md](references/inspect.md) - Debugging reactivity with `$inspect`
- [render.md](references/render.md) - Template snippet rendering with `{@render ...}`
- [snippet.md](references/snippet.md) - Snippet syntax and composition
- [svelte-reactivity.md](references/svelte-reactivity.md) - Reactivity primitives and subscribers
- [stores-to-runes-migration.md](references/stores-to-runes-migration.md) - Migrating stores to `.svelte.ts` classes and singletons
- [sveltekit-and-typescript.md](references/sveltekit-and-typescript.md) - SvelteKit 2 routing, `$app/state`, typed loads, and generic components
- [api-and-state-orchestration.md](references/api-and-state-orchestration.md) - Local API loopback, `$state.raw` telemetry, and fault-tolerant caching

