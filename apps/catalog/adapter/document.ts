import { z } from 'zod';
import type { Spec } from '@json-render/core';
import { resolveBinding, type Binding } from '../src/bindings';

const children = z.array(z.string().min(1).max(80)).max(64);
const leaf = z.tuple([]);
const boundProps = z.strictObject({ binding: z.string().min(1).max(80) });
const element = z.discriminatedUnion('type', [
  z.strictObject({ type: z.literal('Stack'), props: z.strictObject({}), children }),
  z.strictObject({ type: z.literal('ObjectList'), props: boundProps, children: leaf }),
  z.strictObject({ type: z.literal('ObjectDetail'), props: boundProps, children: leaf }),
  z.strictObject({ type: z.literal('Draft'), props: boundProps, children: leaf }),
]);
const schema = z.strictObject({
  version: z.literal(1), root: z.string().min(1).max(80),
  elements: z.record(z.string().min(1).max(80), element),
});

/** Candidate evaluation document; it is not a production Rust admission contract. */
export type ViewDocument = z.infer<typeof schema>;

/** Parse a bounded candidate view and refuse unknown or write bindings before rendering. */
export function parseView(text: string, bindings: Readonly<Record<string, Binding>>): ViewDocument {
  const fail = (node: string, reason: string): never => { throw new Error(`${node}: ${reason}`); };
  if (new TextEncoder().encode(text).length > 65536) fail('root', 'document exceeds 64 KiB');
  let input: unknown;
  try { input = JSON.parse(text); } catch { return fail('root', 'invalid JSON'); }
  const result = schema.safeParse(input);
  if (!result.success) {
    const issue = result.error.issues[0];
    return fail(String(issue.path[0] === 'elements' ? issue.path[1] : 'root'), issue.message);
  }
  const doc = result.data;
  const nodes = Object.keys(doc.elements);
  if (nodes.length > 64) fail(doc.root, 'too many components');
  const seen = new Set<string>();
  function visit(id: string, depth: number): void {
    if (depth > 8) fail(id, 'maximum depth exceeded');
    if (seen.has(id)) fail(id, 'cycle or repeated component reference');
    const node = Object.hasOwn(doc.elements, id) ? doc.elements[id] : undefined;
    if (!node) return fail(id, 'missing component');
    seen.add(id);
    if (node.type !== 'Stack') {
      const binding = Object.hasOwn(bindings, node.props.binding) ? bindings[node.props.binding] : undefined;
      if (!binding) fail(id, 'unknown read binding');
      try { resolveBinding(binding!); } catch (error) { fail(id, String(error)); }
    }
    for (const child of node.children) visit(child, depth + 1);
  }
  visit(doc.root, 0);
  if (seen.size !== nodes.length) fail(doc.root, 'unreachable component');
  return doc;
}

/** Translate admitted evaluation data into a renderer spec without model-authored actions. */
export function rendererSpec(document: ViewDocument): Spec {
  return {
    root: document.root,
    elements: Object.fromEntries(Object.entries(document.elements).map(([id, node]) => [id, {
      ...node,
      ...(node.type === 'Draft' ? { on: { submit: { action: 'propose' } } } : {}),
    }])),
  };
}
