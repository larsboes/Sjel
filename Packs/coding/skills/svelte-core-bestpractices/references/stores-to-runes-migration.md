# Stores-to-Runes Migration & Universal Reactivity in `.svelte.ts`

Svelte 5 replaces the legacy `svelte/store` API (`writable`, `readable`, `derived`, `get()`) with universal reactivity powered by runes (`$state`, `$derived`, `$effect.root`). Reactivity is no longer confined to `.svelte` component files or tied to Svelte store subscription protocols: any `.svelte.ts` or `.svelte.js` module can define and export reactive state.

---

## 1. Why Migrate from Stores to Runes

| Legacy Svelte 3/4 Stores | Svelte 5 Runes in `.svelte.ts` |
| :--- | :--- |
| **Object cloning on mutation**: `store.update(s => ({ ...s, val: 42 }))` triggers whole-store notifications. | **Fine-grained mutation**: `state.val = 42` triggers updates *only* for consumers reading `val`. |
| **Subscription management**: Manual subscriptions require unsubscription tracking (`onDestroy`, `unsubscribe()`) to avoid memory leaks. | **Automatic dependency tracking**: Runes track dependencies during execution. No manual cleanup for reads. |
| **Store syntax friction**: Must prefix with `$` in components (`$user`), but cannot use `$` in standard TS files, forcing `get(store)`. | **Transparent JavaScript syntax**: Normal property access (`user.name`), standard methods, and getters/setters. |
| **Derived boilerplate**: `derived([a, b, c], ([$a, $b, $c]) => ...)` with complex tuple destructuring. | **Native getters and `$derived`**: `get fullName() { return `${this.first} ${this.last}`; }`. |

---

## 2. Core Migration Patterns

### Pattern A: Simple Mutable Store $\to$ Class Singleton

#### Legacy Store (`store.ts`)
```ts
import { writable } from 'svelte/store';

export interface UserProfile {
  id: string;
  name: string;
  avatarUrl: string;
}

export const currentUser = writable<UserProfile | null>(null);

export function setUserName(name: string) {
  currentUser.update(user => user ? { ...user, name } : null);
}
```

#### Svelte 5 Equivalent (`user.svelte.ts`)
```ts
export interface UserProfile {
  id: string;
  name: string;
  avatarUrl: string;
}

class UserStore {
  current = $state<UserProfile | null>(null);

  setName(name: string): void {
    if (this.current) {
      this.current.name = name; // Fine-grained in-place mutation
    }
  }

  clear(): void {
    this.current = null;
  }
}

export const userStore = new UserStore();
```

In any Svelte 5 component:
```svelte
<script lang="ts">
  import { userStore } from '$lib/user.svelte';
</script>

{#if userStore.current}
  <input bind:value={userStore.current.name} />
  <button onclick={() => userStore.clear()}>Sign Out</button>
{/if}
```

---

### Pattern B: Derived Stores $\to$ Class Getters or `$derived.by`

In Svelte 5 class instances, standard TypeScript getters that read `$state` properties act as reactive derived values automatically.

#### Legacy Derived Store
```ts
import { derived } from 'svelte/store';
import { items, filter } from './stores';

export const visibleItems = derived(
  [items, filter],
  ([$items, $filter]) => $items.filter(item => item.category === $filter)
);
```

#### Svelte 5 Class Getters
```ts
// items.svelte.ts
class ItemCatalog {
  items = $state<Item[]>([]);
  activeFilter = $state<string>('all');

  // Automatically behaves like $derived: only re-evaluates when items or activeFilter change
  get visible(): Item[] {
    if (this.activeFilter === 'all') return this.items;
    return this.items.filter(i => i.category === this.activeFilter);
  }

  get count(): number {
    return this.visible.length;
  }
}

export const catalog = new ItemCatalog();
```

> [!TIP]
> Use getters for derived calculations in classes. If your derived logic requires non-class module scope or explicit memoization across complex closures, use `$derived.by(() => { ... })`.

---

### Pattern C: Resource Lifecycle & Subscription Reference Counting

When a store needs to run a background poll, open a WebSocket, or listen to an event stream *only while components are using it*, use subscription reference counting.

```ts
// capabilities.svelte.ts
import { fetchCapabilities, type CapabilityView } from './api';

class CapabilityStore {
  items = $state<CapabilityView[]>([]);
  offline = $state(false);
  loading = $state(true);

  #subscribers = 0;
  #timer: ReturnType<typeof setInterval> | undefined;

  async refresh(): Promise<void> {
    try {
      this.items = await fetchCapabilities();
      this.offline = false;
    } catch {
      this.offline = true; // Retain last known state to prevent empty flicker
    } finally {
      this.loading = false;
    }
  }

  /**
   * Mount hook for consuming components: starts background polling on first subscriber,
   * stops when the last subscriber unmounts.
   */
  subscribe(intervalMs = 15_000): () => void {
    this.#subscribers += 1;
    if (this.#subscribers === 1) {
      void this.refresh();
      this.#timer = setInterval(() => void this.refresh(), intervalMs);
    }

    return () => {
      this.#subscribers -= 1;
      if (this.#subscribers === 0 && this.#timer !== undefined) {
        clearInterval(this.#timer);
        this.#timer = undefined;
      }
    };
  }

  byName(name: string): CapabilityView | undefined {
    return this.items.find(c => c.name === name);
  }
}

export const capabilities = new CapabilityStore();
```

Usage in components:
```svelte
<script lang="ts">
  import { onMount } from 'svelte';
  import { capabilities } from '$lib/capabilities.svelte';

  onMount(() => capabilities.subscribe(10_000));
</script>

{#each capabilities.items as cap (cap.name)}
  <p>{cap.name} - {cap.status}</p>
{/each}
```

---

### Pattern D: Integrating External Event Sources with `createSubscriber`

For subscribing to external browser events (e.g. `online`/`offline`, window dimensions, media queries) within Svelte's reactive graph without full class lifecycle overhead, use `createSubscriber`:

```ts
// network.svelte.ts
import { createSubscriber } from 'svelte/reactivity';

export class NetworkStatus {
  #subscribe = createSubscriber((update) => {
    window.addEventListener('online', update);
    window.addEventListener('offline', update);

    return () => {
      window.removeEventListener('online', update);
      window.removeEventListener('offline', update);
    };
  });

  get isOnline(): boolean {
    this.#subscribe();
    return typeof navigator !== 'undefined' ? navigator.onLine : true;
  }
}

export const network = new NetworkStatus();
```

---

## 3. Interoperability with Legacy Svelte Stores

When working with third-party libraries (e.g. legacy UI packages) that still produce or require Svelte stores:

### Consuming a Legacy Store in a Component
In `.svelte` files, the `$` prefix still works for legacy stores:
```svelte
<script lang="ts">
  import { legacyStore } from 'external-library';
</script>

<p>Value: {$legacyStore}</p>
```

### Consuming a Legacy Store in a `.svelte.ts` Module
Use `fromStore` from `svelte/store` to convert a store into a reactive rune object:
```ts
import { fromStore } from 'svelte/store';
import { legacyStore } from 'external-library';

export class BridgeAdapter {
  #storeRune = fromStore(legacyStore);

  get current() {
    return this.#storeRune.current;
  }
}
```

### Exposing Rune State to Legacy Store Consumers
If an external API requires a `Readable<T>` or `Writable<T>` store, create a thin adapter:
```ts
import type { Readable } from 'svelte/store';

export function toReadableStore<T>(getter: () => T): Readable<T> {
  return {
    subscribe(callback: (value: T) => void) {
      // Use $effect.root to listen to rune updates outside components
      const cleanup = $effect.root(() => {
        $effect(() => {
          callback(getter());
        });
      });
      return cleanup;
    }
  };
}
```

---

## 4. Key Pitfalls & Rules

1. **Only `.svelte.ts` and `.svelte.js` files can use runes**: If you place `$state` or `$derived` inside a standard `.ts` file, the Svelte compiler will reject it.
2. **Never wrap state reads in `$effect` just to assign another variable**: Always use `$derived` or class getters.
3. **Use `$state.raw` for large, read-heavy arrays/objects**: If an API returns thousands of records that are replaced rather than mutated item-by-item, `$state.raw(...)` eliminates proxy wrapping costs.
