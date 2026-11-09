import { describe, expect, test } from 'bun:test';
import { parsePath, resumePoint, sittingPrompt } from '../src/path';

const path = (graph: string) => parsePath(`## Learning Path\n\n\`\`\`mermaid\ngraph TD\n${graph}\n\`\`\``)!;

describe('prerequisite-aware resume', () => {
  test('declaration order cannot bypass prerequisites, and joins require every parent', () => {
    const graph = path('end["Apply"]\nleft["Left"]:::held --> end\nright["Right"] --> end\nroot["Root"]:::held --> right');
    expect(resumePoint(graph).node?.id).toBe('right');
    graph.nodes.find((n) => n.id === 'right')!.state = 'held';
    expect(resumePoint(graph).node?.id).toBe('end');
  });

  test('a current step resumes before other ready roots', () => {
    expect(resumePoint(path('a["Root"]\nb["Unfinished"]:::current')).node?.id).toBe('b');
  });

  test('cycles, completed paths, ambiguous current nodes and unknown connectors are explicit', () => {
    expect(resumePoint(path('a --> b --> a')).reason).toBe('blocked');
    expect(resumePoint(path('a["A"]:::held')).reason).toBe('complete');
    expect(resumePoint(path('a["A"]:::current\nb["B"]:::current')).reason).toBe('invalid');
    expect(resumePoint(path('a & b --> c')).reason).toBe('invalid');
  });

  test('labelled and chained arrows preserve dependency edges', () => {
    const graph = path('a["Foundation"]:::held -->|needs| b["Next"] -.-> c["Later"]');
    expect(graph.nodes.map((n) => n.prerequisites)).toEqual([[], ['a'], ['b']]);
    expect(resumePoint(graph).node?.label).toBe('Next');
  });

  test('the planner supplies the exact resume point, budget and outcome', () => {
    const prompt = sittingPrompt('teach', '[[Resources/Learning/P]]', null, '', { next: 'B: Next', minutes: 25, outcome: 'Predict the bottleneck' });
    expect(prompt).toContain('Start at B: Next');
    expect(prompt).toContain('Time budget: 25 minutes');
    expect(prompt).toContain('Desired outcome: Predict the bottleneck');
  });
});
