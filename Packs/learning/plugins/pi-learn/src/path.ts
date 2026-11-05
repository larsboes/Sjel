/**
 * The learning-path schema, read from a path note's Markdown.
 *
 * The vault contract (`Resources/Vault-Contract.md`, Teaching records) owns the schema: a
 * `## Learning Path` section with one mermaid graph, and a node state given by its class.
 */

export type NodeState = 'held' | 'current' | 'open';

export interface PathNode {
  id: string;
  label: string;
  state: NodeState;
}

export interface LearningPath {
  nodes: PathNode[];
}

const STATES: NodeState[] = ['held', 'current', 'open'];

/** A node mention: id, an optional shaped label (quoted or bare), an optional `:::class`. */
const MENTION =
  /([A-Za-z_][\w-]*)(?:\s*(?:\[\[|\(\(|\{\{|\[|\(|\{|>)\s*(?:"([^"]*)"|([^\]\)\}]*?))\s*(?:\]\]|\)\)|\}\}|\]|\)|\}))?(?::::(\w+))?/g;

/** The mermaid source of the first code block under `## Learning Path`, or null. */
export function pathGraph(markdown: string): string | null {
  const section = /^## Learning Path[ \t]*$/m.exec(markdown);
  if (!section) return null;
  const rest = markdown.slice(section.index + section[0].length);
  const end = rest.search(/^## /m);
  const body = end >= 0 ? rest.slice(0, end) : rest;
  const block = /```mermaid[ \t]*\n([\s\S]*?)```/.exec(body);
  return block ? block[1] : null;
}

/**
 * Nodes and their states. A node takes its state from `id[Label]:::state` or from a
 * `class a,b state` line; a node with neither is open.
 */
export function parsePath(markdown: string): LearningPath | null {
  const graph = pathGraph(markdown);
  if (graph === null) return null;
  const nodes = new Map<string, PathNode>();
  const node = (id: string) => {
    let n = nodes.get(id);
    if (!n) {
      n = { id, label: id, state: 'open' };
      nodes.set(id, n);
    }
    return n;
  };
  const isState = (s: string): s is NodeState => (STATES as string[]).includes(s);

  for (const raw of graph.split('\n')) {
    const line = raw.trim();
    if (!line || line.startsWith('%%') || /^(graph|flowchart|classDef)\b/.test(line)) continue;
    const cls = /^class\s+([\w,\s]+?)\s+(\w+)\s*;?$/.exec(line);
    if (cls) {
      if (isState(cls[2])) for (const id of cls[1].split(',')) node(id.trim()).state = cls[2];
      continue;
    }
    if (/^(subgraph|end|click|style|linkStyle)\b/.test(line)) continue;
    // Edge labels (`-->|needs|`) are text, not nodes.
    const stripped = line.replace(/\|[^|]*\|/g, ' ');
    for (const m of stripped.matchAll(MENTION)) {
      const [, id, quoted, bare, state] = m;
      const n = node(id);
      const label = (quoted ?? bare ?? '').trim();
      if (label) n.label = label;
      if (state && isState(state)) n.state = state;
    }
  }
  return { nodes: [...nodes.values()] };
}

/** Counts per state, for the progress line. */
export function progress(path: LearningPath): Record<NodeState, number> {
  const out: Record<NodeState, number> = { held: 0, current: 0, open: 0 };
  for (const n of path.nodes) out[n.state]++;
  return out;
}

export type Kind = 'teach' | 'practice';

/** The opening prompt of a sitting, from the path, the focus note and a typed topic. */
export function sittingPrompt(kind: Kind, path: string | null, focus: string | null, topic: string): string {
  if (kind === 'practice') {
    if (focus) return `/skill:practice Practice the claims of ${focus}${path ? `, on the learning path ${path}` : ''}.`;
    if (path) return `/skill:practice Practice the held nodes of the learning path ${path}.`;
    return `/skill:practice ${topic}`;
  }
  if (path && focus) return `/skill:teach Continue the learning path ${path} at ${focus}.`;
  if (path) return `/skill:teach Continue the learning path ${path}. Resume at the node marked current, else the first open node.`;
  if (focus) return `/skill:teach Teach ${focus}: probe what I hold of it, then deepen the note.`;
  return `/skill:teach ${topic}`;
}
