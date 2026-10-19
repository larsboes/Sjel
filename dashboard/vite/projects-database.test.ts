import { describe, expect, test } from 'bun:test';
import { countCheckboxes, embed, linkTargets, resolveFile, segments, setCheckbox } from '../src/lib/projects/database';
import type { ProjectTask } from '../src/lib/api';

const row = (id: string, fields: Record<string, string> = {}) => ({ id, fields }) as ProjectTask;

describe('project note embeds', () => {
  test('a link target is the last path segment, not the label', () => {
    expect(linkTargets('[[Projects/Sjel/Sjel|Axon]], [[Waschmaschine]]')).toEqual(['Sjel', 'Waschmaschine']);
  });

  test('tasks by project match the tasks that link the host note', () => {
    const view = embed('![[Tasks.base#Active Tasks (by Project)|Tasks]]', 'Home-Lab/Home-Lab.md');
    if (!view || !('kinds' in view)) throw new Error('expected a live view');
    expect(view.label).toBe('Tasks');
    expect(view.filter(row('Home-Lab/Tasks/a.md', { projects: '[[Home-Lab]]' }))).toBe(true);
    expect(view.filter(row('Blog/Tasks/b.md', { projects: '[[Blog]]' }))).toBe(false);
  });

  test('one level down takes records in direct subfolders only', () => {
    const view = embed('![[Resources/Bases/Project-Notes.base#🗂️ One Level Down]]', 'Home-Lab/Home-Lab.md');
    if (!view || !('kinds' in view)) throw new Error('expected a live view');
    expect(view.filter(row('Home-Lab/Tasks/a.md'))).toBe(true);
    expect(view.filter(row('Home-Lab/a.md'))).toBe(false);
    expect(view.filter(row('Home-Lab/Home-Server/Deep/a.md'))).toBe(false);
  });

  test('a base over vault data links into the vault', () => {
    expect(embed('![[Papers.base|All Papers]]', 'X/X.md')).toEqual({ vault: 'Resources/Bases/Papers.base', label: 'All Papers' });
  });

  test('segments keep the markdown around an embed in order', () => {
    const parts = segments('# A\n\n![[Tasks.base#Active Tasks (by Project)]]\n\ntext', 'A/A.md');
    expect(parts.map((p) => ('markdown' in p ? 'md' : 'embed'))).toEqual(['md', 'embed', 'md']);
    expect(segments('![[image.png]]', 'A/A.md')).toEqual([{ markdown: '![[image.png]]' }]);
  });
});

describe('project attachments', () => {
  const files = ['Blog/Assets/a.png', 'Home-Lab/Assets/a.png', 'Archive/Old/Sources/paper.pdf'];

  test('a name resolves to the host project first, then anywhere', () => {
    expect(resolveFile('a.png', 'Home-Lab/Home-Lab.md', files)).toBe('Home-Lab/Assets/a.png');
    expect(resolveFile('a.png|400', 'Waschmaschine/W.md', files)).toBe('Blog/Assets/a.png');
    expect(resolveFile('Sources/paper.pdf', 'Archive/Old/Old.md', files)).toBe('Archive/Old/Sources/paper.pdf');
    expect(resolveFile('missing.png', 'Blog/Blog.md', files)).toBeNull();
  });

  test('an attachment line becomes its own segment', () => {
    const parts = segments('intro\n![[a.png]]\n![[gone.png]]', 'Blog/Blog.md', files);
    expect(parts).toEqual([{ markdown: 'intro' }, { file: 'Blog/Assets/a.png', image: true }, { markdown: '![[gone.png]]' }]);
  });
});

describe('note checkboxes', () => {
  const note = '---\ntype: task\nsummary: "- [ ] not a box"\n---\n\n## Details\n\n- [ ] one\n- [x] two\n```\n- [ ] in a fence\n```\n1. [ ] three';

  test('the n-th box after the frontmatter flips, and nothing else changes', () => {
    expect(setCheckbox(note, 0, true)).toBe(note.replace('- [ ] one', '- [x] one'));
    expect(setCheckbox(note, 1, false)).toBe(note.replace('- [x] two', '- [ ] two'));
    expect(setCheckbox(note, 2, true)).toBe(note.replace('1. [ ] three', '1. [x] three'));
  });

  test('a box inside a code fence is not counted, and a missing box is null', () => {
    expect(countCheckboxes('- [ ] a\n```\n- [ ] b\n```\n- [x] c')).toBe(2);
    expect(setCheckbox(note, 3, true)).toBeNull();
  });
});
