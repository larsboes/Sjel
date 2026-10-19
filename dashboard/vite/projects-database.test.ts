import { describe, expect, test } from 'bun:test';
import {
  countCheckboxes,
  embed,
  extractExcerpt,
  extractFrontmatter,
  findBacklinks,
  linkTargets,
  resolveFile,
  resolveRecord,
  segments,
  setCheckbox,
} from '../src/lib/projects/database';
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

  test('a figure embed becomes its own segment with id and optional label', () => {
    const parts = segments(
      '# Intro\n![[figure:miele-w961-filter|W 961 mains filter]]\n![[figure:plain-id]]\nOutro',
      'Waschmaschine/W.md'
    );
    expect(parts).toEqual([
      { markdown: '# Intro' },
      { figure: 'miele-w961-filter', label: 'W 961 mains filter' },
      { figure: 'plain-id', label: undefined },
      { markdown: 'Outro' },
    ]);
  });

  test('a gen-ui fenced code block parses into a genui segment', () => {
    const raw = '# Telemetry\n```gen-ui\n{\n  "type": "metric-grid",\n  "title": "Subsystem Status",\n  "items": [{"label": "Voltage", "value": "230V"}]\n}\n```\nAfter text';
    const parts = segments(raw, 'Test/Test.md');
    expect(parts.length).toBe(3);
    expect('markdown' in parts[0]).toBe(true);
    expect('genui' in parts[1]).toBe(true);
    if ('genui' in parts[1]) {
      expect(parts[1].genui.type).toBe('metric-grid');
      if (parts[1].genui.type === 'metric-grid') {
        expect(parts[1].genui.title).toBe('Subsystem Status');
        expect(parts[1].genui.items.length).toBe(1);
      }
    }
    expect('markdown' in parts[2]).toBe(true);
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

describe('wikilinks and backlinks', () => {
  const sampleRecords = [
    {
      path: 'Washing Machine/Washing Machine Electrics.md',
      title: 'Washing Machine Electrics',
      content: '---\ntype: note\n---\n# Electrics\nSee [[Washing Machine Connect and Test]]. Also [[Miele Gala W 961]].',
    },
    {
      path: 'Washing Machine/Washing Machine Connect and Test.md',
      title: 'Washing Machine Connect and Test',
      content: '---\ntype: note\n---\n# Connect\nDetails in [[Washing Machine Electrics#Mains filter]].',
    },
    {
      path: 'Washing Machine/Tasks/Replace Filter.md',
      title: 'Replace Filter',
      content: '---\ntype: task\n---\nFollow [[Washing Machine Electrics]].',
    },
    {
      path: 'Other/Note.md',
      title: 'Other Note',
      content: 'Independent note.',
    },
  ];

  test('resolveRecord matches title and strips anchors and aliases', () => {
    const res1 = resolveRecord('Washing Machine Electrics#Mains filter', 'Washing Machine/Note.md', sampleRecords);
    expect(res1?.path).toBe('Washing Machine/Washing Machine Electrics.md');

    const res2 = resolveRecord('Washing Machine Connect and Test|Test', 'Washing Machine/Note.md', sampleRecords);
    expect(res2?.path).toBe('Washing Machine/Washing Machine Connect and Test.md');
  });

  test('findBacklinks finds all records referencing a target', () => {
    const links = findBacklinks(
      { path: 'Washing Machine/Washing Machine Electrics.md', title: 'Washing Machine Electrics' },
      sampleRecords,
    );
    expect(links.map((l) => l.path)).toEqual([
      'Washing Machine/Washing Machine Connect and Test.md',
      'Washing Machine/Tasks/Replace Filter.md',
    ]);
  });

  test('extractExcerpt removes frontmatter and headings', () => {
    const raw = '---\ntitle: Test\n---\n# Heading\nFirst line of text.\n\nSecond line here.';
    expect(extractExcerpt(raw, 2)).toBe('First line of text. Second line here.');
  });

  test('extractExcerpt with custom separator and multiline content', () => {
    const raw = '---\ntitle: Test\n---\n# Heading\nFirst line.\n\nSecond line.';
    expect(extractExcerpt(raw, 2, '\n\n')).toBe('First line.\n\nSecond line.');
  });

  test('extractFrontmatter parses keys and values cleanly', () => {
    const raw = '---\ntitle: Test Note\nmaturity: seedling\nstatus: active\n---\nBody';
    expect(extractFrontmatter(raw)).toEqual({
      title: 'Test Note',
      maturity: 'seedling',
      status: 'active',
    });
  });
});

