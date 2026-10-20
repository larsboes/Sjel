import { expect, test } from 'bun:test';
import { disposeNativeElement } from '../Packs/knowledge/plugins/interactive/src/native-renderer';
import { parseBlock } from '../Packs/knowledge/plugins/interactive/src/parser';

test('the interactive block resolves an HTML asset with an optional fixed height', () => {
  expect(parseBlock('file: Knowledge/Computing/Systems/Platforms/Assets/dial-indicator.html\nheight: 560\n')).toEqual({
    opts: { file: 'Knowledge/Computing/Systems/Platforms/Assets/dial-indicator.html', height: 560 }, content: '',
  });
});

test('disposal is scoped to one embedded controller and supports one-shot cleanup', () => {
  const first = new EventTarget();
  const second = new EventTarget();
  let firstCleanups = 0;
  let secondCleanups = 0;
  first.addEventListener('interactive-dispose', () => firstCleanups++, { once: true });
  second.addEventListener('interactive-dispose', () => secondCleanups++, { once: true });
  disposeNativeElement(first as HTMLElement);
  disposeNativeElement(first as HTMLElement);
  expect(firstCleanups).toBe(1);
  expect(secondCleanups).toBe(0);
  disposeNativeElement(second as HTMLElement);
  expect(secondCleanups).toBe(1);
});

test('an asset read that finishes after native embed teardown cannot mount stale content', () => {
  const mainPath = new URL('../Packs/knowledge/plugins/interactive/src/main.ts', import.meta.url).pathname;
  const result = Bun.spawnSync([process.execPath, '-e', `
    import { mock } from 'bun:test';
    import { strict as assert } from 'node:assert';
    class TFile { name = 'figure.html'; path = 'Assets/figure.html'; }
    mock.module('obsidian', () => ({ Plugin: class {}, TFile, normalizePath: p => p, MarkdownRenderChild: class {} }));
    const { default: Interactive } = await import(${JSON.stringify(mainPath)});
    for (const teardown of ['block', 'plugin']) {
      let completeRead;
      let readStarted;
      const started = new Promise(resolve => { readStarted = resolve; });
      const reading = new Promise(resolve => { completeRead = resolve; });
      const plugin = new Interactive();
      plugin.app = {
        vault: {
          getAbstractFileByPath: () => new TFile(),
          cachedRead: () => { readStarted(); return reading; },
          on: () => ({}),
        },
      };
      let mounts = 0;
      let disposals = 0;
      const root = new EventTarget();
      root.empty = () => { mounts++; };
      root.addEventListener('interactive-dispose', () => { disposals++; });
      const element = { empty() {}, createDiv: () => root };
      let removeBlock;
      const child = { registerEvent() {}, register: fn => { removeBlock = fn; } };
      const rendering = plugin.renderNativeBlock('file: Assets/figure.html', element, { sourcePath: 'Note.md' }, child);
      await started;
      if (teardown === 'block') removeBlock();
      else plugin.onunload();
      completeRead('<div>Late content</div>');
      await rendering;
      assert.equal(mounts, 0, teardown + ': stale content mounted');
      assert.equal(disposals, 1, teardown + ': controller was not disposed once');
      removeBlock();
      assert.equal(disposals, 1, teardown + ': controller disposed twice');
    }
    console.log('native embed teardown verified');
  `], { cwd: new URL('..', import.meta.url).pathname });
  expect(result.stderr.toString()).toBe('');
  expect(result.exitCode).toBe(0);
  expect(result.stdout.toString()).toContain('native embed teardown verified');
});
