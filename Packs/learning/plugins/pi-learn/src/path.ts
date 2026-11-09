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
  prerequisites: string[];
}

export interface LearningPath {
  nodes: PathNode[];
  dependenciesKnown: boolean;
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
      n = { id, label: id, state: 'open', prerequisites: [] };
      nodes.set(id, n);
    }
    return n;
  };
  const isState = (s: string): s is NodeState => (STATES as string[]).includes(s);

  let dependenciesKnown = true;
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
    const stripped = line.replace(/\|[^|]*\|/g, ' ').replace(/;\s*$/, '');
    const mentions = [...stripped.matchAll(MENTION)];
    for (const m of mentions) {
      const [, id, quoted, bare, state] = m;
      const n = node(id);
      const label = (quoted ?? bare ?? '').trim();
      if (label) n.label = label;
      if (state && isState(state)) n.state = state;
    }
    for (let i = 1; i < mentions.length; i++) {
      const before = mentions[i - 1], after = mentions[i];
      const connector = stripped.slice(before.index! + before[0].length, after.index).trim();
      if (/^(-->|-\.->|==>)$/.test(connector)) {
        const prerequisites = node(after[1]).prerequisites;
        if (!prerequisites.includes(before[1])) prerequisites.push(before[1]);
      } else {
        dependenciesKnown = false;
      }
    }
  }
  return { nodes: [...nodes.values()], dependenciesKnown };
}

/** Counts per state, for the progress line. */
export function progress(path: LearningPath): Record<NodeState, number> {
  const out: Record<NodeState, number> = { held: 0, current: 0, open: 0 };
  for (const n of path.nodes) out[n.state]++;
  return out;
}

export interface ResumePoint {
  node: PathNode | null;
  reason: 'current' | 'ready' | 'complete' | 'blocked' | 'invalid';
}

/** Resume current work, otherwise choose an open node whose prerequisites are held. */
export function resumePoint(path: LearningPath): ResumePoint {
  const current = path.nodes.filter((n) => n.state === 'current');
  if (current.length > 1 || !path.nodes.length) return { node: null, reason: 'invalid' };
  if (current.length) return { node: current[0], reason: 'current' };
  if (path.nodes.every((n) => n.state === 'held')) return { node: null, reason: 'complete' };
  if (!path.dependenciesKnown) return { node: null, reason: 'invalid' };
  const held = new Set(path.nodes.filter((n) => n.state === 'held').map((n) => n.id));
  const node = path.nodes.find((n) => n.state === 'open' && n.prerequisites.every((id) => held.has(id)));
  return node ? { node, reason: 'ready' } : { node: null, reason: 'blocked' };
}

export type Kind = 'teach' | 'practice';
export interface SittingBrief { outcome?: string; minutes?: number; next?: string }

/** The opening prompt, including a learner-supplied goal, budget and graph-derived next step. */
export function sittingPrompt(kind: Kind, path: string | null, focus: string | null, topic: string, brief: SittingBrief = {}): string {
  let prompt: string;
  if (kind === 'practice') {
    if (focus) prompt = `/skill:practice Practice the claims of ${focus}${path ? `, on the learning path ${path}` : ''}.`;
    else if (path) prompt = `/skill:practice Practice the held nodes of the learning path ${path}.`;
    else prompt = `/skill:practice ${topic}`;
  } else {
    if (path && focus) prompt = `/skill:teach Continue the learning path ${path} at ${focus}.`;
    else if (path) prompt = `/skill:teach Continue the learning path ${path}. ${brief.next ? `Start at ${brief.next}.` : 'Resume at the node marked current, else the first open node whose prerequisites are held. If none is ready, inspect the graph before teaching.'}`;
    else if (focus) prompt = `/skill:teach Teach ${focus}: probe what I hold of it, then deepen the note.`;
    else prompt = `/skill:teach ${topic}`;
  }
  if ((path || focus) && topic) prompt += ` ${topic}`;
  if (brief.outcome?.trim()) prompt += `\nDesired outcome: ${brief.outcome.trim()}`;
  if (brief.minutes !== undefined) prompt += `\nTime budget: ${brief.minutes} minutes. Plan within this budget and reserve time to close; stop with an honest handoff if work remains.`;
  return prompt;
}
