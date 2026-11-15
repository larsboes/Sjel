/** Immutable host state with renderer writes restricted to local UI and form drafts. */
export interface RendererStateStore {
  get(path: string): unknown;
  set(path: string, value: unknown): void;
  update(updates: Record<string, unknown>): void;
  getSnapshot(): Record<string, unknown>;
  getServerSnapshot?(): Record<string, unknown>;
  subscribe(listener: () => void): () => void;
}

function freezeTree<T>(value: T, seen: WeakSet<object>): T {
  if (value !== null && typeof value === 'object' && !seen.has(value)) {
    seen.add(value);
    for (const child of Object.values(value)) freezeTree(child, seen);
    Object.freeze(value);
  }
  return value;
}

function requireLocalPath(path: string): void {
  if (!path.startsWith('/') || /~(?![01])/.test(path)) {
    throw new Error(`Invalid state pointer: ${path}`);
  }
  const segments = path.slice(1).split('/').map(part => part.replace(/~1/g, '/').replace(/~0/g, '~'));
  if (!['draft', 'ui'].includes(segments[0]) || segments.some(part => ['__proto__', 'prototype', 'constructor'].includes(part))) {
    throw new Error(`Renderer cannot write state pointer: ${path}`);
  }
}

/**
 * Wrap an immutable JSON store without exposing its host write authority.
 * Reads freeze shared snapshots; host updates must replace objects, not mutate them.
 * Throws when a renderer write leaves /draft or /ui, or its value cannot be cloned.
 */
export function createClientStateStore(host: RendererStateStore): RendererStateStore {
  const frozen = new WeakSet<object>();
  const serverSnapshot = host.getServerSnapshot?.bind(host);
  return {
    get: path => freezeTree(host.get(path), frozen),
    getSnapshot: () => freezeTree(host.getSnapshot(), frozen),
    ...(serverSnapshot ? { getServerSnapshot: () => freezeTree(serverSnapshot(), frozen) } : {}),
    subscribe: listener => host.subscribe(listener),
    set(path, value) {
      requireLocalPath(path);
      host.set(path, structuredClone(value));
    },
    update(updates) {
      for (const path of Object.keys(updates)) requireLocalPath(path);
      host.update(structuredClone(updates));
    },
  };
}
