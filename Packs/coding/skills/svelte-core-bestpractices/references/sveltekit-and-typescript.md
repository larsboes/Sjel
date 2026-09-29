# SvelteKit 2 & TypeScript Integration

SvelteKit 2 and Svelte 5 establish an end-to-end type-safe architecture spanning server-side loaders, form actions, client navigation, and component props.

---

## 1. Rune-Based `$app/state` vs Legacy `$app/stores`

In SvelteKit 2 (version 2.12+) paired with Svelte 5, the legacy `$app/stores` module is superseded by `$app/state`.

```ts
// ❌ Legacy SvelteKit 1 / Svelte 4 store pattern
import { page, navigating } from '$app/stores';
// In template: {$page.url.pathname}
// In script: get(page).url.pathname

// ✅ Modern SvelteKit 2 + Svelte 5 rune pattern
import { page, navigating } from '$app/state';
// In template: {page.url.pathname}
// In script: page.url.pathname
```

### Key Differences
* **No `$` store prefix**: `page` in `$app/state` is a reactive proxy object. Reading `page.url`, `page.params`, `page.data`, or `page.status` tracks fine-grained dependencies automatically.
* **Universal availability**: Can be read directly inside `.svelte.ts` modules without requiring component context or store subscriptions.
* **`navigating` state**: `navigating.to?.url` and `navigating.from?.url` reflect transitions in real time without store subscriptions.

---

## 2. Typed Load Functions & Route Contracts

SvelteKit automatically generates route types under `./$types` for every directory in `src/routes/`.

### Server Load (`+page.server.ts`)
```ts
import type { PageServerLoad } from './$types';
import { error } from '@sveltejs/kit';

export const load: PageServerLoad = async ({ params, fetch }) => {
  const res = await fetch(`/api/devices/${params.deviceId}`);
  if (!res.ok) {
    throw error(res.status, 'Device not found');
  }

  const device = (await res.json()) as { id: string; name: string; online: boolean };
  return { device };
};
```

### Page Component (`+page.svelte`)
Consume typed load data using `$props()`:

```svelte
<script lang="ts">
  import type { PageData } from './$types';

  // data is strictly typed from the load function return type
  let { data }: { data: PageData } = $props();
</script>

<h1>{data.device.name}</h1>
<p>Status: {data.device.online ? 'Online' : 'Offline'}</p>
```

### Layout Inheritance
Parent layout data cascades down the route tree. To access merged parent data in a child page:
```svelte
<script lang="ts">
  import type { PageData } from './$types';

  let { data }: { data: PageData } = $props();
  // data contains fields from +layout.server.ts AND +page.server.ts
</script>
```

---

## 3. Type-Safe Form Actions & Progressive Enhancement

Form actions provide a robust, JavaScript-optional mutation pipeline.

### Server Actions (`+page.server.ts`)
```ts
import type { Actions } from './$types';
import { fail } from '@sveltejs/kit';

export const actions: Actions = {
  renameDevice: async ({ request, params }) => {
    const formData = await request.formData();
    const name = formData.get('name')?.toString().trim();

    if (!name || name.length < 3) {
      return fail(400, {
        error: 'Name must be at least 3 characters',
        name,
      });
    }

    await db.renameDevice(params.deviceId, name);
    return { success: true };
  },
};
```

### Client Form with `use:enhance` (`+page.svelte`)
```svelte
<script lang="ts">
  import { enhance } from '$app/forms';
  import type { ActionData, PageData } from './$types';

  let { data, form }: { data: PageData; form: ActionData } = $props();
  let isSubmitting = $state(false);
</script>

<form
  method="POST"
  action="?/renameDevice"
  use:enhance={({ cancel }) => {
    isSubmitting = true;

    return async ({ result, update }) => {
      isSubmitting = false;
      // update({ reset: false }) preserves form values if validation failed
      await update();
    };
  }}
>
  <label for="name">Device Name</label>
  <input
    id="name"
    name="name"
    defaultValue={form?.name ?? data.device.name}
    disabled={isSubmitting}
  />

  {#if form?.error}
    <p class="error">{form.error}</p>
  {/if}

  <button type="submit" disabled={isSubmitting}>
    {isSubmitting ? 'Saving...' : 'Save'}
  </button>
</form>
```

---

## 4. Advanced TypeScript Modeling with Runes

### Generic Components (`generics="..."`)
Use the `generics` attribute in `<script lang="ts">` for polymorphic, reusable UI components:

```svelte
<!-- DataTable.svelte -->
<script lang="ts" generics="T extends Record<string, unknown>, K extends keyof T">
  import type { Snippet } from 'svelte';

  interface Props {
    items: T[];
    keyField: K;
    header: Snippet;
    row: Snippet<[T]>;
    empty?: Snippet;
  }

  let { items, keyField, header, row, empty }: Props = $props();
</script>

<table>
  <thead>
    {@render header()}
  </thead>
  <tbody>
    {#each items as item (item[keyField])}
      {@render row(item)}
    {:else}
      {#if empty}
        {@render empty()}
      {/if}
    {/each}
  </tbody>
</table>
```

### Discriminated Union Props
Model mutually exclusive component behaviors cleanly:

```svelte
<!-- ActionButton.svelte -->
<script lang="ts">
  import type { Snippet } from 'svelte';

  type ButtonProps =
    | {
        variant: 'link';
        href: string;
        children: Snippet;
        onclick?: never;
      }
    | {
        variant: 'button';
        href?: never;
        children: Snippet;
        onclick: (event: MouseEvent) => void;
      };

  let props: ButtonProps = $props();
</script>

{#if props.variant === 'link'}
  <a href={props.href} class="btn-link">
    {@render props.children()}
  </a>
{:else}
  <button type="button" onclick={props.onclick} class="btn-action">
    {@render props.children()}
  </button>
{/if}
```

### Snippet Typing Contracts
Always use explicit `Snippet` types rather than `any` or `Component`:

```ts
import type { Snippet } from 'svelte';

// Zero-argument snippet (e.g. children)
let { children }: { children?: Snippet } = $props();

// Parameterized snippet (e.g. item and index)
let { row }: { row: Snippet<[item: T, index: number]> } = $props();
```

### Typed Callback Props vs Legacy Event Dispatchers
Do **not** use `createEventDispatcher` in Svelte 5. Use typed callback properties:

```svelte
<!-- SearchInput.svelte -->
<script lang="ts">
  interface Props {
    value: string;
    placeholder?: string;
    onsearch: (query: string) => void;
    onclear?: () => void;
  }

  let { value = $bindable(''), placeholder = 'Search...', onsearch, onclear }: Props = $props();
</script>

<input
  type="search"
  bind:value
  {placeholder}
  onkeydown={(e) => {
    if (e.key === 'Enter') onsearch(value);
  }}
/>
{#if value && onclear}
  <button onclick={onclear}>✕</button>
{/if}
```

---

## 5. Navigation & Invalidation Architecture

* **Targeted Invalidation**: Avoid blanket `invalidateAll()` when only a specific API endpoint or dependency changed. Use `invalidate(url)` or custom dependency keys:
  ```ts
  // In load function:
  depends('axon:capabilities');

  // In mutation handler:
  import { invalidate } from '$app/navigation';
  await invalidate('axon:capabilities');
  ```
* **Shallow Routing (`pushState`)**: For modals, drawers, or slide-overs that should update browser history without re-running page loaders:
  ```ts
  import { pushState } from '$app/navigation';
  import { page } from '$app/state';

  function openInspector(entityId: string) {
    pushState('', { inspectingId: entityId });
  }
  ```
