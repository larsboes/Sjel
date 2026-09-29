# API & State Orchestration in Svelte 5

Modern systems-connected web applications (such as Sjel dashboards communicating with local Rust Axum daemons, Tauri desktop bridges, and real-time SSE telemetry) require resilient state orchestration. This guide establishes architectural patterns for fault-tolerant state, zero-overhead streaming, and hybrid IPC/HTTP communication.

---

## 1. Fault-Tolerant State & Last-Known-Good Principle

### The Empty Shell Antipattern
When an internal capability or backend daemon restarts, a single failed network poll must **never** clear UI state or flash empty screens. A stale UI with an "offline" indicator is strictly superior to a disappearing interface that breaks user focus.

```ts
// capabilities.svelte.ts
import { fetchCapabilities, type CapabilityView } from './api';

class CapabilityStore {
  items = $state<CapabilityView[]>([]);
  offline = $state(false);
  loading = $state(true);

  async refresh(): Promise<void> {
    try {
      this.items = await fetchCapabilities();
      this.offline = false;
    } catch {
      // Daemon is temporarily down or restarting. Keep last known items!
      this.offline = true;
    } finally {
      this.loading = false;
    }
  }
}

export const capabilities = new CapabilityStore();
```

In the component:
```svelte
<script lang="ts">
  import { capabilities } from '$lib/capabilities.svelte';
</script>

{#if capabilities.offline}
  <aside class="warning-banner">Capability daemon offline. Showing cached status.</aside>
{/if}

{#if capabilities.loading && capabilities.items.length === 0}
  <p>Connecting...</p>
{:else}
  <ul>
    {#each capabilities.items as cap (cap.name)}
      <li>{cap.name}: {cap.status}</li>
    {/each}
  </ul>
{/if}
```

---

## 2. High-Throughput Telemetry with `$state.raw`

### Why `$state` Proxies Hurt for High-Frequency Feeds
`$state(...)` recursively wraps objects in Javascript Proxies to enable fine-grained mutation tracking. For high-frequency telemetry, log streams, or large analytical arrays (e.g. 5,000+ items arriving every 100ms), proxy creation induces severe garbage collection thrashing.

### The Solution: `$state.raw`
Use `$state.raw` for data that is replaced as a whole rather than mutated property-by-property:

```ts
// telemetry.svelte.ts
export interface LogEntry {
  id: string;
  timestamp: string;
  level: 'info' | 'warn' | 'error';
  message: string;
}

class TelemetryStore {
  // $state.raw wraps the array without proxying elements
  entries = $state.raw<LogEntry[]>([]);
  maxEntries = 1000;

  append(batch: LogEntry[]): void {
    // Reassignment triggers reactivity with zero proxy overhead
    const combined = [...this.entries, ...batch];
    this.entries = combined.length > this.maxEntries 
      ? combined.slice(combined.length - this.maxEntries) 
      : combined;
  }

  clear(): void {
    this.entries = [];
  }
}

export const telemetry = new TelemetryStore();
```

---

## 3. Resilient SSE (Server-Sent Events) Stream Lifecycle

Real-time feeds from Axum backend channels must gracefully handle network breaks and prevent orphan connections when components unmount.

```ts
// stream.svelte.ts
class RealtimeFeed {
  connected = $state(false);
  lastMessage = $state<string | null>(null);

  #eventSource: EventSource | null = null;
  #subscribers = 0;
  #retryDelay = 1000;

  connect(): () => void {
    this.#subscribers += 1;
    if (this.#subscribers === 1) {
      this.#initStream();
    }

    return () => {
      this.#subscribers -= 1;
      if (this.#subscribers === 0 && this.#eventSource) {
        this.#eventSource.close();
        this.#eventSource = null;
        this.connected = false;
      }
    };
  }

  #initStream(): void {
    this.#eventSource = new EventSource('/api/events');

    this.#eventSource.onopen = () => {
      this.connected = true;
      this.#retryDelay = 1000;
    };

    this.#eventSource.onmessage = (event) => {
      this.lastMessage = event.data;
    };

    this.#eventSource.onerror = () => {
      this.connected = false;
      this.#eventSource?.close();
      this.#eventSource = null;

      // Exponential backoff reconnect if subscribers still exist
      if (this.#subscribers > 0) {
        setTimeout(() => this.#initStream(), this.#retryDelay);
        this.#retryDelay = Math.min(this.#retryDelay * 2, 30_000);
      }
    };
  }
}

export const realtimeFeed = new RealtimeFeed();
```

---

## 4. Dual Tauri IPC and HTTP Loopback Abstraction

When targeting both browser web clients and desktop apps (Tauri), abstract communication behind a unified transport layer:

```ts
// transport.ts
export function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

export async function requestRpc<T>(command: string, params: Record<string, unknown> = {}): Promise<T> {
  if (isTauri()) {
    const { invoke } = await import('@tauri-apps/api/core');
    return await invoke<T>(command, params);
  }

  // Fallback to local HTTP loopback (Axum server on localhost)
  const res = await fetch(`/api/rpc/${command}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(params),
  });

  if (!res.ok) {
    const errorText = await res.text();
    throw new Error(`RPC call ${command} failed (${res.status}): ${errorText}`);
  }

  return (await res.json()) as T;
}
```

---

## 5. Optimistic Updates with Rollback

For immediate user feedback on mutations (e.g. toggling a state, renaming a resource), apply the change locally first and roll back if the API call fails:

```ts
// tasks.svelte.ts
export interface Task {
  id: string;
  title: string;
  completed: boolean;
}

class TaskManager {
  tasks = $state<Task[]>([]);

  async toggleTask(id: string): Promise<void> {
    const target = this.tasks.find(t => t.id === id);
    if (!target) return;

    // 1. Optimistic mutation
    const previous = target.completed;
    target.completed = !previous;

    try {
      // 2. Network call
      const res = await fetch(`/api/tasks/${id}/toggle`, { method: 'POST' });
      if (!res.ok) throw new Error('Failed to update task');
    } catch (err) {
      // 3. Rollback on error
      target.completed = previous;
      console.error(`Task ${id} toggle failed, rolling back`, err);
      throw err;
    }
  }
}

export const taskManager = new TaskManager();
```
