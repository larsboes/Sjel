'use strict';

var obsidian = require('obsidian');
var child_process = require('child_process');
var path = require('path');
var electron = require('electron');
var state = require('@codemirror/state');
var view = require('@codemirror/view');
var commands = require('@codemirror/commands');

function _interopNamespaceDefault(e) {
    var n = Object.create(null);
    if (e) {
        Object.keys(e).forEach(function (k) {
            if (k !== 'default') {
                var d = Object.getOwnPropertyDescriptor(e, k);
                Object.defineProperty(n, k, d.get ? d : {
                    enumerable: true,
                    get: function () { return e[k]; }
                });
            }
        });
    }
    n.default = e;
    return Object.freeze(n);
}

var path__namespace = /*#__PURE__*/_interopNamespaceDefault(path);

// --- Quarto outline -------------------------------------------------------
//
// Obsidian's core Outline panel reads headings from metadataCache, which
// only parses .md files — a .qmd opened via registerExtensions still gets
// no heading cache, so the panel stays blank (issue #3). parseQmdHeadings
// scans the file text directly: ATX headings (`# ...`, up to 3 spaces of
// indent per CommonMark) only — setext headings (underlined with === / ---)
// are intentionally not supported, they are vanishingly rare in Quarto and
// the --- form collides with YAML/frontmatter syntax. The scan skips the
// YAML frontmatter block and fenced code blocks (``` / ~~~) so a `#` line
// inside an R/Python cell is not mistaken for a heading.
const QMD_OUTLINE_VIEW = 'qmd-outline-view';
// Nest a flat heading list into a tree so the outline can fold sub-trees,
// matching Obsidian's core Outline panel. A heading owns every later heading
// of a deeper level until one at its own level or shallower appears. Levels
// may skip (h1 -> h3); the level-stack handles that without synthetic nodes.
function buildHeadingTree(headings) {
    const roots = [];
    const stack = [];
    for (const h of headings) {
        const node = { ...h, children: [] };
        while (stack.length && stack[stack.length - 1].level >= h.level)
            stack.pop();
        (stack.length ? stack[stack.length - 1].children : roots).push(node);
        stack.push(node);
    }
    return roots;
}
function parseQmdHeadings(content) {
    const lines = content.split(/\r?\n/);
    const headings = [];
    let inFrontmatter = false;
    // Open code-fence state. Per CommonMark, a fence closes only on the same
    // marker char with a run at least as long as the opener — so a longer
    // ```` inside a ``` block, or a ~~~ inside a ``` block, does not close it.
    let fenceMarker = null; // '`' or '~' while inside a code block
    let fenceLength = 0; // length of the run that opened the current block
    for (let i = 0; i < lines.length; i++) {
        const line = lines[i];
        // YAML frontmatter is only frontmatter when --- is the very first line.
        if (i === 0 && /^---\s*$/.test(line)) {
            inFrontmatter = true;
            continue;
        }
        if (inFrontmatter) {
            if (/^(---|\.\.\.)\s*$/.test(line))
                inFrontmatter = false;
            continue;
        }
        // Fenced code block: a run of >=3 backticks or tildes, up to 3 spaces
        // of indent.
        const fence = line.match(/^\s{0,3}(`{3,}|~{3,})/);
        if (fence) {
            const run = fence[1];
            const marker = run[0];
            if (fenceMarker === null) {
                fenceMarker = marker;
                fenceLength = run.length;
            }
            else if (marker === fenceMarker && run.length >= fenceLength) {
                fenceMarker = null;
                fenceLength = 0;
            }
            continue;
        }
        if (fenceMarker !== null)
            continue;
        const h = line.match(/^\s{0,3}(#{1,6})\s+(.+?)\s*$/);
        if (h) {
            // Drop a trailing pandoc/quarto attribute block: `## Title {#id .cls}`.
            const text = h[2].replace(/\s*\{[^}]*\}\s*$/, '').trim();
            if (text)
                headings.push({ level: h[1].length, text, line: i });
        }
    }
    return headings;
}
class QmdOutlineView extends obsidian.ItemView {
    constructor(leaf, plugin) {
        super(leaf);
        // Headings whose children are folded away. Keyed by a text-path (the chain
        // of ancestor heading texts) so a fold survives the re-render that an edit
        // elsewhere in the file triggers — a line-based key would drift. Lives for
        // the view's lifetime; cleared only when the view is closed.
        this.collapsed = new Set();
        this.plugin = plugin;
    }
    getViewType() {
        return QMD_OUTLINE_VIEW;
    }
    getDisplayText() {
        return 'Quarto outline';
    }
    getIcon() {
        return 'list';
    }
    async onOpen() {
        // Enforce singleton: detach any other outline leaves so only this one
        // remains. Covers manual splits, popouts, or duplicate spawns.
        for (const leaf of this.app.workspace.getLeavesOfType(QMD_OUTLINE_VIEW)) {
            if (leaf !== this.leaf)
                leaf.detach();
        }
        // The outline may already be the active leaf at this point (opened via
        // command/setting), so capture the underlying .qmd before rendering.
        this.plugin.trackActiveQuartoFile();
        this.render();
    }
    // Find the open markdown view for a file, regardless of which leaf is
    // active. .qmd files open as 'markdown' leaves (registerExtensions).
    markdownViewFor(file) {
        for (const leaf of this.app.workspace.getLeavesOfType('markdown')) {
            if (leaf.view instanceof obsidian.MarkdownView && leaf.view.file?.path === file.path) {
                return leaf.view;
            }
        }
        return null;
    }
    render() {
        const container = this.contentEl;
        container.empty();
        container.addClass('qmd-outline');
        const file = this.plugin.lastActiveQuartoFile;
        if (!file) {
            container.createDiv({
                cls: 'qmd-outline-empty',
                text: this.plugin.settings.outlineMarkdownFiles
                    ? 'No Quarto (.qmd) or Markdown (.md) file is active.'
                    : 'No Quarto (.qmd) file is active.',
            });
            return;
        }
        // Read live content from the open editor rather than the active leaf —
        // clicking inside this sidebar makes it the active leaf.
        const mdView = this.markdownViewFor(file);
        if (!mdView) {
            container.createDiv({
                cls: 'qmd-outline-empty',
                text: `Open ${file.name} to see its outline.`,
            });
            return;
        }
        const headings = parseQmdHeadings(mdView.editor.getValue());
        if (headings.length === 0) {
            container.createDiv({
                cls: 'qmd-outline-empty',
                text: 'No headings in this file.',
            });
            return;
        }
        const list = container.createDiv({ cls: 'qmd-outline-list' });
        for (const node of buildHeadingTree(headings)) {
            this.renderNode(list, node, file, '');
        }
    }
    // Render one heading row plus, recursively, its sub-tree. A heading with
    // children gets a chevron that folds them away; the fold state is kept in
    // this.collapsed so it persists across re-renders.
    renderNode(parentEl, node, file, parentPath) {
        // Path = ancestor heading texts chained. A newline cannot occur in a
        // heading's text, so it is a collision-free separator between levels.
        const path = `${parentPath}\n${node.level}:${node.text}`;
        const hasChildren = node.children.length > 0;
        let isCollapsed = hasChildren && this.collapsed.has(path);
        const item = parentEl.createDiv({
            cls: 'qmd-outline-item',
            // Keyboard-accessible: focusable, announced as a link, and the keydown
            // handler below makes Enter/Space jump and Left/Right fold.
            attr: { tabindex: '0', role: 'link' },
        });
        // Indentation is driven by CSS off this attribute — no inline styles.
        item.dataset.level = String(node.level);
        // Toggle slot is always present (even leaf headings) so heading text
        // lines up regardless of whether a chevron is shown.
        const toggle = item.createSpan({ cls: 'qmd-outline-toggle' });
        item.createSpan({ cls: 'qmd-outline-item-text', text: node.text });
        let childrenEl = null;
        if (hasChildren) {
            obsidian.setIcon(toggle, 'chevron-down');
            toggle.addClass('is-clickable');
            childrenEl = parentEl.createDiv({ cls: 'qmd-outline-children' });
            for (const child of node.children) {
                this.renderNode(childrenEl, child, file, path);
            }
        }
        const applyFold = () => {
            item.toggleClass('is-collapsed', isCollapsed);
            childrenEl?.toggleClass('is-collapsed', isCollapsed);
        };
        const setFold = (collapse) => {
            isCollapsed = collapse;
            if (collapse)
                this.collapsed.add(path);
            else
                this.collapsed.delete(path);
            applyFold();
        };
        applyFold(); // reflect any persisted fold state on first paint
        const jumpTo = () => {
            // Resolve the editor by file, not by "active leaf" — the click itself
            // just moved focus to this sidebar.
            const view = this.markdownViewFor(file);
            if (!view)
                return;
            const pos = { line: node.line, ch: 0 };
            this.app.workspace.setActiveLeaf(view.leaf, { focus: true });
            view.editor.setCursor(pos);
            view.editor.scrollIntoView({ from: pos, to: pos }, true);
            view.editor.focus();
        };
        item.addEventListener('click', jumpTo);
        item.addEventListener('keydown', (evt) => {
            if (evt.key === 'Enter' || evt.key === ' ') {
                evt.preventDefault();
                jumpTo();
            }
            else if (hasChildren && evt.key === 'ArrowRight' && isCollapsed) {
                evt.preventDefault();
                setFold(false);
            }
            else if (hasChildren && evt.key === 'ArrowLeft' && !isCollapsed) {
                evt.preventDefault();
                setFold(true);
            }
        });
        if (hasChildren) {
            toggle.addEventListener('click', (evt) => {
                // Fold instead of jumping; the click would otherwise bubble to the
                // row's jump handler.
                evt.stopPropagation();
                setFold(!isCollapsed);
            });
        }
    }
}

const QMD_YAML_VIEW = 'qmd-yaml-view';
const QMD_LUA_VIEW = 'qmd-lua-view';
const yamlHighlightField = state.StateField.define({
    create(state) {
        return buildYamlDecorations(state.doc);
    },
    update(decorations, transaction) {
        if (transaction.docChanged) {
            return buildYamlDecorations(transaction.state.doc);
        }
        return decorations.map(transaction.changes);
    },
    provide: (field) => view.EditorView.decorations.from(field),
});
function buildYamlDecorations(doc) {
    const builder = new state.RangeSetBuilder();
    for (let lineNumber = 1; lineNumber <= doc.lines; lineNumber++) {
        const line = doc.line(lineNumber);
        decorateYamlLine(builder, line.from, line.text);
    }
    return builder.finish();
}
function decorateYamlLine(builder, lineStart, line) {
    const indent = line.match(/^\s*/)?.[0] ?? '';
    const content = line.slice(indent.length);
    const contentStart = lineStart + indent.length;
    if (!content)
        return;
    if (content.startsWith('#')) {
        markYamlToken(builder, contentStart, lineStart + line.length, 'qmd-yaml-comment');
        return;
    }
    const markerMatch = content.match(/^(-\s+)(.*)$/);
    if (markerMatch) {
        const markerLength = markerMatch[1].length;
        markYamlToken(builder, contentStart, contentStart + markerLength, 'qmd-yaml-list-marker');
        decorateYamlSegment(builder, contentStart + markerLength, markerMatch[2]);
        return;
    }
    decorateYamlSegment(builder, contentStart, content);
}
function decorateYamlSegment(builder, segmentStart, segment) {
    const docMatch = segment.match(/^(\.{3}|-{3})(\s*(#.*)?)$/);
    if (docMatch) {
        const markerLength = docMatch[1].length;
        markYamlToken(builder, segmentStart, segmentStart + markerLength, 'qmd-yaml-doc-marker');
        decorateYamlValue(builder, segmentStart + markerLength, docMatch[2] ?? '');
        return;
    }
    const colon = findYamlKeyColon(segment);
    if (colon !== -1) {
        markYamlToken(builder, segmentStart, segmentStart + colon, 'qmd-yaml-key');
        markYamlToken(builder, segmentStart + colon, segmentStart + colon + 1, 'qmd-yaml-colon');
        decorateYamlValue(builder, segmentStart + colon + 1, segment.slice(colon + 1));
        return;
    }
    decorateYamlValue(builder, segmentStart, segment);
}
function decorateYamlValue(builder, valueStart, value) {
    const leading = value.match(/^\s*/)?.[0] ?? '';
    const rest = value.slice(leading.length);
    const restStart = valueStart + leading.length;
    if (!rest)
        return;
    const commentIndex = findYamlComment(rest);
    const scalar = commentIndex === -1 ? rest : rest.slice(0, commentIndex);
    decorateYamlScalar(builder, restStart, scalar);
    if (commentIndex !== -1) {
        markYamlToken(builder, restStart + commentIndex, restStart + rest.length, 'qmd-yaml-comment');
    }
}
function decorateYamlScalar(builder, scalarStart, scalar) {
    const trailing = scalar.match(/\s*$/)?.[0] ?? '';
    const token = trailing ? scalar.slice(0, scalar.length - trailing.length) : scalar;
    if (!token)
        return;
    const className = yamlScalarClass(token);
    markYamlToken(builder, scalarStart, scalarStart + token.length, className);
}
function yamlScalarClass(token) {
    if (/^['"].*['"]$/.test(token))
        return 'qmd-yaml-string';
    if (/^[&*][A-Za-z0-9_-]+$/.test(token))
        return 'qmd-yaml-anchor';
    // YAML 1.2 (what Quarto/Pandoc use): only true/false/null/~ are
    // booleans/null. yes/no/on/off are plain scalars, not booleans.
    if (/^(true|false|null|~)$/i.test(token))
        return 'qmd-yaml-boolean';
    if (/^[-+]?(?:\d+\.?\d*|\.\d+)(?:e[-+]?\d+)?$/i.test(token))
        return 'qmd-yaml-number';
    if (/^[>|][+-]?$/.test(token))
        return 'qmd-yaml-block';
    if (/^(html|pdf|typst|latex|beamer|revealjs|docx|odt|epub|gfm|jats|dashboard)$/i.test(token)) {
        return 'qmd-yaml-quarto-format';
    }
    return 'qmd-yaml-scalar';
}
function findYamlKeyColon(segment) {
    let singleQuoted = false;
    let doubleQuoted = false;
    for (let i = 0; i < segment.length; i++) {
        const char = segment[i];
        const prev = i > 0 ? segment[i - 1] : '';
        if (char === "'" && !doubleQuoted) {
            singleQuoted = !singleQuoted;
        }
        else if (char === '"' && !singleQuoted && prev !== '\\') {
            doubleQuoted = !doubleQuoted;
        }
        else if (char === ':' && !singleQuoted && !doubleQuoted) {
            const next = segment[i + 1] ?? '';
            const key = segment.slice(0, i).trim();
            if (key && (!next || /\s/.test(next)))
                return i;
        }
    }
    return -1;
}
function findYamlComment(value) {
    let singleQuoted = false;
    let doubleQuoted = false;
    for (let i = 0; i < value.length; i++) {
        const char = value[i];
        const prev = i > 0 ? value[i - 1] : '';
        if (char === "'" && !doubleQuoted) {
            singleQuoted = !singleQuoted;
        }
        else if (char === '"' && !singleQuoted && prev !== '\\') {
            doubleQuoted = !doubleQuoted;
        }
        else if (char === '#' && !singleQuoted && !doubleQuoted && (i === 0 || /\s/.test(prev))) {
            return i;
        }
    }
    return -1;
}
function markYamlToken(builder, from, to, className) {
    if (to <= from)
        return;
    builder.add(from, to, view.Decoration.mark({ class: className }));
}
// --- Lua highlighting -----------------------------------------------------
//
// Minimal Lua syntax highlighting for the Lua file view — enough to make
// pandoc/Quarto filter scripts readable. A single forward scan over the
// whole document text marks comments, strings, numbers and keywords;
// everything else is left unstyled. The forward scan guarantees the
// RangeSetBuilder receives ranges in ascending, non-overlapping order.
const LUA_KEYWORDS = new Set([
    'and', 'break', 'do', 'else', 'elseif', 'end', 'false', 'for', 'function',
    'goto', 'if', 'in', 'local', 'nil', 'not', 'or', 'repeat', 'return', 'then',
    'true', 'until', 'while',
]);
const luaHighlightField = state.StateField.define({
    create(state) {
        return buildLuaDecorations(state.doc);
    },
    update(decorations, transaction) {
        if (transaction.docChanged) {
            return buildLuaDecorations(transaction.state.doc);
        }
        return decorations.map(transaction.changes);
    },
    provide: (field) => view.EditorView.decorations.from(field),
});
function buildLuaDecorations(doc) {
    const builder = new state.RangeSetBuilder();
    const s = doc.toString();
    const len = s.length;
    const mark = (from, to, className) => {
        if (to > from)
            builder.add(from, to, view.Decoration.mark({ class: className }));
    };
    // If a Lua long bracket opens at `open` (`[`, `[=[`, `[==[`, …), return the
    // index just past its matching close; an unterminated bracket runs to EOF.
    // Returns -1 when `open` is not a long-bracket opener.
    const longBracketEnd = (open) => {
        if (s[open] !== '[')
            return -1;
        let j = open + 1;
        let level = 0;
        while (s[j] === '=') {
            level++;
            j++;
        }
        if (s[j] !== '[')
            return -1;
        const close = ']' + '='.repeat(level) + ']';
        const idx = s.indexOf(close, j + 1);
        return idx === -1 ? len : idx + close.length;
    };
    let i = 0;
    while (i < len) {
        const c = s[i];
        // comment: line (`-- …`) or block (`--[[ … ]]`, `--[==[ … ]==]`)
        if (c === '-' && s[i + 1] === '-') {
            const block = longBracketEnd(i + 2);
            if (block !== -1) {
                mark(i, block, 'qmd-lua-comment');
                i = block;
                continue;
            }
            let j = i + 2;
            while (j < len && s[j] !== '\n')
                j++;
            mark(i, j, 'qmd-lua-comment');
            i = j;
            continue;
        }
        // long-bracket string
        if (c === '[') {
            const long = longBracketEnd(i);
            if (long !== -1) {
                mark(i, long, 'qmd-lua-string');
                i = long;
                continue;
            }
        }
        // quoted string (single or double); does not span lines
        if (c === '"' || c === "'") {
            let j = i + 1;
            while (j < len && s[j] !== c && s[j] !== '\n') {
                if (s[j] === '\\')
                    j++;
                j++;
            }
            if (j < len && s[j] === c)
                j++;
            mark(i, j, 'qmd-lua-string');
            i = j;
            continue;
        }
        // number (decimal, hex, fractional, exponent)
        if (/[0-9]/.test(c) || (c === '.' && /[0-9]/.test(s[i + 1] ?? ''))) {
            let j = i;
            if (c === '0' && (s[i + 1] === 'x' || s[i + 1] === 'X')) {
                j = i + 2;
                while (j < len && /[0-9a-fA-F.]/.test(s[j]))
                    j++;
            }
            else {
                while (j < len && /[0-9.]/.test(s[j]))
                    j++;
                if (s[j] === 'e' || s[j] === 'E') {
                    j++;
                    if (s[j] === '+' || s[j] === '-')
                        j++;
                    while (j < len && /[0-9]/.test(s[j]))
                        j++;
                }
            }
            mark(i, j, 'qmd-lua-number');
            i = j;
            continue;
        }
        // identifier — only keywords get marked
        if (/[A-Za-z_]/.test(c)) {
            let j = i + 1;
            while (j < len && /[A-Za-z0-9_]/.test(s[j]))
                j++;
            if (LUA_KEYWORDS.has(s.slice(i, j)))
                mark(i, j, 'qmd-lua-keyword');
            i = j;
            continue;
        }
        i++;
    }
    return builder.finish();
}
const YAML_CODE_VIEW = {
    label: 'YAML file',
    ariaLabel: 'YAML file contents',
    highlightField: yamlHighlightField,
};
const LUA_CODE_VIEW = {
    label: 'Lua file',
    ariaLabel: 'Lua file contents',
    highlightField: luaHighlightField,
};
// A minimal CodeMirror-backed file view, shared by the YAML and Lua file
// views. The only per-language difference is the highlight StateField and
// the labels, supplied through CodeViewConfig.
//
// getViewType() is left abstract on purpose: Obsidian's View constructor
// calls getViewType() *during* super(), before subclass constructor params
// and field initializers have run, so it cannot read instance state. Each
// concrete subclass returns a module-level literal instead.
class QmdCodeFileView extends obsidian.TextFileView {
    constructor(leaf, config) {
        super(leaf);
        this.config = config;
        this.editorView = null;
        this.settingViewData = false;
    }
    getDisplayText() {
        return this.file?.name ?? this.config.label;
    }
    getIcon() {
        return 'file-code';
    }
    onload() {
        super.onload();
        this.contentEl.empty();
        this.contentEl.addClass('qmd-code-view');
        this.editorView = new view.EditorView({
            parent: this.contentEl,
            state: state.EditorState.create({
                doc: this.data ?? '',
                extensions: [
                    state.EditorState.tabSize.of(2),
                    this.config.highlightField,
                    view.EditorView.contentAttributes.of({
                        'aria-label': this.config.ariaLabel,
                        autocapitalize: 'off',
                        autocomplete: 'off',
                        spellcheck: 'false',
                    }),
                    view.keymap.of([
                        { key: 'Tab', run: commands.indentMore },
                        { key: 'Shift-Tab', run: commands.indentLess },
                    ]),
                    view.EditorView.updateListener.of((update) => {
                        if (!update.docChanged || this.settingViewData)
                            return;
                        this.data = update.state.doc.toString();
                        this.requestSave();
                    }),
                ],
            }),
        });
        this.register(() => {
            this.editorView?.destroy();
            this.editorView = null;
        });
    }
    getViewData() {
        return this.editorView?.state.doc.toString() ?? this.data ?? '';
    }
    setViewData(data) {
        this.data = data;
        const view = this.editorView;
        if (!view)
            return;
        const current = view.state.doc.toString();
        if (current === data)
            return;
        this.settingViewData = true;
        try {
            view.dispatch({
                changes: {
                    from: 0,
                    to: current.length,
                    insert: data,
                },
            });
        }
        finally {
            this.settingViewData = false;
        }
    }
    clear() {
        this.setViewData('');
    }
}
class QmdYamlFileView extends QmdCodeFileView {
    constructor(leaf) {
        super(leaf, YAML_CODE_VIEW);
    }
    getViewType() {
        return QMD_YAML_VIEW;
    }
}
class QmdLuaFileView extends QmdCodeFileView {
    constructor(leaf) {
        super(leaf, LUA_CODE_VIEW);
    }
    getViewType() {
        return QMD_LUA_VIEW;
    }
}

// Shared Typst styling injected into both built-in Typst presets via
// `include-in-header`. Kept as one constant so the two presets cannot drift
// out of sync. Indentation matches the YAML position where it is interpolated
// (10 spaces, sitting inside `    include-in-header:` → `      - text: |`).
const TYPST_STYLING_HEADER = `    include-in-header:
      - text: |
          // Single accent color used throughout
          #let accent = rgb("#2E5C8A")

          // Page header: current top-level section + page number
          #set page(header: context {
            let heads = query(selector(heading.where(level: 1)).before(here()))
            let t = if heads.len() > 0 { heads.last().body } else [ ]
            text(size: 9pt, fill: gray, [#t #h(1fr) #counter(page).display()])
          })
          // Link color
          #show link: set text(fill: accent)
          // Boxed code blocks
          #show raw.where(block: true): it => block(
            fill: rgb("#f5f5f5"),
            inset: 10pt,
            radius: 4pt,
            width: 100%,
            it,
          )
          // Block quotes: colored left bar + muted italic
          #show quote.where(block: true): it => block(
            stroke: (left: 3pt + accent),
            inset: (left: 12pt, top: 4pt, bottom: 4pt),
            text(style: "italic", fill: gray.darken(20%), it.body),
          )
          // H1 headings: subtle accent rule
          #show heading.where(level: 1): it => [
            #it
            #v(-0.5em)
            #line(length: 100%, stroke: 0.5pt + accent.lighten(40%))
          ]`;
// Built-in presets. v0: hard-coded; later versions will let users edit/add
// these in plugin settings (same shape, persisted in QmdPluginSettings).
const DEFAULT_PRESETS = [
    {
        id: 'empty',
        name: 'Empty',
        description: 'Minimal front-matter, no format specified.',
        source: 'built-in',
        body: `---
title: ""
---

# Untitled
`,
    },
    {
        id: 'docx',
        name: 'Word (.docx)',
        description: 'format: docx with table of contents and numbered sections.',
        source: 'built-in',
        body: `---
title: ""
author: ""
date: today
format:
  docx:
    toc: true
    number-sections: true
    # reference-doc: reference.docx
---

# Untitled
`,
    },
    {
        id: 'typst-notes',
        name: 'Typst PDF — Notes (Eisvogel-style)',
        description: 'Eisvogel-inspired Typst PDF: TOC, numbered sections, page header, boxed code.',
        source: 'built-in',
        body: `---
title: ""
author: ""
date: today
toc: false
toc-depth: 2
number-sections: true
format:
  typst:
    papersize: a4
    margin:
      x: 2cm
      y: 2.5cm
    fontsize: 11pt
    section-numbering: "1.1.1"
    # mainfont: "Libertinus Serif"
    # sansfont: "Libertinus Sans"
    # monofont: "JetBrains Mono"
${TYPST_STYLING_HEADER}
---

# Untitled
`,
    },
    {
        id: 'typst-report',
        name: 'Typst PDF — Report (Eisvogel-style)',
        description: 'Eisvogel-inspired Typst report: cover metadata, TOC, numbered sections, boxed code, bibliography hints.',
        source: 'built-in',
        body: `---
title: ""
subtitle: ""
author: ""
date: today
# abstract: |
#   Short summary of the report.
# keywords: [keyword1, keyword2]
# bibliography: references.bib
# csl: ieee.csl
toc: true
toc-depth: 3
number-sections: true
format:
  typst:
    papersize: a4
    margin:
      x: 2.5cm
      y: 2.5cm
    fontsize: 11pt
    section-numbering: "1.1.1"
    # mainfont: "Libertinus Serif"
    # sansfont: "Libertinus Sans"
    # monofont: "JetBrains Mono"
${TYPST_STYLING_HEADER}
---

# Untitled
`,
    },
];
class PresetSuggestModal extends obsidian.SuggestModal {
    constructor(app, presets, onChoose) {
        super(app);
        this.presets = presets;
        this.onChoose = onChoose;
        this.setPlaceholder('Pick a Quarto file preset…');
    }
    getSuggestions(query) {
        const q = query.toLowerCase();
        if (!q)
            return this.presets;
        return this.presets.filter((p) => p.name.toLowerCase().includes(q) ||
            p.description.toLowerCase().includes(q));
    }
    renderSuggestion(preset, el) {
        const title = preset.source === 'built-in'
            ? `${preset.name}  (built-in)`
            : preset.name;
        el.createEl('div', { text: title, cls: 'qmd-preset-name' });
        el.createEl('small', { text: preset.description, cls: 'qmd-preset-desc' });
    }
    onChooseSuggestion(preset) {
        this.onChoose(preset);
    }
}
class FilenameModal extends obsidian.Modal {
    constructor(app, preset, folderPath, onSubmit) {
        super(app);
        this.preset = preset;
        this.folderPath = folderPath;
        this.onSubmit = onSubmit;
        this.value = 'untitled';
    }
    onOpen() {
        const { contentEl } = this;
        contentEl.empty();
        contentEl.createEl('h3', { text: `New Quarto file: ${this.preset.name}` });
        contentEl.createEl('p', {
            text: `Folder: ${this.folderPath || '(vault root)'}`,
            cls: 'setting-item-description',
        });
        new obsidian.Setting(contentEl)
            .setName('Filename')
            .setDesc('".qmd" is added automatically if omitted.')
            .addText((text) => {
            text
                .setPlaceholder('untitled')
                .setValue(this.value)
                .onChange((v) => (this.value = v));
            text.inputEl.focus();
            text.inputEl.select();
            text.inputEl.addEventListener('keydown', (e) => {
                // Skip during IME composition (Japanese/Chinese/Korean input),
                // and let modifier combos pass through to the OS / Obsidian.
                if (e.key === 'Enter' &&
                    !e.isComposing &&
                    !e.shiftKey &&
                    !e.metaKey &&
                    !e.ctrlKey &&
                    !e.altKey) {
                    e.preventDefault();
                    this.submit();
                }
            });
        });
        new obsidian.Setting(contentEl).addButton((btn) => btn
            .setButtonText('Create')
            .setCta()
            .onClick(() => this.submit()));
    }
    submit() {
        const raw = this.value.trim();
        if (!raw) {
            new obsidian.Notice('Filename is required.');
            return;
        }
        this.close();
        this.onSubmit(raw);
    }
    onClose() {
        this.contentEl.empty();
    }
}
// Sanitize a user-typed base filename so vault.create can't choke and the
// result still works after syncing to Windows / macOS / Linux:
//   - strip a trailing ".qmd" (case-insensitive); caller re-adds it
//   - replace path separators with "-"
//   - drop characters Windows forbids in filenames: : * ? " < > |
//   - trim leading/trailing whitespace and dots (Windows strips them anyway,
//     and a leading "." would make the file hidden on POSIX)
function sanitizeBaseName(input) {
    const withoutExt = input.replace(/\.qmd$/i, '');
    const noSlashes = withoutExt.replace(/[\\/]/g, '-');
    const noReserved = noSlashes.replace(/[:*?"<>|]/g, '');
    return noReserved.replace(/^[\s.]+|[\s.]+$/g, '');
}
// Pick a target path next to the active file, falling back to vault root.
// Existing files are not overwritten — appends "-1", "-2", … until free.
async function buildTargetPath(app, folderPath, base) {
    const safe = sanitizeBaseName(base) || 'untitled';
    let candidate = obsidian.normalizePath(folderPath ? `${folderPath}/${safe}.qmd` : `${safe}.qmd`);
    let n = 1;
    while (app.vault.getAbstractFileByPath(candidate)) {
        candidate = obsidian.normalizePath(folderPath ? `${folderPath}/${safe}-${n}.qmd` : `${safe}-${n}.qmd`);
        n += 1;
    }
    return candidate;
}
function activeFolderPath(app) {
    const active = app.workspace.getActiveFile();
    if (active?.parent)
        return active.parent.path === '/' ? '' : active.parent.path;
    return '';
}
// Read every top-level .qmd inside `folderPath` and turn each into a user
// preset. Subfolders are skipped — keeps the picker flat and predictable.
// Missing folder or read errors degrade silently to "no user presets"; the
// built-ins still show up so the command never appears broken.
async function loadUserPresets(app, folderPath) {
    const trimmed = folderPath.trim();
    if (!trimmed)
        return [];
    const folder = app.vault.getAbstractFileByPath(obsidian.normalizePath(trimmed));
    if (!(folder instanceof obsidian.TFolder))
        return [];
    const out = [];
    for (const child of folder.children) {
        if (!(child instanceof obsidian.TFile))
            continue;
        if (child.extension.toLowerCase() !== 'qmd')
            continue;
        try {
            const body = await app.vault.cachedRead(child);
            out.push({
                id: `user:${child.path}`,
                name: child.basename,
                description: `From ${child.path}`,
                source: 'user',
                body,
            });
        }
        catch (err) {
            console.warn(`[qmd-as-md] Skipped template ${child.path}:`, err);
        }
    }
    out.sort((a, b) => a.name.localeCompare(b.name));
    return out;
}
async function newQmdFromPreset(app, templatesFolder) {
    const userPresets = await loadUserPresets(app, templatesFolder);
    // User presets first — they're the ones the user actively curated; built-ins
    // fall to the bottom as fallback options.
    const presets = [...userPresets, ...DEFAULT_PRESETS];
    const modal = new PresetSuggestModal(app, presets, (preset) => {
        const folder = activeFolderPath(app);
        new FilenameModal(app, preset, folder, async (filename) => {
            try {
                const target = await buildTargetPath(app, folder, filename);
                const parent = target.includes('/')
                    ? target.slice(0, target.lastIndexOf('/'))
                    : '';
                if (parent) {
                    const existing = app.vault.getAbstractFileByPath(parent);
                    if (existing instanceof obsidian.TFile) {
                        // A file already occupies the would-be parent folder path —
                        // createFolder would surface a confusing low-level error; bail
                        // with a precise message instead.
                        new obsidian.Notice(`Cannot create folder "${parent}": a file with that name already exists.`);
                        return;
                    }
                    if (!(existing instanceof obsidian.TFolder)) {
                        await app.vault.createFolder(parent);
                    }
                }
                const file = await app.vault.create(target, preset.body);
                if (file instanceof obsidian.TFile) {
                    await app.workspace.getLeaf(false).openFile(file);
                }
                new obsidian.Notice(`Created ${target}`);
            }
            catch (err) {
                const msg = err instanceof Error ? err.message : String(err);
                new obsidian.Notice(`Failed to create file: ${msg}`);
            }
        }).open();
    });
    modal.open();
}

// --- Quarto output plumbing -----------------------------------------------
//
// Node's spawn-stream chunks don't align with line boundaries — a single
// data event can contain a partial line, and a logical line can be split
// across two events. Build a per-stream processor that buffers the
// trailing partial line and only emits whole lines. Call .flush() on the
// close handler to release any final partial line.
//
// logQuartoLine routes a single line to console by severity prefix:
// "ERROR:" -> console.error, "WARNING:"/"WARN:" -> console.warn,
// everything else -> console.log. Centralised so both the preview and
// render paths stay in sync and new prefixes only need handling here.
function logQuartoLine(prefix, line) {
    if (/^ERROR:/.test(line)) {
        console.error(`${prefix}: ${line}`);
    }
    else if (/^WARN(ING)?:/.test(line)) {
        console.warn(`${prefix}: ${line}`);
    }
    else {
        console.log(`${prefix}: ${line}`);
    }
}
function makeLineProcessor(handle) {
    let buffer = '';
    const proc = ((chunk) => {
        buffer += chunk;
        const lines = buffer.split(/\r?\n/);
        // Last element is the trailing fragment after the final newline
        // (or the whole chunk if there was no newline at all). Keep it
        // for the next chunk.
        buffer = lines.pop() ?? '';
        for (const line of lines) {
            if (line)
                handle(line);
        }
    });
    proc.flush = () => {
        if (buffer) {
            handle(buffer);
            buffer = '';
        }
    };
    return proc;
}
const ANSI_ESCAPE_RE = new RegExp(String.fromCharCode(27) + '\\[[0-?]*[ -/]*[@-~]', 'g');
function stripAnsiCodes(text) {
    return text.replace(ANSI_ESCAPE_RE, '');
}
function previewUrlFromLine(line) {
    const match = stripAnsiCodes(line).match(/Browse at\s+(https?:\/\/\S+)/);
    return match?.[1] ?? null;
}
const DEFAULT_SETTINGS = {
    quartoPath: 'quarto',
    enableQmdLinking: true,
    quartoTypst: '',
    openPdfInObsidian: false,
    previewInObsidian: true,
    previewMarkdownFiles: false,
    outlineMarkdownFiles: false,
    showYamlFiles: false,
    showLuaFiles: false,
    showOutline: false,
    templatesFolder: '',
};
class QmdAsMdPlugin extends obsidian.Plugin {
    constructor() {
        super(...arguments);
        this.activePreviewProcesses = new Map();
        // The .qmd file the outline should describe. Tracked separately from the
        // active leaf: clicking inside the outline sidebar makes *it* the active
        // leaf, so the outline must remember the last real .qmd rather than ask
        // "what is active now?" each render.
        this.lastActiveQuartoFile = null;
    }
    async onload() {
        try {
            await this.loadSettings();
            this.registerView(QMD_OUTLINE_VIEW, (leaf) => new QmdOutlineView(leaf, this));
            this.registerView(QMD_YAML_VIEW, (leaf) => new QmdYamlFileView(leaf));
            this.registerView(QMD_LUA_VIEW, (leaf) => new QmdLuaFileView(leaf));
            if (this.settings.enableQmdLinking) {
                this.registerQmdExtension();
            }
            if (this.settings.showYamlFiles) {
                this.registerYamlExtensions();
            }
            if (this.settings.showLuaFiles) {
                this.registerLuaExtensions();
            }
            this.addSettingTab(new QmdSettingTab(this.app, this));
            this.addRibbonIcon('eye', 'Toggle Quarto preview', async () => {
                const file = this.getActiveQuartoCommandFile();
                if (file)
                    await this.togglePreview(file);
            });
            this.addCommand({
                id: 'toggle-quarto-preview',
                name: 'Toggle Quarto preview',
                callback: async () => {
                    const file = this.getActiveQuartoCommandFile();
                    if (file)
                        await this.togglePreview(file);
                },
            });
            this.addCommand({
                id: 'toggle-quarto-preview-in-obsidian',
                name: 'Toggle Quarto preview in Obsidian',
                callback: async () => {
                    const file = this.getActiveQuartoCommandFile();
                    if (file)
                        await this.togglePreview(file, 'obsidian');
                },
            });
            this.addCommand({
                id: 'toggle-quarto-preview-external',
                name: 'Toggle Quarto preview in external browser',
                callback: async () => {
                    const file = this.getActiveQuartoCommandFile();
                    if (file)
                        await this.togglePreview(file, 'external');
                },
            });
            this.addRibbonIcon('file-output', 'Render Quarto to PDF', async () => {
                const file = this.getActiveQuartoCommandFile();
                if (file)
                    await this.renderPdf(file);
            });
            this.registerRenderCommand('render-quarto-pdf', 'Render Quarto (use format defined in YAML)');
            this.registerRenderCommand('render-quarto-pdf-typst', 'Render Quarto to PDF (Typst engine)', 'typst');
            this.registerRenderCommand('render-quarto-pdf-latex', 'Render Quarto to PDF (LaTeX engine)', 'pdf');
            this.addCommand({
                id: 'open-quarto-outline',
                name: 'Open Quarto outline',
                callback: () => this.activateOutlineView(),
            });
            this.addCommand({
                id: 'new-quarto-file-from-preset',
                name: 'New Quarto file from preset',
                icon: 'file-plus-2',
                callback: () => newQmdFromPreset(this.app, this.settings.templatesFolder),
            });
            // Keep any open outline view in sync with the focused file and its
            // edits. Debounced so a burst of keystrokes re-parses once it settles.
            const refresh = obsidian.debounce(() => {
                this.trackActiveQuartoFile();
                this.refreshOutlineViews();
            }, 250, true);
            this.registerEvent(this.app.workspace.on('active-leaf-change', refresh));
            this.registerEvent(this.app.workspace.on('editor-change', refresh));
            // Opt-in: only auto-open the outline when the user enabled it. The
            // command above always works regardless of this setting.
            if (this.settings.showOutline) {
                this.app.workspace.onLayoutReady(() => this.activateOutlineView());
            }
        }
        catch (error) {
            console.error('Error loading plugin:', error);
            new obsidian.Notice('Failed to load the QMD as md plugin. Check the developer console for details.');
        }
    }
    onunload() {
        this.stopAllPreviews();
    }
    async loadSettings() {
        const loaded = (await this.loadData());
        this.settings = Object.assign({}, DEFAULT_SETTINGS, loaded);
    }
    async saveSettings() {
        await this.saveData(this.settings);
    }
    isQuartoFile(file) {
        return file.extension === 'qmd';
    }
    isMarkdownFile(file) {
        return file.extension === 'md';
    }
    hasQuartoProjectConfigInPath(file) {
        let dir = file.parent?.path ?? '';
        while (true) {
            const configPath = obsidian.normalizePath(dir ? `${dir}/_quarto.yml` : '_quarto.yml');
            if (this.app.vault.getAbstractFileByPath(configPath) instanceof obsidian.TFile) {
                return true;
            }
            if (!dir)
                return false;
            const slash = dir.lastIndexOf('/');
            dir = slash === -1 ? '' : dir.slice(0, slash);
        }
    }
    getActiveQuartoCommandFile() {
        const activeView = this.app.workspace.getActiveViewOfType(obsidian.MarkdownView);
        const file = activeView?.file;
        if (!file) {
            new obsidian.Notice(this.settings.previewMarkdownFiles
                ? 'Quarto commands require an active .qmd or .md file.'
                : 'Quarto commands require an active .qmd file.');
            return null;
        }
        if (this.isQuartoFile(file)) {
            return file;
        }
        if (this.isMarkdownFile(file)) {
            if (!this.settings.previewMarkdownFiles) {
                new obsidian.Notice('Markdown support is off. Enable it in settings to preview or render .md files with Quarto.');
                return null;
            }
            if (!this.hasQuartoProjectConfigInPath(file)) {
                new obsidian.Notice('Markdown Quarto commands require _quarto.yml in this file folder or an ancestor up to the vault root.');
                return null;
            }
            return file;
        }
        if (this.settings.previewMarkdownFiles) {
            new obsidian.Notice('Quarto commands require an active .qmd or .md file.');
        }
        else {
            new obsidian.Notice('Quarto commands require an active .qmd file.');
        }
        return null;
    }
    getVaultFullPath(file) {
        const adapter = this.app.vault.adapter;
        if (adapter instanceof obsidian.FileSystemAdapter) {
            return adapter.getFullPath(file.path);
        }
        new obsidian.Notice('Vault is not on a local filesystem; cannot run Quarto.');
        return null;
    }
    pdfPathFor(file) {
        return file.path.replace(/\.(qmd|md)$/i, '.pdf');
    }
    // Pull the TFile a leaf currently shows, without resorting to
    // `as any`. The built-in PDF view (and most file-backed views)
    // extend FileView, which exposes a typed `file: TFile | null`.
    leafFile(leaf) {
        return leaf.view instanceof obsidian.FileView ? leaf.view.file : null;
    }
    // Open (or reuse) a leaf showing the given vault-relative PDF path in
    // Obsidian's native PDF viewer. Returns the leaf so callers can keep
    // refreshing it on subsequent preview compiles.
    //
    // Leaf-resolution order:
    //   1. Caller's captured ref, if still attached to the workspace.
    //   2. Any open 'pdf' leaf already showing this exact file (user may
    //      have opened it manually, or renderPdf may have opened it).
    //   3. New vertical split.
    async openOrRefreshPdfPreview(vaultPath, existingLeaf) {
        const pdfTFile = await this.waitForVaultFile(vaultPath);
        if (!pdfTFile) {
            new obsidian.Notice(`Quarto preview produced ${vaultPath} but it did not appear in the vault within the timeout.`);
            return null;
        }
        try {
            const reusable = existingLeaf?.parent != null
                ? existingLeaf
                : this.app.workspace
                    .getLeavesOfType('pdf')
                    .find((l) => this.leafFile(l)?.path === pdfTFile.path) ?? null;
            const leaf = reusable ?? this.app.workspace.getLeaf('split', 'vertical');
            // Skip the openFile call when the leaf already shows this file —
            // calling openFile in that case is harmless for the file display
            // but still triggers a reveal/focus shuffle the user does not want.
            // Obsidian's PDF viewer picks up the file rewrite via its own
            // mtime watcher, so live reload still works without our help.
            const currentFile = this.leafFile(leaf);
            if (!currentFile || currentFile.path !== pdfTFile.path) {
                await leaf.openFile(pdfTFile, { active: false });
            }
            await this.app.workspace.revealLeaf(leaf);
            return leaf;
        }
        catch (err) {
            console.error('[qmd-as-md] Failed to open PDF preview in native viewer:', err);
            new obsidian.Notice(`Could not open ${vaultPath} in Obsidian's PDF viewer.`);
            return null;
        }
    }
    async openPreviewUrl(url, mode) {
        console.log('[qmd-as-md][diag] openPreviewUrl called. url:', url, 'mode:', mode);
        new obsidian.Notice(`Preview available at ${url}`);
        if (mode === 'external') {
            // Quarto is launched with --no-browser in every mode; this opens the
            // captured URL once while leaving Quarto's live-reload client in charge
            // after the page is loaded.
            try {
                await electron.shell.openExternal(url);
            }
            catch (err) {
                console.error('[qmd-as-md] Failed to open external preview:', err);
                new obsidian.Notice(`Could not open external browser. Preview URL: ${url}`, 10000);
            }
            return;
        }
        // The "Web viewer" core plugin (Obsidian 1.8+) registers the
        // 'webviewer' view type. If the user has it disabled, setViewState
        // silently fails / leaves an empty leaf, and the user is left
        // wondering why nothing opened. Detect and report instead.
        const internalPlugins = this.app.internalPlugins;
        const webviewerOn = internalPlugins?.getEnabledPluginById?.('webviewer') != null ||
            internalPlugins?.plugins?.webviewer?.enabled === true;
        if (!webviewerOn) {
            new obsidian.Notice('Obsidian core plugin "Web viewer" is disabled — cannot show preview in-app. ' +
                'Enable it in Settings → Core plugins, or use "Toggle Quarto preview in external browser" instead. ' +
                'Falling back to your external browser.', 10000);
            console.warn('[qmd-as-md] webviewer core plugin disabled; preview URL was:', url);
            void electron.shell.openExternal(url);
            return;
        }
        try {
            const leaf = this.app.workspace.getLeaf('tab');
            await leaf.setViewState({
                type: 'webviewer',
                active: true,
                state: { url },
            });
            await this.app.workspace.revealLeaf(leaf);
        }
        catch (err) {
            console.error('[qmd-as-md] Failed to open preview in webviewer:', err);
            new obsidian.Notice("Could not open preview in Obsidian's web viewer. Falling back to external browser.");
            void electron.shell.openExternal(url);
        }
    }
    registerRenderCommand(id, name, toFormat) {
        this.addCommand({
            id,
            name,
            icon: 'file-output',
            callback: async () => {
                const file = this.getActiveQuartoCommandFile();
                if (file)
                    await this.renderPdf(file, toFormat);
            },
        });
    }
    registerQmdExtension() {
        this.registerExtensions(['qmd'], 'markdown');
    }
    registerYamlExtensions() {
        this.registerExtensions(['yml', 'yaml'], QMD_YAML_VIEW);
    }
    registerLuaExtensions() {
        this.registerExtensions(['lua'], QMD_LUA_VIEW);
    }
    // Open the Quarto outline in the right sidebar, reusing an existing
    // outline leaf if one is already open.
    async activateOutlineView() {
        const { workspace } = this.app;
        // Capture the current .qmd before opening the outline — setViewState
        // with active:true makes the outline the active leaf, after which the
        // active markdown view is gone.
        this.trackActiveQuartoFile();
        let leaf = workspace.getLeavesOfType(QMD_OUTLINE_VIEW)[0] ?? null;
        if (!leaf) {
            leaf = workspace.getRightLeaf(false);
            await leaf?.setViewState({ type: QMD_OUTLINE_VIEW, active: true });
        }
        if (leaf)
            await workspace.revealLeaf(leaf);
        this.refreshOutlineViews();
    }
    // Remember the active file the outline should describe. Called whenever the
    // active leaf changes; an active leaf the outline cannot describe (a non-.qmd
    // file, or a .md file when outlineMarkdownFiles is off, including the outline
    // sidebar itself) leaves the last value untouched so the outline keeps
    // describing that file.
    trackActiveQuartoFile() {
        const view = this.app.workspace.getActiveViewOfType(obsidian.MarkdownView);
        if (!view?.file)
            return;
        if (this.isQuartoFile(view.file) ||
            (this.settings.outlineMarkdownFiles && this.isMarkdownFile(view.file))) {
            this.lastActiveQuartoFile = view.file;
        }
    }
    // Re-render every open outline view. No-op when none are open.
    refreshOutlineViews() {
        for (const leaf of this.app.workspace.getLeavesOfType(QMD_OUTLINE_VIEW)) {
            if (leaf.view instanceof QmdOutlineView) {
                leaf.view.render();
            }
        }
    }
    // Close any open outline views — used when the user turns the setting off.
    detachOutlineViews() {
        for (const leaf of this.app.workspace.getLeavesOfType(QMD_OUTLINE_VIEW)) {
            leaf.detach();
        }
    }
    defaultPreviewMode() {
        return this.settings.previewInObsidian ? 'obsidian' : 'external';
    }
    async togglePreview(file, mode = this.defaultPreviewMode()) {
        const activePreview = this.activePreviewProcesses.get(file.path);
        if (activePreview?.mode === mode) {
            await this.stopPreview(file);
        }
        else {
            if (activePreview) {
                await this.stopPreview(file);
            }
            await this.startPreview(file, mode);
        }
    }
    async startPreview(file, mode = this.defaultPreviewMode()) {
        const activePreview = this.activePreviewProcesses.get(file.path);
        if (activePreview?.mode === mode) {
            return; // Preview already running for this file in this mode.
        }
        if (activePreview) {
            await this.stopPreview(file);
        }
        try {
            const abstractFile = this.app.vault.getAbstractFileByPath(file.path);
            if (!abstractFile || !(abstractFile instanceof obsidian.TFile)) {
                new obsidian.Notice(`File ${file.path} not found`);
                return;
            }
            const filePath = this.getVaultFullPath(abstractFile);
            if (!filePath)
                return;
            const workingDir = path__namespace.dirname(filePath);
            const envVars = { ...process.env };
            if (this.settings.quartoTypst.trim()) {
                envVars.QUARTO_TYPST = this.settings.quartoTypst.trim();
            }
            // Always suppress Quarto's own browser launch. The plugin opens the
            // captured URL exactly once for the selected target, which avoids
            // duplicate tabs and avoids Quarto-managed browser navigation closing
            // the preview process on subsequent source changes.
            const args = ['preview', filePath, '--no-browser'];
            // detached: `quarto preview` forks a separate long-lived server
            // process. Making the spawned process a process-group leader (POSIX)
            // lets killPreviewProcess signal the whole group — a plain kill() of
            // the wrapper would orphan the server, leaving it serving and
            // recompiling after "stop". No process groups on Windows; the kill
            // there goes through taskkill instead.
            const quartoProcess = child_process.spawn(this.settings.quartoPath, args, {
                cwd: workingDir,
                env: envVars,
                detached: process.platform !== 'win32',
            });
            let previewUrl = null;
            // PDF-preview state. Quarto emits "Output created: foo.pdf" on
            // every recompile (often several times per recompile); the
            // handler dedups so we don't spawn tabs on every save.
            //
            //  leaf:  current PDF tab, if any.
            //  path:  the path that leaf is showing (and the path of an
            //         in-flight open call, recorded synchronously when it
            //         is scheduled). Also gates the preview-URL skip logic
            //         on the "Browse at" branch below — when a PDF
            //         preview is active, we don't open Quarto's PDF.js
            //         wrapper page in the webviewer too.
            //  busy:  a call to openOrRefreshPdfPreview is in flight.
            //
            // Schedule open when any of:
            //   - leaf is detached (user closed the tab manually)
            //   - the new output path differs from the tracked one
            //     (multi-format project, or rename)
            //   - we never opened in this session
            // Skip when busy (bursts of emissions during recompile dedup
            // automatically; the final emission of a burst wins because it
            // arrives after busy clears).
            let pdfPreviewLeaf = null;
            let pdfPreviewPath = null;
            let pdfPreviewBusy = false;
            const schedulePdfPreview = (vaultPath) => {
                if (pdfPreviewBusy)
                    return;
                const leafAttached = pdfPreviewLeaf?.parent != null;
                const pathSame = pdfPreviewPath === vaultPath;
                if (leafAttached && pathSame)
                    return;
                pdfPreviewBusy = true;
                pdfPreviewPath = vaultPath;
                this.openOrRefreshPdfPreview(vaultPath, pdfPreviewLeaf)
                    .then((leaf) => {
                    if (leaf)
                        pdfPreviewLeaf = leaf;
                })
                    .catch((err) => {
                    console.error('[qmd-as-md] PDF preview open failed:', err);
                    pdfPreviewLeaf = null;
                    pdfPreviewPath = null;
                })
                    .finally(() => {
                    pdfPreviewBusy = false;
                });
            };
            // Quarto "ERROR:" lines from this preview run. Used both to surface
            // recompile failures live (preview keeps running, so the close
            // handler never fires) and to explain a startup exit in the Notice.
            const errorLines = [];
            // Dedupe: a single failed recompile emits the same ERROR: block on
            // every save until fixed — only Notice when the error text changes.
            let lastErrorShown = '';
            // Per-line handler: log the line, then look for the two markers
            // we care about ("Output created:" and "Browse at").
            const handlePreviewLine = (line) => {
                logQuartoLine('Quarto Preview', line);
                if (/^ERROR:/.test(line)) {
                    errorLines.push(line);
                    if (line !== lastErrorShown) {
                        lastErrorShown = line;
                        new obsidian.Notice(`Quarto preview error:\n${line}`, 15000);
                    }
                    return;
                }
                // A clean compile clears the dedupe guard so the same error
                // reappearing after a good build is surfaced again.
                if (line.includes('Output created:')) {
                    lastErrorShown = '';
                }
                // Detect "Output created: <path>" — quarto prints this on every
                // compile in preview mode. If the output is a PDF, route to
                // Obsidian's native PDF viewer rather than the webviewer page
                // Quarto serves at /web/viewer.html. Subsequent compiles refresh
                // the same leaf so live reload still works.
                const outMatch = line.match(/Output created:\s*(.+?)\s*$/);
                if (outMatch && /\.pdf$/i.test(outMatch[1].trim()) && mode === 'obsidian') {
                    const outBasename = path__namespace.basename(outMatch[1].trim());
                    const sourceDir = file.parent?.path ?? '';
                    const vaultPath = obsidian.normalizePath(sourceDir ? `${sourceDir}/${outBasename}` : outBasename);
                    schedulePdfPreview(vaultPath);
                    return;
                }
                const matchedPreviewUrl = previewUrlFromLine(line);
                if (!previewUrl && matchedPreviewUrl) {
                    console.log('[qmd-as-md][diag] Browse-at line seen.', 'matched:', matchedPreviewUrl, 'pdfPreviewPath:', pdfPreviewPath, 'mode:', mode);
                    previewUrl = matchedPreviewUrl;
                    // If we already opened a native PDF preview, skip the
                    // webviewer URL — Quarto's PDF.js wrapper would just be
                    // a worse version of the same content.
                    if (pdfPreviewPath) {
                        new obsidian.Notice(`PDF preview opened natively. Server URL: ${previewUrl}`);
                    }
                    else {
                        void this.openPreviewUrl(previewUrl, mode);
                    }
                }
            };
            // One buffered processor per stream — stdout and stderr each
            // need their own partial-line buffer, or interleaved fragments
            // from the two streams would be spliced into synthetic lines.
            const previewStdout = makeLineProcessor(handlePreviewLine);
            const previewStderr = makeLineProcessor(handlePreviewLine);
            quartoProcess.stdout?.on('data', (data) => previewStdout(data.toString()));
            quartoProcess.stderr?.on('data', (data) => previewStderr(data.toString()));
            // child_process.spawn does not throw on a missing binary; it emits
            // an 'error' event later. Without this listener an ENOENT just
            // produced a silent "exit 1" close with no output to console.
            quartoProcess.on('error', (err) => {
                console.error('[qmd-as-md] Failed to spawn quarto for preview:', err);
                new obsidian.Notice(`Failed to spawn '${this.settings.quartoPath}': ${err.message}. ` +
                    'Check the Quarto path setting and that Quarto is on PATH.');
                if (this.activePreviewProcesses.get(file.path)?.process === quartoProcess) {
                    this.activePreviewProcesses.delete(file.path);
                }
            });
            quartoProcess.on('close', (code, signal) => {
                previewStdout.flush(); // release any final partial line
                previewStderr.flush();
                if (code !== null && code !== 0) {
                    const reason = errorLines.length > 0
                        ? errorLines.join('\n')
                        : 'Check the developer console for details.';
                    new obsidian.Notice(`Quarto preview exited with code ${code}.\n${reason}`, 15000);
                }
                else if (code === null && signal && signal !== 'SIGTERM' && signal !== 'SIGKILL') {
                    // SIGTERM/SIGKILL come from our own stopPreview / onunload — silent.
                    new obsidian.Notice(`Quarto preview process was terminated by ${signal}`);
                }
                if (this.activePreviewProcesses.get(file.path)?.process === quartoProcess) {
                    this.activePreviewProcesses.delete(file.path);
                }
            });
            this.activePreviewProcesses.set(file.path, { process: quartoProcess, mode });
            new obsidian.Notice(`Quarto preview started (${mode === 'obsidian' ? 'Obsidian' : 'external browser'})`);
        }
        catch (error) {
            console.error('Failed to start Quarto preview:', error);
            new obsidian.Notice('Failed to start Quarto preview');
        }
    }
    // `quarto preview` forks a long-lived server as a child of the spawned
    // process, so killing only the wrapper leaves that server running. Signal
    // the whole process tree: the process group on POSIX (the child was
    // spawned detached, see startPreview), or taskkill /t on Windows.
    killPreviewProcess(quartoProcess) {
        if (quartoProcess.killed || quartoProcess.pid === undefined)
            return;
        if (process.platform === 'win32') {
            child_process.spawn('taskkill', ['/pid', String(quartoProcess.pid), '/t', '/f']);
            return;
        }
        try {
            // Negative PID targets the whole process group.
            process.kill(-quartoProcess.pid, 'SIGTERM');
        }
        catch {
            // Group already gone, or never became a leader — best-effort direct kill.
            try {
                quartoProcess.kill('SIGTERM');
            }
            catch {
                /* already dead */
            }
        }
    }
    async stopPreview(file) {
        const activePreview = this.activePreviewProcesses.get(file.path);
        if (activePreview) {
            this.killPreviewProcess(activePreview.process);
            this.activePreviewProcesses.delete(file.path);
            new obsidian.Notice('Quarto preview stopped');
        }
    }
    stopAllPreviews() {
        const hadPreviews = this.activePreviewProcesses.size > 0;
        this.activePreviewProcesses.forEach((activePreview, filePath) => {
            this.killPreviewProcess(activePreview.process);
            this.activePreviewProcesses.delete(filePath);
        });
        if (hadPreviews) {
            new obsidian.Notice('All Quarto previews stopped');
        }
    }
    async renderPdf(file, toFormat) {
        try {
            const abstractFile = this.app.vault.getAbstractFileByPath(file.path);
            if (!abstractFile || !(abstractFile instanceof obsidian.TFile)) {
                new obsidian.Notice(`File ${file.path} not found`);
                return;
            }
            // A running `quarto preview` keeps recompiling the same source and
            // writes to overlapping output paths. Stop it before a one-shot
            // render so the two Quarto processes do not fight over the output.
            if (this.activePreviewProcesses.has(file.path)) {
                await this.stopPreview(file);
            }
            const filePath = this.getVaultFullPath(abstractFile);
            if (!filePath)
                return;
            const workingDir = path__namespace.dirname(filePath);
            const envVars = { ...process.env };
            if (this.settings.quartoTypst.trim()) {
                envVars.QUARTO_TYPST = this.settings.quartoTypst.trim();
            }
            const engineLabel = toFormat === 'typst' ? 'Typst' : toFormat === 'pdf' ? 'LaTeX' : 'format defined in YAML';
            new obsidian.Notice(`Rendering Quarto (${engineLabel})...`);
            // Best-guess path used for the pre-render leaf-capture (so we can
            // reuse an existing PDF tab on recompile). The authoritative path
            // comes from quarto's "Output created:" stdout line, parsed below.
            const guessedPdfPath = this.pdfPathFor(file);
            const existingLeaf = this.app.workspace
                .getLeavesOfType('pdf')
                .find((l) => this.leafFile(l)?.path === guessedPdfPath);
            const args = ['render', filePath];
            if (toFormat)
                args.push('--to', toFormat);
            const quartoProcess = child_process.spawn(this.settings.quartoPath, args, {
                cwd: workingDir,
                env: envVars,
            });
            let detectedOutputBasename = null;
            // Quarto prints the human-readable cause on "ERROR:" lines (bad YAML,
            // missing engine, etc.). Keep them so a failing close can surface the
            // real reason in the Notice instead of a bare exit code.
            const errorLines = [];
            // Per-line handler: log the line, then watch for "Output created:".
            const handleRenderLine = (line) => {
                logQuartoLine('Quarto', line);
                const match = line.match(/Output created:\s*(.+?)\s*$/);
                if (match) {
                    detectedOutputBasename = path__namespace.basename(match[1].trim());
                }
                if (/^ERROR:/.test(line)) {
                    errorLines.push(line);
                }
            };
            // One buffered processor per stream — stdout and stderr each
            // need their own partial-line buffer, or interleaved fragments
            // from the two streams would be spliced into synthetic lines.
            const renderStdout = makeLineProcessor(handleRenderLine);
            const renderStderr = makeLineProcessor(handleRenderLine);
            quartoProcess.stdout?.on('data', (data) => renderStdout(data.toString()));
            quartoProcess.stderr?.on('data', (data) => renderStderr(data.toString()));
            // child_process.spawn does not throw on a missing binary; it emits
            // an 'error' event later. Without this listener an ENOENT just
            // produced a silent "exit 1" close with no output to console.
            quartoProcess.on('error', (err) => {
                console.error('[qmd-as-md] Failed to spawn quarto for render:', err);
                new obsidian.Notice(`Failed to spawn '${this.settings.quartoPath}': ${err.message}. ` +
                    'Check the Quarto path setting and that Quarto is on PATH.');
            });
            quartoProcess.on('close', (code, signal) => {
                void (async () => {
                    renderStdout.flush(); // release any final partial line
                    renderStderr.flush();
                    // A clean exit is code 0. Anything else is a failure, except a
                    // termination by SIGTERM/SIGKILL — that means the process was
                    // intentionally cancelled (matching the preview handler, which
                    // suppresses notices for those signals). Stay quiet then.
                    if (code === 0) {
                        // fall through to the success path below
                    }
                    else if (code === null && (signal === 'SIGTERM' || signal === 'SIGKILL')) {
                        console.error(`[qmd-as-md] Quarto render cancelled (${signal}).`);
                        return;
                    }
                    else {
                        const exitLabel = code !== null
                            ? `exit ${code}`
                            : signal
                                ? `terminated by ${signal}`
                                : 'terminated';
                        // The full output was already streamed line-by-line through
                        // console.log / console.error as it arrived — no need to
                        // re-dump it. Surface the actual ERROR: line(s) in the Notice so
                        // the user sees the cause (bad YAML, missing engine, ...) without
                        // having to open the developer console.
                        console.error(`[qmd-as-md] Quarto render failed (${exitLabel}).`);
                        const reason = errorLines.length > 0
                            ? errorLines.join('\n')
                            : 'Check the developer console for details.';
                        new obsidian.Notice(`Quarto render failed (${exitLabel}).\n${reason}`, 15000);
                        return;
                    }
                    const sourceDir = file.parent?.path ?? '';
                    const outputVaultPath = obsidian.normalizePath(detectedOutputBasename
                        ? (sourceDir ? `${sourceDir}/${detectedOutputBasename}` : detectedOutputBasename)
                        : guessedPdfPath);
                    const outputTFile = await this.waitForVaultFile(outputVaultPath);
                    if (!outputTFile) {
                        new obsidian.Notice(`Quarto rendered, but ${outputVaultPath} did not appear in the vault within the timeout. Check Quarto's output-dir or vault sync.`);
                        return;
                    }
                    const isPdf = outputVaultPath.toLowerCase().endsWith('.pdf');
                    if (!this.settings.openPdfInObsidian || !isPdf) {
                        new obsidian.Notice(isPdf
                            ? `PDF rendered: ${outputVaultPath}`
                            : `Rendered: ${outputVaultPath} (Obsidian's built-in viewer only handles PDFs).`);
                        return;
                    }
                    try {
                        const leaf = existingLeaf?.parent != null
                            ? existingLeaf
                            : this.app.workspace.getLeaf('split', 'vertical');
                        await leaf.openFile(outputTFile, { active: false });
                        await this.app.workspace.revealLeaf(leaf);
                        new obsidian.Notice(`Opened ${outputVaultPath}`);
                    }
                    catch (err) {
                        console.error('Failed to open PDF in Obsidian:', err);
                        new obsidian.Notice(`PDF rendered at ${outputVaultPath}, but Obsidian could not open it (no PDF viewer registered?).`);
                    }
                })().catch((err) => {
                    console.error('[qmd-as-md] Quarto render close handler failed:', err);
                });
            });
        }
        catch (error) {
            console.error('Failed to render Quarto PDF:', error);
            new obsidian.Notice('Failed to render Quarto PDF');
        }
    }
    async waitForVaultFile(vaultPath, timeoutMs = 5000) {
        const start = Date.now();
        while (Date.now() - start < timeoutMs) {
            const f = this.app.vault.getAbstractFileByPath(vaultPath);
            if (f instanceof obsidian.TFile)
                return f;
            await new Promise((r) => window.setTimeout(r, 200));
        }
        return null;
    }
}
class QmdSettingTab extends obsidian.PluginSettingTab {
    constructor(app, plugin) {
        super(app, plugin);
        this.plugin = plugin;
    }
    display() {
        const { containerEl } = this;
        containerEl.empty();
        new obsidian.Setting(containerEl)
            .setName('Quarto path')
            .setDesc('Path to the Quarto executable (e.g. quarto, /usr/local/bin/quarto)')
            .addText((text) => text
            .setPlaceholder('quarto')
            .setValue(this.plugin.settings.quartoPath)
            .onChange(async (value) => {
            this.plugin.settings.quartoPath = value;
            await this.plugin.saveSettings();
        }));
        new obsidian.Setting(containerEl)
            .setName('Templates folder')
            .setDesc('Vault folder containing your own .qmd template files for "New Quarto file from preset". ' +
            'Each .qmd in this folder (top level only — subfolders are ignored) becomes a preset; ' +
            'the file name is used as the preset name and the file contents are inserted verbatim. ' +
            'Leave empty to show only the built-in presets.')
            .addText((text) => text
            .setPlaceholder('e.g. _quarto-templates')
            .setValue(this.plugin.settings.templatesFolder)
            .onChange(async (value) => {
            this.plugin.settings.templatesFolder = value;
            await this.plugin.saveSettings();
        }));
        new obsidian.Setting(containerEl)
            .setName('Enable editing Quarto files')
            .setDesc('When on, .qmd files open in the Markdown editor. Turn off if another plugin handles .qmd editing.')
            .addToggle((toggle) => toggle
            .setValue(this.plugin.settings.enableQmdLinking)
            .onChange(async (value) => {
            this.plugin.settings.enableQmdLinking = value;
            if (value) {
                this.plugin.registerQmdExtension();
            }
            await this.plugin.saveSettings();
        }));
        new obsidian.Setting(containerEl)
            .setName('QUARTO_TYPST variable')
            .setDesc('Value for the QUARTO_TYPST environment variable (leave empty to unset).')
            .addText((text) => text
            .setPlaceholder('e.g. typst_path')
            .setValue(this.plugin.settings.quartoTypst)
            .onChange(async (value) => {
            this.plugin.settings.quartoTypst = value;
            await this.plugin.saveSettings();
        }));
        new obsidian.Setting(containerEl)
            .setName('Open compiled PDF in Obsidian')
            .setDesc("When rendering to PDF, open the resulting file inside Obsidian using the built-in PDF viewer. The .qmd source must live in the vault so the rendered PDF is accessible.")
            .addToggle((toggle) => toggle
            .setValue(this.plugin.settings.openPdfInObsidian)
            .onChange(async (value) => {
            this.plugin.settings.openPdfInObsidian = value;
            await this.plugin.saveSettings();
        }));
        new obsidian.Setting(containerEl)
            .setName('Open Quarto preview in Obsidian')
            .setDesc('Default target for the generic Toggle Quarto preview command and ribbon icon. ' +
            "When on: PDF previews (format: typst / pdf) open in Obsidian's native PDF viewer; " +
            "non-PDF previews (HTML, etc.) open in Obsidian 1.8's built-in web viewer. " +
            'When off, the plugin opens Quarto\'s preview URL in your default external browser. ' +
            'Use the explicit preview commands to choose a target without changing this setting.')
            .addToggle((toggle) => toggle
            .setValue(this.plugin.settings.previewInObsidian)
            .onChange(async (value) => {
            this.plugin.settings.previewInObsidian = value;
            await this.plugin.saveSettings();
        }));
        new obsidian.Setting(containerEl)
            .setName('Preview and render Markdown files with Quarto')
            .setDesc('When on, Quarto preview and render commands also accept .md files that have _quarto.yml in their folder or an ancestor up to the vault root. Leave off to restrict commands to .qmd files.')
            .addToggle((toggle) => toggle
            .setValue(this.plugin.settings.previewMarkdownFiles)
            .onChange(async (value) => {
            this.plugin.settings.previewMarkdownFiles = value;
            await this.plugin.saveSettings();
        }));
        new obsidian.Setting(containerEl)
            .setName('Show YAML files')
            .setDesc('When on, .yml and .yaml files appear in Obsidian using a CodeMirror editor with Quarto-oriented YAML highlighting. Turn off and reload the plugin to hide them again.')
            .addToggle((toggle) => toggle
            .setValue(this.plugin.settings.showYamlFiles)
            .onChange(async (value) => {
            this.plugin.settings.showYamlFiles = value;
            if (value) {
                this.plugin.registerYamlExtensions();
            }
            else {
                new obsidian.Notice('Reload the plugin to hide YAML files again.');
            }
            await this.plugin.saveSettings();
        }));
        new obsidian.Setting(containerEl)
            .setName('Show Lua files')
            .setDesc('When on, .lua files appear in Obsidian using a CodeMirror editor with minimal Lua syntax highlighting — handy for editing Quarto/pandoc filter scripts. Turn off and reload the plugin to hide them again.')
            .addToggle((toggle) => toggle
            .setValue(this.plugin.settings.showLuaFiles)
            .onChange(async (value) => {
            this.plugin.settings.showLuaFiles = value;
            if (value) {
                this.plugin.registerLuaExtensions();
            }
            else {
                new obsidian.Notice('Reload the plugin to hide Lua files again.');
            }
            await this.plugin.saveSettings();
        }));
        new obsidian.Setting(containerEl)
            .setName('Show Quarto outline')
            .setDesc("Add a sidebar outline of the active .qmd file's headings (Obsidian's " +
            'core Outline panel cannot read .qmd files). Active file only — ' +
            'headings from included files are not listed. The "Open Quarto ' +
            'outline" command works regardless of this toggle.')
            .addToggle((toggle) => toggle
            .setValue(this.plugin.settings.showOutline)
            .onChange(async (value) => {
            this.plugin.settings.showOutline = value;
            await this.plugin.saveSettings();
            if (value) {
                await this.plugin.activateOutlineView();
            }
            else {
                this.plugin.detachOutlineViews();
            }
        }));
        new obsidian.Setting(containerEl)
            .setName('Outline Markdown files')
            .setDesc('When on, the Quarto outline also lists headings of the active .md file, not just .qmd files. ' +
            "Obsidian's core Outline panel already covers .md files — enable this only if you prefer the Quarto outline for both.")
            .addToggle((toggle) => toggle
            .setValue(this.plugin.settings.outlineMarkdownFiles)
            .onChange(async (value) => {
            this.plugin.settings.outlineMarkdownFiles = value;
            await this.plugin.saveSettings();
            // Drop a now-ineligible .md target so the outline does not keep
            // describing a file it is no longer allowed to.
            if (!value &&
                this.plugin.lastActiveQuartoFile &&
                this.plugin.isMarkdownFile(this.plugin.lastActiveQuartoFile)) {
                this.plugin.lastActiveQuartoFile = null;
            }
            this.plugin.trackActiveQuartoFile();
            this.plugin.refreshOutlineViews();
        }));
    }
}

module.exports = QmdAsMdPlugin;
//# sourceMappingURL=data:application/json;charset=utf-8;base64,eyJ2ZXJzaW9uIjozLCJmaWxlIjoibWFpbi5qcyIsInNvdXJjZXMiOlsic3JjL291dGxpbmUudHMiLCJzcmMvY29kZS12aWV3LnRzIiwic3JjL25ldy1maWxlLnRzIiwic3JjL21haW4udHMiXSwic291cmNlc0NvbnRlbnQiOlsiaW1wb3J0IHsgSXRlbVZpZXcsIE1hcmtkb3duVmlldywgVEZpbGUsIFdvcmtzcGFjZUxlYWYsIHNldEljb24gfSBmcm9tICdvYnNpZGlhbic7XG5pbXBvcnQgdHlwZSBRbWRBc01kUGx1Z2luIGZyb20gJy4vbWFpbic7XG5cbi8vIC0tLSBRdWFydG8gb3V0bGluZSAtLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tXG4vL1xuLy8gT2JzaWRpYW4ncyBjb3JlIE91dGxpbmUgcGFuZWwgcmVhZHMgaGVhZGluZ3MgZnJvbSBtZXRhZGF0YUNhY2hlLCB3aGljaFxuLy8gb25seSBwYXJzZXMgLm1kIGZpbGVzIOKAlCBhIC5xbWQgb3BlbmVkIHZpYSByZWdpc3RlckV4dGVuc2lvbnMgc3RpbGwgZ2V0c1xuLy8gbm8gaGVhZGluZyBjYWNoZSwgc28gdGhlIHBhbmVsIHN0YXlzIGJsYW5rIChpc3N1ZSAjMykuIHBhcnNlUW1kSGVhZGluZ3Ncbi8vIHNjYW5zIHRoZSBmaWxlIHRleHQgZGlyZWN0bHk6IEFUWCBoZWFkaW5ncyAoYCMgLi4uYCwgdXAgdG8gMyBzcGFjZXMgb2Zcbi8vIGluZGVudCBwZXIgQ29tbW9uTWFyaykgb25seSDigJQgc2V0ZXh0IGhlYWRpbmdzICh1bmRlcmxpbmVkIHdpdGggPT09IC8gLS0tKVxuLy8gYXJlIGludGVudGlvbmFsbHkgbm90IHN1cHBvcnRlZCwgdGhleSBhcmUgdmFuaXNoaW5nbHkgcmFyZSBpbiBRdWFydG8gYW5kXG4vLyB0aGUgLS0tIGZvcm0gY29sbGlkZXMgd2l0aCBZQU1ML2Zyb250bWF0dGVyIHN5bnRheC4gVGhlIHNjYW4gc2tpcHMgdGhlXG4vLyBZQU1MIGZyb250bWF0dGVyIGJsb2NrIGFuZCBmZW5jZWQgY29kZSBibG9ja3MgKGBgYCAvIH5+fikgc28gYSBgI2AgbGluZVxuLy8gaW5zaWRlIGFuIFIvUHl0aG9uIGNlbGwgaXMgbm90IG1pc3Rha2VuIGZvciBhIGhlYWRpbmcuXG5cbmV4cG9ydCBjb25zdCBRTURfT1VUTElORV9WSUVXID0gJ3FtZC1vdXRsaW5lLXZpZXcnO1xuXG5pbnRlcmZhY2UgUW1kSGVhZGluZyB7XG4gIGxldmVsOiBudW1iZXI7XG4gIHRleHQ6IHN0cmluZztcbiAgbGluZTogbnVtYmVyOyAvLyAwLWJhc2VkIGxpbmUgaW5kZXggaW4gdGhlIHNvdXJjZVxufVxuXG5pbnRlcmZhY2UgUW1kSGVhZGluZ05vZGUgZXh0ZW5kcyBRbWRIZWFkaW5nIHtcbiAgY2hpbGRyZW46IFFtZEhlYWRpbmdOb2RlW107XG59XG5cbi8vIE5lc3QgYSBmbGF0IGhlYWRpbmcgbGlzdCBpbnRvIGEgdHJlZSBzbyB0aGUgb3V0bGluZSBjYW4gZm9sZCBzdWItdHJlZXMsXG4vLyBtYXRjaGluZyBPYnNpZGlhbidzIGNvcmUgT3V0bGluZSBwYW5lbC4gQSBoZWFkaW5nIG93bnMgZXZlcnkgbGF0ZXIgaGVhZGluZ1xuLy8gb2YgYSBkZWVwZXIgbGV2ZWwgdW50aWwgb25lIGF0IGl0cyBvd24gbGV2ZWwgb3Igc2hhbGxvd2VyIGFwcGVhcnMuIExldmVsc1xuLy8gbWF5IHNraXAgKGgxIC0+IGgzKTsgdGhlIGxldmVsLXN0YWNrIGhhbmRsZXMgdGhhdCB3aXRob3V0IHN5bnRoZXRpYyBub2Rlcy5cbmZ1bmN0aW9uIGJ1aWxkSGVhZGluZ1RyZWUoaGVhZGluZ3M6IFFtZEhlYWRpbmdbXSk6IFFtZEhlYWRpbmdOb2RlW10ge1xuICBjb25zdCByb290czogUW1kSGVhZGluZ05vZGVbXSA9IFtdO1xuICBjb25zdCBzdGFjazogUW1kSGVhZGluZ05vZGVbXSA9IFtdO1xuICBmb3IgKGNvbnN0IGggb2YgaGVhZGluZ3MpIHtcbiAgICBjb25zdCBub2RlOiBRbWRIZWFkaW5nTm9kZSA9IHsgLi4uaCwgY2hpbGRyZW46IFtdIH07XG4gICAgd2hpbGUgKHN0YWNrLmxlbmd0aCAmJiBzdGFja1tzdGFjay5sZW5ndGggLSAxXS5sZXZlbCA+PSBoLmxldmVsKSBzdGFjay5wb3AoKTtcbiAgICAoc3RhY2subGVuZ3RoID8gc3RhY2tbc3RhY2subGVuZ3RoIC0gMV0uY2hpbGRyZW4gOiByb290cykucHVzaChub2RlKTtcbiAgICBzdGFjay5wdXNoKG5vZGUpO1xuICB9XG4gIHJldHVybiByb290cztcbn1cblxuZnVuY3Rpb24gcGFyc2VRbWRIZWFkaW5ncyhjb250ZW50OiBzdHJpbmcpOiBRbWRIZWFkaW5nW10ge1xuICBjb25zdCBsaW5lcyA9IGNvbnRlbnQuc3BsaXQoL1xccj9cXG4vKTtcbiAgY29uc3QgaGVhZGluZ3M6IFFtZEhlYWRpbmdbXSA9IFtdO1xuICBsZXQgaW5Gcm9udG1hdHRlciA9IGZhbHNlO1xuICAvLyBPcGVuIGNvZGUtZmVuY2Ugc3RhdGUuIFBlciBDb21tb25NYXJrLCBhIGZlbmNlIGNsb3NlcyBvbmx5IG9uIHRoZSBzYW1lXG4gIC8vIG1hcmtlciBjaGFyIHdpdGggYSBydW4gYXQgbGVhc3QgYXMgbG9uZyBhcyB0aGUgb3BlbmVyIOKAlCBzbyBhIGxvbmdlclxuICAvLyBgYGBgIGluc2lkZSBhIGBgYCBibG9jaywgb3IgYSB+fn4gaW5zaWRlIGEgYGBgIGJsb2NrLCBkb2VzIG5vdCBjbG9zZSBpdC5cbiAgbGV0IGZlbmNlTWFya2VyOiBzdHJpbmcgfCBudWxsID0gbnVsbDsgLy8gJ2AnIG9yICd+JyB3aGlsZSBpbnNpZGUgYSBjb2RlIGJsb2NrXG4gIGxldCBmZW5jZUxlbmd0aCA9IDA7IC8vIGxlbmd0aCBvZiB0aGUgcnVuIHRoYXQgb3BlbmVkIHRoZSBjdXJyZW50IGJsb2NrXG5cbiAgZm9yIChsZXQgaSA9IDA7IGkgPCBsaW5lcy5sZW5ndGg7IGkrKykge1xuICAgIGNvbnN0IGxpbmUgPSBsaW5lc1tpXTtcblxuICAgIC8vIFlBTUwgZnJvbnRtYXR0ZXIgaXMgb25seSBmcm9udG1hdHRlciB3aGVuIC0tLSBpcyB0aGUgdmVyeSBmaXJzdCBsaW5lLlxuICAgIGlmIChpID09PSAwICYmIC9eLS0tXFxzKiQvLnRlc3QobGluZSkpIHtcbiAgICAgIGluRnJvbnRtYXR0ZXIgPSB0cnVlO1xuICAgICAgY29udGludWU7XG4gICAgfVxuICAgIGlmIChpbkZyb250bWF0dGVyKSB7XG4gICAgICBpZiAoL14oLS0tfFxcLlxcLlxcLilcXHMqJC8udGVzdChsaW5lKSkgaW5Gcm9udG1hdHRlciA9IGZhbHNlO1xuICAgICAgY29udGludWU7XG4gICAgfVxuXG4gICAgLy8gRmVuY2VkIGNvZGUgYmxvY2s6IGEgcnVuIG9mID49MyBiYWNrdGlja3Mgb3IgdGlsZGVzLCB1cCB0byAzIHNwYWNlc1xuICAgIC8vIG9mIGluZGVudC5cbiAgICBjb25zdCBmZW5jZSA9IGxpbmUubWF0Y2goL15cXHN7MCwzfShgezMsfXx+ezMsfSkvKTtcbiAgICBpZiAoZmVuY2UpIHtcbiAgICAgIGNvbnN0IHJ1biA9IGZlbmNlWzFdO1xuICAgICAgY29uc3QgbWFya2VyID0gcnVuWzBdO1xuICAgICAgaWYgKGZlbmNlTWFya2VyID09PSBudWxsKSB7XG4gICAgICAgIGZlbmNlTWFya2VyID0gbWFya2VyO1xuICAgICAgICBmZW5jZUxlbmd0aCA9IHJ1bi5sZW5ndGg7XG4gICAgICB9IGVsc2UgaWYgKG1hcmtlciA9PT0gZmVuY2VNYXJrZXIgJiYgcnVuLmxlbmd0aCA+PSBmZW5jZUxlbmd0aCkge1xuICAgICAgICBmZW5jZU1hcmtlciA9IG51bGw7XG4gICAgICAgIGZlbmNlTGVuZ3RoID0gMDtcbiAgICAgIH1cbiAgICAgIGNvbnRpbnVlO1xuICAgIH1cbiAgICBpZiAoZmVuY2VNYXJrZXIgIT09IG51bGwpIGNvbnRpbnVlO1xuXG4gICAgY29uc3QgaCA9IGxpbmUubWF0Y2goL15cXHN7MCwzfSgjezEsNn0pXFxzKyguKz8pXFxzKiQvKTtcbiAgICBpZiAoaCkge1xuICAgICAgLy8gRHJvcCBhIHRyYWlsaW5nIHBhbmRvYy9xdWFydG8gYXR0cmlidXRlIGJsb2NrOiBgIyMgVGl0bGUgeyNpZCAuY2xzfWAuXG4gICAgICBjb25zdCB0ZXh0ID0gaFsyXS5yZXBsYWNlKC9cXHMqXFx7W159XSpcXH1cXHMqJC8sICcnKS50cmltKCk7XG4gICAgICBpZiAodGV4dCkgaGVhZGluZ3MucHVzaCh7IGxldmVsOiBoWzFdLmxlbmd0aCwgdGV4dCwgbGluZTogaSB9KTtcbiAgICB9XG4gIH1cbiAgcmV0dXJuIGhlYWRpbmdzO1xufVxuXG5leHBvcnQgY2xhc3MgUW1kT3V0bGluZVZpZXcgZXh0ZW5kcyBJdGVtVmlldyB7XG4gIHBsdWdpbjogUW1kQXNNZFBsdWdpbjtcbiAgLy8gSGVhZGluZ3Mgd2hvc2UgY2hpbGRyZW4gYXJlIGZvbGRlZCBhd2F5LiBLZXllZCBieSBhIHRleHQtcGF0aCAodGhlIGNoYWluXG4gIC8vIG9mIGFuY2VzdG9yIGhlYWRpbmcgdGV4dHMpIHNvIGEgZm9sZCBzdXJ2aXZlcyB0aGUgcmUtcmVuZGVyIHRoYXQgYW4gZWRpdFxuICAvLyBlbHNld2hlcmUgaW4gdGhlIGZpbGUgdHJpZ2dlcnMg4oCUIGEgbGluZS1iYXNlZCBrZXkgd291bGQgZHJpZnQuIExpdmVzIGZvclxuICAvLyB0aGUgdmlldydzIGxpZmV0aW1lOyBjbGVhcmVkIG9ubHkgd2hlbiB0aGUgdmlldyBpcyBjbG9zZWQuXG4gIHByaXZhdGUgY29sbGFwc2VkID0gbmV3IFNldDxzdHJpbmc+KCk7XG5cbiAgY29uc3RydWN0b3IobGVhZjogV29ya3NwYWNlTGVhZiwgcGx1Z2luOiBRbWRBc01kUGx1Z2luKSB7XG4gICAgc3VwZXIobGVhZik7XG4gICAgdGhpcy5wbHVnaW4gPSBwbHVnaW47XG4gIH1cblxuICBnZXRWaWV3VHlwZSgpOiBzdHJpbmcge1xuICAgIHJldHVybiBRTURfT1VUTElORV9WSUVXO1xuICB9XG5cbiAgZ2V0RGlzcGxheVRleHQoKTogc3RyaW5nIHtcbiAgICByZXR1cm4gJ1F1YXJ0byBvdXRsaW5lJztcbiAgfVxuXG4gIGdldEljb24oKTogc3RyaW5nIHtcbiAgICByZXR1cm4gJ2xpc3QnO1xuICB9XG5cbiAgYXN5bmMgb25PcGVuKCk6IFByb21pc2U8dm9pZD4ge1xuICAgIC8vIEVuZm9yY2Ugc2luZ2xldG9uOiBkZXRhY2ggYW55IG90aGVyIG91dGxpbmUgbGVhdmVzIHNvIG9ubHkgdGhpcyBvbmVcbiAgICAvLyByZW1haW5zLiBDb3ZlcnMgbWFudWFsIHNwbGl0cywgcG9wb3V0cywgb3IgZHVwbGljYXRlIHNwYXducy5cbiAgICBmb3IgKGNvbnN0IGxlYWYgb2YgdGhpcy5hcHAud29ya3NwYWNlLmdldExlYXZlc09mVHlwZShRTURfT1VUTElORV9WSUVXKSkge1xuICAgICAgaWYgKGxlYWYgIT09IHRoaXMubGVhZikgbGVhZi5kZXRhY2goKTtcbiAgICB9XG4gICAgLy8gVGhlIG91dGxpbmUgbWF5IGFscmVhZHkgYmUgdGhlIGFjdGl2ZSBsZWFmIGF0IHRoaXMgcG9pbnQgKG9wZW5lZCB2aWFcbiAgICAvLyBjb21tYW5kL3NldHRpbmcpLCBzbyBjYXB0dXJlIHRoZSB1bmRlcmx5aW5nIC5xbWQgYmVmb3JlIHJlbmRlcmluZy5cbiAgICB0aGlzLnBsdWdpbi50cmFja0FjdGl2ZVF1YXJ0b0ZpbGUoKTtcbiAgICB0aGlzLnJlbmRlcigpO1xuICB9XG5cbiAgLy8gRmluZCB0aGUgb3BlbiBtYXJrZG93biB2aWV3IGZvciBhIGZpbGUsIHJlZ2FyZGxlc3Mgb2Ygd2hpY2ggbGVhZiBpc1xuICAvLyBhY3RpdmUuIC5xbWQgZmlsZXMgb3BlbiBhcyAnbWFya2Rvd24nIGxlYXZlcyAocmVnaXN0ZXJFeHRlbnNpb25zKS5cbiAgcHJpdmF0ZSBtYXJrZG93blZpZXdGb3IoZmlsZTogVEZpbGUpOiBNYXJrZG93blZpZXcgfCBudWxsIHtcbiAgICBmb3IgKGNvbnN0IGxlYWYgb2YgdGhpcy5hcHAud29ya3NwYWNlLmdldExlYXZlc09mVHlwZSgnbWFya2Rvd24nKSkge1xuICAgICAgaWYgKGxlYWYudmlldyBpbnN0YW5jZW9mIE1hcmtkb3duVmlldyAmJiBsZWFmLnZpZXcuZmlsZT8ucGF0aCA9PT0gZmlsZS5wYXRoKSB7XG4gICAgICAgIHJldHVybiBsZWFmLnZpZXc7XG4gICAgICB9XG4gICAgfVxuICAgIHJldHVybiBudWxsO1xuICB9XG5cbiAgcmVuZGVyKCk6IHZvaWQge1xuICAgIGNvbnN0IGNvbnRhaW5lciA9IHRoaXMuY29udGVudEVsO1xuICAgIGNvbnRhaW5lci5lbXB0eSgpO1xuICAgIGNvbnRhaW5lci5hZGRDbGFzcygncW1kLW91dGxpbmUnKTtcblxuICAgIGNvbnN0IGZpbGUgPSB0aGlzLnBsdWdpbi5sYXN0QWN0aXZlUXVhcnRvRmlsZTtcbiAgICBpZiAoIWZpbGUpIHtcbiAgICAgIGNvbnRhaW5lci5jcmVhdGVEaXYoe1xuICAgICAgICBjbHM6ICdxbWQtb3V0bGluZS1lbXB0eScsXG4gICAgICAgIHRleHQ6IHRoaXMucGx1Z2luLnNldHRpbmdzLm91dGxpbmVNYXJrZG93bkZpbGVzXG4gICAgICAgICAgPyAnTm8gUXVhcnRvICgucW1kKSBvciBNYXJrZG93biAoLm1kKSBmaWxlIGlzIGFjdGl2ZS4nXG4gICAgICAgICAgOiAnTm8gUXVhcnRvICgucW1kKSBmaWxlIGlzIGFjdGl2ZS4nLFxuICAgICAgfSk7XG4gICAgICByZXR1cm47XG4gICAgfVxuXG4gICAgLy8gUmVhZCBsaXZlIGNvbnRlbnQgZnJvbSB0aGUgb3BlbiBlZGl0b3IgcmF0aGVyIHRoYW4gdGhlIGFjdGl2ZSBsZWFmIOKAlFxuICAgIC8vIGNsaWNraW5nIGluc2lkZSB0aGlzIHNpZGViYXIgbWFrZXMgaXQgdGhlIGFjdGl2ZSBsZWFmLlxuICAgIGNvbnN0IG1kVmlldyA9IHRoaXMubWFya2Rvd25WaWV3Rm9yKGZpbGUpO1xuICAgIGlmICghbWRWaWV3KSB7XG4gICAgICBjb250YWluZXIuY3JlYXRlRGl2KHtcbiAgICAgICAgY2xzOiAncW1kLW91dGxpbmUtZW1wdHknLFxuICAgICAgICB0ZXh0OiBgT3BlbiAke2ZpbGUubmFtZX0gdG8gc2VlIGl0cyBvdXRsaW5lLmAsXG4gICAgICB9KTtcbiAgICAgIHJldHVybjtcbiAgICB9XG5cbiAgICBjb25zdCBoZWFkaW5ncyA9IHBhcnNlUW1kSGVhZGluZ3MobWRWaWV3LmVkaXRvci5nZXRWYWx1ZSgpKTtcbiAgICBpZiAoaGVhZGluZ3MubGVuZ3RoID09PSAwKSB7XG4gICAgICBjb250YWluZXIuY3JlYXRlRGl2KHtcbiAgICAgICAgY2xzOiAncW1kLW91dGxpbmUtZW1wdHknLFxuICAgICAgICB0ZXh0OiAnTm8gaGVhZGluZ3MgaW4gdGhpcyBmaWxlLicsXG4gICAgICB9KTtcbiAgICAgIHJldHVybjtcbiAgICB9XG5cbiAgICBjb25zdCBsaXN0ID0gY29udGFpbmVyLmNyZWF0ZURpdih7IGNsczogJ3FtZC1vdXRsaW5lLWxpc3QnIH0pO1xuICAgIGZvciAoY29uc3Qgbm9kZSBvZiBidWlsZEhlYWRpbmdUcmVlKGhlYWRpbmdzKSkge1xuICAgICAgdGhpcy5yZW5kZXJOb2RlKGxpc3QsIG5vZGUsIGZpbGUsICcnKTtcbiAgICB9XG4gIH1cblxuICAvLyBSZW5kZXIgb25lIGhlYWRpbmcgcm93IHBsdXMsIHJlY3Vyc2l2ZWx5LCBpdHMgc3ViLXRyZWUuIEEgaGVhZGluZyB3aXRoXG4gIC8vIGNoaWxkcmVuIGdldHMgYSBjaGV2cm9uIHRoYXQgZm9sZHMgdGhlbSBhd2F5OyB0aGUgZm9sZCBzdGF0ZSBpcyBrZXB0IGluXG4gIC8vIHRoaXMuY29sbGFwc2VkIHNvIGl0IHBlcnNpc3RzIGFjcm9zcyByZS1yZW5kZXJzLlxuICBwcml2YXRlIHJlbmRlck5vZGUoXG4gICAgcGFyZW50RWw6IEhUTUxFbGVtZW50LFxuICAgIG5vZGU6IFFtZEhlYWRpbmdOb2RlLFxuICAgIGZpbGU6IFRGaWxlLFxuICAgIHBhcmVudFBhdGg6IHN0cmluZ1xuICApOiB2b2lkIHtcbiAgICAvLyBQYXRoID0gYW5jZXN0b3IgaGVhZGluZyB0ZXh0cyBjaGFpbmVkLiBBIG5ld2xpbmUgY2Fubm90IG9jY3VyIGluIGFcbiAgICAvLyBoZWFkaW5nJ3MgdGV4dCwgc28gaXQgaXMgYSBjb2xsaXNpb24tZnJlZSBzZXBhcmF0b3IgYmV0d2VlbiBsZXZlbHMuXG4gICAgY29uc3QgcGF0aCA9IGAke3BhcmVudFBhdGh9XFxuJHtub2RlLmxldmVsfToke25vZGUudGV4dH1gO1xuICAgIGNvbnN0IGhhc0NoaWxkcmVuID0gbm9kZS5jaGlsZHJlbi5sZW5ndGggPiAwO1xuICAgIGxldCBpc0NvbGxhcHNlZCA9IGhhc0NoaWxkcmVuICYmIHRoaXMuY29sbGFwc2VkLmhhcyhwYXRoKTtcblxuICAgIGNvbnN0IGl0ZW0gPSBwYXJlbnRFbC5jcmVhdGVEaXYoe1xuICAgICAgY2xzOiAncW1kLW91dGxpbmUtaXRlbScsXG4gICAgICAvLyBLZXlib2FyZC1hY2Nlc3NpYmxlOiBmb2N1c2FibGUsIGFubm91bmNlZCBhcyBhIGxpbmssIGFuZCB0aGUga2V5ZG93blxuICAgICAgLy8gaGFuZGxlciBiZWxvdyBtYWtlcyBFbnRlci9TcGFjZSBqdW1wIGFuZCBMZWZ0L1JpZ2h0IGZvbGQuXG4gICAgICBhdHRyOiB7IHRhYmluZGV4OiAnMCcsIHJvbGU6ICdsaW5rJyB9LFxuICAgIH0pO1xuICAgIC8vIEluZGVudGF0aW9uIGlzIGRyaXZlbiBieSBDU1Mgb2ZmIHRoaXMgYXR0cmlidXRlIOKAlCBubyBpbmxpbmUgc3R5bGVzLlxuICAgIGl0ZW0uZGF0YXNldC5sZXZlbCA9IFN0cmluZyhub2RlLmxldmVsKTtcblxuICAgIC8vIFRvZ2dsZSBzbG90IGlzIGFsd2F5cyBwcmVzZW50IChldmVuIGxlYWYgaGVhZGluZ3MpIHNvIGhlYWRpbmcgdGV4dFxuICAgIC8vIGxpbmVzIHVwIHJlZ2FyZGxlc3Mgb2Ygd2hldGhlciBhIGNoZXZyb24gaXMgc2hvd24uXG4gICAgY29uc3QgdG9nZ2xlID0gaXRlbS5jcmVhdGVTcGFuKHsgY2xzOiAncW1kLW91dGxpbmUtdG9nZ2xlJyB9KTtcbiAgICBpdGVtLmNyZWF0ZVNwYW4oeyBjbHM6ICdxbWQtb3V0bGluZS1pdGVtLXRleHQnLCB0ZXh0OiBub2RlLnRleHQgfSk7XG5cbiAgICBsZXQgY2hpbGRyZW5FbDogSFRNTEVsZW1lbnQgfCBudWxsID0gbnVsbDtcbiAgICBpZiAoaGFzQ2hpbGRyZW4pIHtcbiAgICAgIHNldEljb24odG9nZ2xlLCAnY2hldnJvbi1kb3duJyk7XG4gICAgICB0b2dnbGUuYWRkQ2xhc3MoJ2lzLWNsaWNrYWJsZScpO1xuICAgICAgY2hpbGRyZW5FbCA9IHBhcmVudEVsLmNyZWF0ZURpdih7IGNsczogJ3FtZC1vdXRsaW5lLWNoaWxkcmVuJyB9KTtcbiAgICAgIGZvciAoY29uc3QgY2hpbGQgb2Ygbm9kZS5jaGlsZHJlbikge1xuICAgICAgICB0aGlzLnJlbmRlck5vZGUoY2hpbGRyZW5FbCwgY2hpbGQsIGZpbGUsIHBhdGgpO1xuICAgICAgfVxuICAgIH1cblxuICAgIGNvbnN0IGFwcGx5Rm9sZCA9ICgpID0+IHtcbiAgICAgIGl0ZW0udG9nZ2xlQ2xhc3MoJ2lzLWNvbGxhcHNlZCcsIGlzQ29sbGFwc2VkKTtcbiAgICAgIGNoaWxkcmVuRWw/LnRvZ2dsZUNsYXNzKCdpcy1jb2xsYXBzZWQnLCBpc0NvbGxhcHNlZCk7XG4gICAgfTtcbiAgICBjb25zdCBzZXRGb2xkID0gKGNvbGxhcHNlOiBib29sZWFuKSA9PiB7XG4gICAgICBpc0NvbGxhcHNlZCA9IGNvbGxhcHNlO1xuICAgICAgaWYgKGNvbGxhcHNlKSB0aGlzLmNvbGxhcHNlZC5hZGQocGF0aCk7XG4gICAgICBlbHNlIHRoaXMuY29sbGFwc2VkLmRlbGV0ZShwYXRoKTtcbiAgICAgIGFwcGx5Rm9sZCgpO1xuICAgIH07XG4gICAgYXBwbHlGb2xkKCk7IC8vIHJlZmxlY3QgYW55IHBlcnNpc3RlZCBmb2xkIHN0YXRlIG9uIGZpcnN0IHBhaW50XG5cbiAgICBjb25zdCBqdW1wVG8gPSAoKSA9PiB7XG4gICAgICAvLyBSZXNvbHZlIHRoZSBlZGl0b3IgYnkgZmlsZSwgbm90IGJ5IFwiYWN0aXZlIGxlYWZcIiDigJQgdGhlIGNsaWNrIGl0c2VsZlxuICAgICAgLy8ganVzdCBtb3ZlZCBmb2N1cyB0byB0aGlzIHNpZGViYXIuXG4gICAgICBjb25zdCB2aWV3ID0gdGhpcy5tYXJrZG93blZpZXdGb3IoZmlsZSk7XG4gICAgICBpZiAoIXZpZXcpIHJldHVybjtcbiAgICAgIGNvbnN0IHBvcyA9IHsgbGluZTogbm9kZS5saW5lLCBjaDogMCB9O1xuICAgICAgdGhpcy5hcHAud29ya3NwYWNlLnNldEFjdGl2ZUxlYWYodmlldy5sZWFmLCB7IGZvY3VzOiB0cnVlIH0pO1xuICAgICAgdmlldy5lZGl0b3Iuc2V0Q3Vyc29yKHBvcyk7XG4gICAgICB2aWV3LmVkaXRvci5zY3JvbGxJbnRvVmlldyh7IGZyb206IHBvcywgdG86IHBvcyB9LCB0cnVlKTtcbiAgICAgIHZpZXcuZWRpdG9yLmZvY3VzKCk7XG4gICAgfTtcblxuICAgIGl0ZW0uYWRkRXZlbnRMaXN0ZW5lcignY2xpY2snLCBqdW1wVG8pO1xuICAgIGl0ZW0uYWRkRXZlbnRMaXN0ZW5lcigna2V5ZG93bicsIChldnQpID0+IHtcbiAgICAgIGlmIChldnQua2V5ID09PSAnRW50ZXInIHx8IGV2dC5rZXkgPT09ICcgJykge1xuICAgICAgICBldnQucHJldmVudERlZmF1bHQoKTtcbiAgICAgICAganVtcFRvKCk7XG4gICAgICB9IGVsc2UgaWYgKGhhc0NoaWxkcmVuICYmIGV2dC5rZXkgPT09ICdBcnJvd1JpZ2h0JyAmJiBpc0NvbGxhcHNlZCkge1xuICAgICAgICBldnQucHJldmVudERlZmF1bHQoKTtcbiAgICAgICAgc2V0Rm9sZChmYWxzZSk7XG4gICAgICB9IGVsc2UgaWYgKGhhc0NoaWxkcmVuICYmIGV2dC5rZXkgPT09ICdBcnJvd0xlZnQnICYmICFpc0NvbGxhcHNlZCkge1xuICAgICAgICBldnQucHJldmVudERlZmF1bHQoKTtcbiAgICAgICAgc2V0Rm9sZCh0cnVlKTtcbiAgICAgIH1cbiAgICB9KTtcblxuICAgIGlmIChoYXNDaGlsZHJlbikge1xuICAgICAgdG9nZ2xlLmFkZEV2ZW50TGlzdGVuZXIoJ2NsaWNrJywgKGV2dCkgPT4ge1xuICAgICAgICAvLyBGb2xkIGluc3RlYWQgb2YganVtcGluZzsgdGhlIGNsaWNrIHdvdWxkIG90aGVyd2lzZSBidWJibGUgdG8gdGhlXG4gICAgICAgIC8vIHJvdydzIGp1bXAgaGFuZGxlci5cbiAgICAgICAgZXZ0LnN0b3BQcm9wYWdhdGlvbigpO1xuICAgICAgICBzZXRGb2xkKCFpc0NvbGxhcHNlZCk7XG4gICAgICB9KTtcbiAgICB9XG4gIH1cbn1cbiIsImltcG9ydCB7IEVkaXRvclN0YXRlLCBSYW5nZVNldEJ1aWxkZXIsIFN0YXRlRmllbGQsIFRleHQgfSBmcm9tICdAY29kZW1pcnJvci9zdGF0ZSc7XG5pbXBvcnQgeyBEZWNvcmF0aW9uLCBEZWNvcmF0aW9uU2V0LCBFZGl0b3JWaWV3LCBrZXltYXAgfSBmcm9tICdAY29kZW1pcnJvci92aWV3JztcbmltcG9ydCB7IGluZGVudE1vcmUsIGluZGVudExlc3MgfSBmcm9tICdAY29kZW1pcnJvci9jb21tYW5kcyc7XG5pbXBvcnQgeyBUZXh0RmlsZVZpZXcsIFdvcmtzcGFjZUxlYWYgfSBmcm9tICdvYnNpZGlhbic7XG5cbmV4cG9ydCBjb25zdCBRTURfWUFNTF9WSUVXID0gJ3FtZC15YW1sLXZpZXcnO1xuZXhwb3J0IGNvbnN0IFFNRF9MVUFfVklFVyA9ICdxbWQtbHVhLXZpZXcnO1xuXG5jb25zdCB5YW1sSGlnaGxpZ2h0RmllbGQgPSBTdGF0ZUZpZWxkLmRlZmluZTxEZWNvcmF0aW9uU2V0Pih7XG4gIGNyZWF0ZShzdGF0ZSk6IERlY29yYXRpb25TZXQge1xuICAgIHJldHVybiBidWlsZFlhbWxEZWNvcmF0aW9ucyhzdGF0ZS5kb2MpO1xuICB9LFxuICB1cGRhdGUoZGVjb3JhdGlvbnMsIHRyYW5zYWN0aW9uKTogRGVjb3JhdGlvblNldCB7XG4gICAgaWYgKHRyYW5zYWN0aW9uLmRvY0NoYW5nZWQpIHtcbiAgICAgIHJldHVybiBidWlsZFlhbWxEZWNvcmF0aW9ucyh0cmFuc2FjdGlvbi5zdGF0ZS5kb2MpO1xuICAgIH1cbiAgICByZXR1cm4gZGVjb3JhdGlvbnMubWFwKHRyYW5zYWN0aW9uLmNoYW5nZXMpO1xuICB9LFxuICBwcm92aWRlOiAoZmllbGQpID0+IEVkaXRvclZpZXcuZGVjb3JhdGlvbnMuZnJvbShmaWVsZCksXG59KTtcblxuZnVuY3Rpb24gYnVpbGRZYW1sRGVjb3JhdGlvbnMoZG9jOiBUZXh0KTogRGVjb3JhdGlvblNldCB7XG4gIGNvbnN0IGJ1aWxkZXIgPSBuZXcgUmFuZ2VTZXRCdWlsZGVyPERlY29yYXRpb24+KCk7XG4gIGZvciAobGV0IGxpbmVOdW1iZXIgPSAxOyBsaW5lTnVtYmVyIDw9IGRvYy5saW5lczsgbGluZU51bWJlcisrKSB7XG4gICAgY29uc3QgbGluZSA9IGRvYy5saW5lKGxpbmVOdW1iZXIpO1xuICAgIGRlY29yYXRlWWFtbExpbmUoYnVpbGRlciwgbGluZS5mcm9tLCBsaW5lLnRleHQpO1xuICB9XG4gIHJldHVybiBidWlsZGVyLmZpbmlzaCgpO1xufVxuXG5mdW5jdGlvbiBkZWNvcmF0ZVlhbWxMaW5lKGJ1aWxkZXI6IFJhbmdlU2V0QnVpbGRlcjxEZWNvcmF0aW9uPiwgbGluZVN0YXJ0OiBudW1iZXIsIGxpbmU6IHN0cmluZyk6IHZvaWQge1xuICBjb25zdCBpbmRlbnQgPSBsaW5lLm1hdGNoKC9eXFxzKi8pPy5bMF0gPz8gJyc7XG4gIGNvbnN0IGNvbnRlbnQgPSBsaW5lLnNsaWNlKGluZGVudC5sZW5ndGgpO1xuICBjb25zdCBjb250ZW50U3RhcnQgPSBsaW5lU3RhcnQgKyBpbmRlbnQubGVuZ3RoO1xuICBpZiAoIWNvbnRlbnQpIHJldHVybjtcblxuICBpZiAoY29udGVudC5zdGFydHNXaXRoKCcjJykpIHtcbiAgICBtYXJrWWFtbFRva2VuKGJ1aWxkZXIsIGNvbnRlbnRTdGFydCwgbGluZVN0YXJ0ICsgbGluZS5sZW5ndGgsICdxbWQteWFtbC1jb21tZW50Jyk7XG4gICAgcmV0dXJuO1xuICB9XG5cbiAgY29uc3QgbWFya2VyTWF0Y2ggPSBjb250ZW50Lm1hdGNoKC9eKC1cXHMrKSguKikkLyk7XG4gIGlmIChtYXJrZXJNYXRjaCkge1xuICAgIGNvbnN0IG1hcmtlckxlbmd0aCA9IG1hcmtlck1hdGNoWzFdLmxlbmd0aDtcbiAgICBtYXJrWWFtbFRva2VuKGJ1aWxkZXIsIGNvbnRlbnRTdGFydCwgY29udGVudFN0YXJ0ICsgbWFya2VyTGVuZ3RoLCAncW1kLXlhbWwtbGlzdC1tYXJrZXInKTtcbiAgICBkZWNvcmF0ZVlhbWxTZWdtZW50KGJ1aWxkZXIsIGNvbnRlbnRTdGFydCArIG1hcmtlckxlbmd0aCwgbWFya2VyTWF0Y2hbMl0pO1xuICAgIHJldHVybjtcbiAgfVxuXG4gIGRlY29yYXRlWWFtbFNlZ21lbnQoYnVpbGRlciwgY29udGVudFN0YXJ0LCBjb250ZW50KTtcbn1cblxuZnVuY3Rpb24gZGVjb3JhdGVZYW1sU2VnbWVudChidWlsZGVyOiBSYW5nZVNldEJ1aWxkZXI8RGVjb3JhdGlvbj4sIHNlZ21lbnRTdGFydDogbnVtYmVyLCBzZWdtZW50OiBzdHJpbmcpOiB2b2lkIHtcbiAgY29uc3QgZG9jTWF0Y2ggPSBzZWdtZW50Lm1hdGNoKC9eKFxcLnszfXwtezN9KShcXHMqKCMuKik/KSQvKTtcbiAgaWYgKGRvY01hdGNoKSB7XG4gICAgY29uc3QgbWFya2VyTGVuZ3RoID0gZG9jTWF0Y2hbMV0ubGVuZ3RoO1xuICAgIG1hcmtZYW1sVG9rZW4oYnVpbGRlciwgc2VnbWVudFN0YXJ0LCBzZWdtZW50U3RhcnQgKyBtYXJrZXJMZW5ndGgsICdxbWQteWFtbC1kb2MtbWFya2VyJyk7XG4gICAgZGVjb3JhdGVZYW1sVmFsdWUoYnVpbGRlciwgc2VnbWVudFN0YXJ0ICsgbWFya2VyTGVuZ3RoLCBkb2NNYXRjaFsyXSA/PyAnJyk7XG4gICAgcmV0dXJuO1xuICB9XG5cbiAgY29uc3QgY29sb24gPSBmaW5kWWFtbEtleUNvbG9uKHNlZ21lbnQpO1xuICBpZiAoY29sb24gIT09IC0xKSB7XG4gICAgbWFya1lhbWxUb2tlbihidWlsZGVyLCBzZWdtZW50U3RhcnQsIHNlZ21lbnRTdGFydCArIGNvbG9uLCAncW1kLXlhbWwta2V5Jyk7XG4gICAgbWFya1lhbWxUb2tlbihidWlsZGVyLCBzZWdtZW50U3RhcnQgKyBjb2xvbiwgc2VnbWVudFN0YXJ0ICsgY29sb24gKyAxLCAncW1kLXlhbWwtY29sb24nKTtcbiAgICBkZWNvcmF0ZVlhbWxWYWx1ZShidWlsZGVyLCBzZWdtZW50U3RhcnQgKyBjb2xvbiArIDEsIHNlZ21lbnQuc2xpY2UoY29sb24gKyAxKSk7XG4gICAgcmV0dXJuO1xuICB9XG5cbiAgZGVjb3JhdGVZYW1sVmFsdWUoYnVpbGRlciwgc2VnbWVudFN0YXJ0LCBzZWdtZW50KTtcbn1cblxuZnVuY3Rpb24gZGVjb3JhdGVZYW1sVmFsdWUoYnVpbGRlcjogUmFuZ2VTZXRCdWlsZGVyPERlY29yYXRpb24+LCB2YWx1ZVN0YXJ0OiBudW1iZXIsIHZhbHVlOiBzdHJpbmcpOiB2b2lkIHtcbiAgY29uc3QgbGVhZGluZyA9IHZhbHVlLm1hdGNoKC9eXFxzKi8pPy5bMF0gPz8gJyc7XG4gIGNvbnN0IHJlc3QgPSB2YWx1ZS5zbGljZShsZWFkaW5nLmxlbmd0aCk7XG4gIGNvbnN0IHJlc3RTdGFydCA9IHZhbHVlU3RhcnQgKyBsZWFkaW5nLmxlbmd0aDtcbiAgaWYgKCFyZXN0KSByZXR1cm47XG5cbiAgY29uc3QgY29tbWVudEluZGV4ID0gZmluZFlhbWxDb21tZW50KHJlc3QpO1xuICBjb25zdCBzY2FsYXIgPSBjb21tZW50SW5kZXggPT09IC0xID8gcmVzdCA6IHJlc3Quc2xpY2UoMCwgY29tbWVudEluZGV4KTtcbiAgZGVjb3JhdGVZYW1sU2NhbGFyKGJ1aWxkZXIsIHJlc3RTdGFydCwgc2NhbGFyKTtcbiAgaWYgKGNvbW1lbnRJbmRleCAhPT0gLTEpIHtcbiAgICBtYXJrWWFtbFRva2VuKGJ1aWxkZXIsIHJlc3RTdGFydCArIGNvbW1lbnRJbmRleCwgcmVzdFN0YXJ0ICsgcmVzdC5sZW5ndGgsICdxbWQteWFtbC1jb21tZW50Jyk7XG4gIH1cbn1cblxuZnVuY3Rpb24gZGVjb3JhdGVZYW1sU2NhbGFyKGJ1aWxkZXI6IFJhbmdlU2V0QnVpbGRlcjxEZWNvcmF0aW9uPiwgc2NhbGFyU3RhcnQ6IG51bWJlciwgc2NhbGFyOiBzdHJpbmcpOiB2b2lkIHtcbiAgY29uc3QgdHJhaWxpbmcgPSBzY2FsYXIubWF0Y2goL1xccyokLyk/LlswXSA/PyAnJztcbiAgY29uc3QgdG9rZW4gPSB0cmFpbGluZyA/IHNjYWxhci5zbGljZSgwLCBzY2FsYXIubGVuZ3RoIC0gdHJhaWxpbmcubGVuZ3RoKSA6IHNjYWxhcjtcbiAgaWYgKCF0b2tlbikgcmV0dXJuO1xuXG4gIGNvbnN0IGNsYXNzTmFtZSA9IHlhbWxTY2FsYXJDbGFzcyh0b2tlbik7XG4gIG1hcmtZYW1sVG9rZW4oYnVpbGRlciwgc2NhbGFyU3RhcnQsIHNjYWxhclN0YXJ0ICsgdG9rZW4ubGVuZ3RoLCBjbGFzc05hbWUpO1xufVxuXG5mdW5jdGlvbiB5YW1sU2NhbGFyQ2xhc3ModG9rZW46IHN0cmluZyk6IHN0cmluZyB7XG4gIGlmICgvXlsnXCJdLipbJ1wiXSQvLnRlc3QodG9rZW4pKSByZXR1cm4gJ3FtZC15YW1sLXN0cmluZyc7XG4gIGlmICgvXlsmKl1bQS1aYS16MC05Xy1dKyQvLnRlc3QodG9rZW4pKSByZXR1cm4gJ3FtZC15YW1sLWFuY2hvcic7XG4gIC8vIFlBTUwgMS4yICh3aGF0IFF1YXJ0by9QYW5kb2MgdXNlKTogb25seSB0cnVlL2ZhbHNlL251bGwvfiBhcmVcbiAgLy8gYm9vbGVhbnMvbnVsbC4geWVzL25vL29uL29mZiBhcmUgcGxhaW4gc2NhbGFycywgbm90IGJvb2xlYW5zLlxuICBpZiAoL14odHJ1ZXxmYWxzZXxudWxsfH4pJC9pLnRlc3QodG9rZW4pKSByZXR1cm4gJ3FtZC15YW1sLWJvb2xlYW4nO1xuICBpZiAoL15bLStdPyg/OlxcZCtcXC4/XFxkKnxcXC5cXGQrKSg/OmVbLStdP1xcZCspPyQvaS50ZXN0KHRva2VuKSkgcmV0dXJuICdxbWQteWFtbC1udW1iZXInO1xuICBpZiAoL15bPnxdWystXT8kLy50ZXN0KHRva2VuKSkgcmV0dXJuICdxbWQteWFtbC1ibG9jayc7XG4gIGlmICgvXihodG1sfHBkZnx0eXBzdHxsYXRleHxiZWFtZXJ8cmV2ZWFsanN8ZG9jeHxvZHR8ZXB1YnxnZm18amF0c3xkYXNoYm9hcmQpJC9pLnRlc3QodG9rZW4pKSB7XG4gICAgcmV0dXJuICdxbWQteWFtbC1xdWFydG8tZm9ybWF0JztcbiAgfVxuICByZXR1cm4gJ3FtZC15YW1sLXNjYWxhcic7XG59XG5cbmZ1bmN0aW9uIGZpbmRZYW1sS2V5Q29sb24oc2VnbWVudDogc3RyaW5nKTogbnVtYmVyIHtcbiAgbGV0IHNpbmdsZVF1b3RlZCA9IGZhbHNlO1xuICBsZXQgZG91YmxlUXVvdGVkID0gZmFsc2U7XG4gIGZvciAobGV0IGkgPSAwOyBpIDwgc2VnbWVudC5sZW5ndGg7IGkrKykge1xuICAgIGNvbnN0IGNoYXIgPSBzZWdtZW50W2ldO1xuICAgIGNvbnN0IHByZXYgPSBpID4gMCA/IHNlZ21lbnRbaSAtIDFdIDogJyc7XG4gICAgaWYgKGNoYXIgPT09IFwiJ1wiICYmICFkb3VibGVRdW90ZWQpIHtcbiAgICAgIHNpbmdsZVF1b3RlZCA9ICFzaW5nbGVRdW90ZWQ7XG4gICAgfSBlbHNlIGlmIChjaGFyID09PSAnXCInICYmICFzaW5nbGVRdW90ZWQgJiYgcHJldiAhPT0gJ1xcXFwnKSB7XG4gICAgICBkb3VibGVRdW90ZWQgPSAhZG91YmxlUXVvdGVkO1xuICAgIH0gZWxzZSBpZiAoY2hhciA9PT0gJzonICYmICFzaW5nbGVRdW90ZWQgJiYgIWRvdWJsZVF1b3RlZCkge1xuICAgICAgY29uc3QgbmV4dCA9IHNlZ21lbnRbaSArIDFdID8/ICcnO1xuICAgICAgY29uc3Qga2V5ID0gc2VnbWVudC5zbGljZSgwLCBpKS50cmltKCk7XG4gICAgICBpZiAoa2V5ICYmICghbmV4dCB8fCAvXFxzLy50ZXN0KG5leHQpKSkgcmV0dXJuIGk7XG4gICAgfVxuICB9XG4gIHJldHVybiAtMTtcbn1cblxuZnVuY3Rpb24gZmluZFlhbWxDb21tZW50KHZhbHVlOiBzdHJpbmcpOiBudW1iZXIge1xuICBsZXQgc2luZ2xlUXVvdGVkID0gZmFsc2U7XG4gIGxldCBkb3VibGVRdW90ZWQgPSBmYWxzZTtcbiAgZm9yIChsZXQgaSA9IDA7IGkgPCB2YWx1ZS5sZW5ndGg7IGkrKykge1xuICAgIGNvbnN0IGNoYXIgPSB2YWx1ZVtpXTtcbiAgICBjb25zdCBwcmV2ID0gaSA+IDAgPyB2YWx1ZVtpIC0gMV0gOiAnJztcbiAgICBpZiAoY2hhciA9PT0gXCInXCIgJiYgIWRvdWJsZVF1b3RlZCkge1xuICAgICAgc2luZ2xlUXVvdGVkID0gIXNpbmdsZVF1b3RlZDtcbiAgICB9IGVsc2UgaWYgKGNoYXIgPT09ICdcIicgJiYgIXNpbmdsZVF1b3RlZCAmJiBwcmV2ICE9PSAnXFxcXCcpIHtcbiAgICAgIGRvdWJsZVF1b3RlZCA9ICFkb3VibGVRdW90ZWQ7XG4gICAgfSBlbHNlIGlmIChjaGFyID09PSAnIycgJiYgIXNpbmdsZVF1b3RlZCAmJiAhZG91YmxlUXVvdGVkICYmIChpID09PSAwIHx8IC9cXHMvLnRlc3QocHJldikpKSB7XG4gICAgICByZXR1cm4gaTtcbiAgICB9XG4gIH1cbiAgcmV0dXJuIC0xO1xufVxuXG5mdW5jdGlvbiBtYXJrWWFtbFRva2VuKFxuICBidWlsZGVyOiBSYW5nZVNldEJ1aWxkZXI8RGVjb3JhdGlvbj4sXG4gIGZyb206IG51bWJlcixcbiAgdG86IG51bWJlcixcbiAgY2xhc3NOYW1lOiBzdHJpbmdcbik6IHZvaWQge1xuICBpZiAodG8gPD0gZnJvbSkgcmV0dXJuO1xuICBidWlsZGVyLmFkZChmcm9tLCB0bywgRGVjb3JhdGlvbi5tYXJrKHsgY2xhc3M6IGNsYXNzTmFtZSB9KSk7XG59XG5cbi8vIC0tLSBMdWEgaGlnaGxpZ2h0aW5nIC0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tXG4vL1xuLy8gTWluaW1hbCBMdWEgc3ludGF4IGhpZ2hsaWdodGluZyBmb3IgdGhlIEx1YSBmaWxlIHZpZXcg4oCUIGVub3VnaCB0byBtYWtlXG4vLyBwYW5kb2MvUXVhcnRvIGZpbHRlciBzY3JpcHRzIHJlYWRhYmxlLiBBIHNpbmdsZSBmb3J3YXJkIHNjYW4gb3ZlciB0aGVcbi8vIHdob2xlIGRvY3VtZW50IHRleHQgbWFya3MgY29tbWVudHMsIHN0cmluZ3MsIG51bWJlcnMgYW5kIGtleXdvcmRzO1xuLy8gZXZlcnl0aGluZyBlbHNlIGlzIGxlZnQgdW5zdHlsZWQuIFRoZSBmb3J3YXJkIHNjYW4gZ3VhcmFudGVlcyB0aGVcbi8vIFJhbmdlU2V0QnVpbGRlciByZWNlaXZlcyByYW5nZXMgaW4gYXNjZW5kaW5nLCBub24tb3ZlcmxhcHBpbmcgb3JkZXIuXG5cbmNvbnN0IExVQV9LRVlXT1JEUyA9IG5ldyBTZXQoW1xuICAnYW5kJywgJ2JyZWFrJywgJ2RvJywgJ2Vsc2UnLCAnZWxzZWlmJywgJ2VuZCcsICdmYWxzZScsICdmb3InLCAnZnVuY3Rpb24nLFxuICAnZ290bycsICdpZicsICdpbicsICdsb2NhbCcsICduaWwnLCAnbm90JywgJ29yJywgJ3JlcGVhdCcsICdyZXR1cm4nLCAndGhlbicsXG4gICd0cnVlJywgJ3VudGlsJywgJ3doaWxlJyxcbl0pO1xuXG5jb25zdCBsdWFIaWdobGlnaHRGaWVsZCA9IFN0YXRlRmllbGQuZGVmaW5lPERlY29yYXRpb25TZXQ+KHtcbiAgY3JlYXRlKHN0YXRlKTogRGVjb3JhdGlvblNldCB7XG4gICAgcmV0dXJuIGJ1aWxkTHVhRGVjb3JhdGlvbnMoc3RhdGUuZG9jKTtcbiAgfSxcbiAgdXBkYXRlKGRlY29yYXRpb25zLCB0cmFuc2FjdGlvbik6IERlY29yYXRpb25TZXQge1xuICAgIGlmICh0cmFuc2FjdGlvbi5kb2NDaGFuZ2VkKSB7XG4gICAgICByZXR1cm4gYnVpbGRMdWFEZWNvcmF0aW9ucyh0cmFuc2FjdGlvbi5zdGF0ZS5kb2MpO1xuICAgIH1cbiAgICByZXR1cm4gZGVjb3JhdGlvbnMubWFwKHRyYW5zYWN0aW9uLmNoYW5nZXMpO1xuICB9LFxuICBwcm92aWRlOiAoZmllbGQpID0+IEVkaXRvclZpZXcuZGVjb3JhdGlvbnMuZnJvbShmaWVsZCksXG59KTtcblxuZnVuY3Rpb24gYnVpbGRMdWFEZWNvcmF0aW9ucyhkb2M6IFRleHQpOiBEZWNvcmF0aW9uU2V0IHtcbiAgY29uc3QgYnVpbGRlciA9IG5ldyBSYW5nZVNldEJ1aWxkZXI8RGVjb3JhdGlvbj4oKTtcbiAgY29uc3QgcyA9IGRvYy50b1N0cmluZygpO1xuICBjb25zdCBsZW4gPSBzLmxlbmd0aDtcblxuICBjb25zdCBtYXJrID0gKGZyb206IG51bWJlciwgdG86IG51bWJlciwgY2xhc3NOYW1lOiBzdHJpbmcpOiB2b2lkID0+IHtcbiAgICBpZiAodG8gPiBmcm9tKSBidWlsZGVyLmFkZChmcm9tLCB0bywgRGVjb3JhdGlvbi5tYXJrKHsgY2xhc3M6IGNsYXNzTmFtZSB9KSk7XG4gIH07XG5cbiAgLy8gSWYgYSBMdWEgbG9uZyBicmFja2V0IG9wZW5zIGF0IGBvcGVuYCAoYFtgLCBgWz1bYCwgYFs9PVtgLCDigKYpLCByZXR1cm4gdGhlXG4gIC8vIGluZGV4IGp1c3QgcGFzdCBpdHMgbWF0Y2hpbmcgY2xvc2U7IGFuIHVudGVybWluYXRlZCBicmFja2V0IHJ1bnMgdG8gRU9GLlxuICAvLyBSZXR1cm5zIC0xIHdoZW4gYG9wZW5gIGlzIG5vdCBhIGxvbmctYnJhY2tldCBvcGVuZXIuXG4gIGNvbnN0IGxvbmdCcmFja2V0RW5kID0gKG9wZW46IG51bWJlcik6IG51bWJlciA9PiB7XG4gICAgaWYgKHNbb3Blbl0gIT09ICdbJykgcmV0dXJuIC0xO1xuICAgIGxldCBqID0gb3BlbiArIDE7XG4gICAgbGV0IGxldmVsID0gMDtcbiAgICB3aGlsZSAoc1tqXSA9PT0gJz0nKSB7XG4gICAgICBsZXZlbCsrO1xuICAgICAgaisrO1xuICAgIH1cbiAgICBpZiAoc1tqXSAhPT0gJ1snKSByZXR1cm4gLTE7XG4gICAgY29uc3QgY2xvc2UgPSAnXScgKyAnPScucmVwZWF0KGxldmVsKSArICddJztcbiAgICBjb25zdCBpZHggPSBzLmluZGV4T2YoY2xvc2UsIGogKyAxKTtcbiAgICByZXR1cm4gaWR4ID09PSAtMSA/IGxlbiA6IGlkeCArIGNsb3NlLmxlbmd0aDtcbiAgfTtcblxuICBsZXQgaSA9IDA7XG4gIHdoaWxlIChpIDwgbGVuKSB7XG4gICAgY29uc3QgYyA9IHNbaV07XG5cbiAgICAvLyBjb21tZW50OiBsaW5lIChgLS0g4oCmYCkgb3IgYmxvY2sgKGAtLVtbIOKApiBdXWAsIGAtLVs9PVsg4oCmIF09PV1gKVxuICAgIGlmIChjID09PSAnLScgJiYgc1tpICsgMV0gPT09ICctJykge1xuICAgICAgY29uc3QgYmxvY2sgPSBsb25nQnJhY2tldEVuZChpICsgMik7XG4gICAgICBpZiAoYmxvY2sgIT09IC0xKSB7XG4gICAgICAgIG1hcmsoaSwgYmxvY2ssICdxbWQtbHVhLWNvbW1lbnQnKTtcbiAgICAgICAgaSA9IGJsb2NrO1xuICAgICAgICBjb250aW51ZTtcbiAgICAgIH1cbiAgICAgIGxldCBqID0gaSArIDI7XG4gICAgICB3aGlsZSAoaiA8IGxlbiAmJiBzW2pdICE9PSAnXFxuJykgaisrO1xuICAgICAgbWFyayhpLCBqLCAncW1kLWx1YS1jb21tZW50Jyk7XG4gICAgICBpID0gajtcbiAgICAgIGNvbnRpbnVlO1xuICAgIH1cblxuICAgIC8vIGxvbmctYnJhY2tldCBzdHJpbmdcbiAgICBpZiAoYyA9PT0gJ1snKSB7XG4gICAgICBjb25zdCBsb25nID0gbG9uZ0JyYWNrZXRFbmQoaSk7XG4gICAgICBpZiAobG9uZyAhPT0gLTEpIHtcbiAgICAgICAgbWFyayhpLCBsb25nLCAncW1kLWx1YS1zdHJpbmcnKTtcbiAgICAgICAgaSA9IGxvbmc7XG4gICAgICAgIGNvbnRpbnVlO1xuICAgICAgfVxuICAgIH1cblxuICAgIC8vIHF1b3RlZCBzdHJpbmcgKHNpbmdsZSBvciBkb3VibGUpOyBkb2VzIG5vdCBzcGFuIGxpbmVzXG4gICAgaWYgKGMgPT09ICdcIicgfHwgYyA9PT0gXCInXCIpIHtcbiAgICAgIGxldCBqID0gaSArIDE7XG4gICAgICB3aGlsZSAoaiA8IGxlbiAmJiBzW2pdICE9PSBjICYmIHNbal0gIT09ICdcXG4nKSB7XG4gICAgICAgIGlmIChzW2pdID09PSAnXFxcXCcpIGorKztcbiAgICAgICAgaisrO1xuICAgICAgfVxuICAgICAgaWYgKGogPCBsZW4gJiYgc1tqXSA9PT0gYykgaisrO1xuICAgICAgbWFyayhpLCBqLCAncW1kLWx1YS1zdHJpbmcnKTtcbiAgICAgIGkgPSBqO1xuICAgICAgY29udGludWU7XG4gICAgfVxuXG4gICAgLy8gbnVtYmVyIChkZWNpbWFsLCBoZXgsIGZyYWN0aW9uYWwsIGV4cG9uZW50KVxuICAgIGlmICgvWzAtOV0vLnRlc3QoYykgfHwgKGMgPT09ICcuJyAmJiAvWzAtOV0vLnRlc3Qoc1tpICsgMV0gPz8gJycpKSkge1xuICAgICAgbGV0IGogPSBpO1xuICAgICAgaWYgKGMgPT09ICcwJyAmJiAoc1tpICsgMV0gPT09ICd4JyB8fCBzW2kgKyAxXSA9PT0gJ1gnKSkge1xuICAgICAgICBqID0gaSArIDI7XG4gICAgICAgIHdoaWxlIChqIDwgbGVuICYmIC9bMC05YS1mQS1GLl0vLnRlc3Qoc1tqXSkpIGorKztcbiAgICAgIH0gZWxzZSB7XG4gICAgICAgIHdoaWxlIChqIDwgbGVuICYmIC9bMC05Ll0vLnRlc3Qoc1tqXSkpIGorKztcbiAgICAgICAgaWYgKHNbal0gPT09ICdlJyB8fCBzW2pdID09PSAnRScpIHtcbiAgICAgICAgICBqKys7XG4gICAgICAgICAgaWYgKHNbal0gPT09ICcrJyB8fCBzW2pdID09PSAnLScpIGorKztcbiAgICAgICAgICB3aGlsZSAoaiA8IGxlbiAmJiAvWzAtOV0vLnRlc3Qoc1tqXSkpIGorKztcbiAgICAgICAgfVxuICAgICAgfVxuICAgICAgbWFyayhpLCBqLCAncW1kLWx1YS1udW1iZXInKTtcbiAgICAgIGkgPSBqO1xuICAgICAgY29udGludWU7XG4gICAgfVxuXG4gICAgLy8gaWRlbnRpZmllciDigJQgb25seSBrZXl3b3JkcyBnZXQgbWFya2VkXG4gICAgaWYgKC9bQS1aYS16X10vLnRlc3QoYykpIHtcbiAgICAgIGxldCBqID0gaSArIDE7XG4gICAgICB3aGlsZSAoaiA8IGxlbiAmJiAvW0EtWmEtejAtOV9dLy50ZXN0KHNbal0pKSBqKys7XG4gICAgICBpZiAoTFVBX0tFWVdPUkRTLmhhcyhzLnNsaWNlKGksIGopKSkgbWFyayhpLCBqLCAncW1kLWx1YS1rZXl3b3JkJyk7XG4gICAgICBpID0gajtcbiAgICAgIGNvbnRpbnVlO1xuICAgIH1cblxuICAgIGkrKztcbiAgfVxuXG4gIHJldHVybiBidWlsZGVyLmZpbmlzaCgpO1xufVxuXG5pbnRlcmZhY2UgQ29kZVZpZXdDb25maWcge1xuICBsYWJlbDogc3RyaW5nO1xuICBhcmlhTGFiZWw6IHN0cmluZztcbiAgaGlnaGxpZ2h0RmllbGQ6IFN0YXRlRmllbGQ8RGVjb3JhdGlvblNldD47XG59XG5cbmNvbnN0IFlBTUxfQ09ERV9WSUVXOiBDb2RlVmlld0NvbmZpZyA9IHtcbiAgbGFiZWw6ICdZQU1MIGZpbGUnLFxuICBhcmlhTGFiZWw6ICdZQU1MIGZpbGUgY29udGVudHMnLFxuICBoaWdobGlnaHRGaWVsZDogeWFtbEhpZ2hsaWdodEZpZWxkLFxufTtcblxuY29uc3QgTFVBX0NPREVfVklFVzogQ29kZVZpZXdDb25maWcgPSB7XG4gIGxhYmVsOiAnTHVhIGZpbGUnLFxuICBhcmlhTGFiZWw6ICdMdWEgZmlsZSBjb250ZW50cycsXG4gIGhpZ2hsaWdodEZpZWxkOiBsdWFIaWdobGlnaHRGaWVsZCxcbn07XG5cbi8vIEEgbWluaW1hbCBDb2RlTWlycm9yLWJhY2tlZCBmaWxlIHZpZXcsIHNoYXJlZCBieSB0aGUgWUFNTCBhbmQgTHVhIGZpbGVcbi8vIHZpZXdzLiBUaGUgb25seSBwZXItbGFuZ3VhZ2UgZGlmZmVyZW5jZSBpcyB0aGUgaGlnaGxpZ2h0IFN0YXRlRmllbGQgYW5kXG4vLyB0aGUgbGFiZWxzLCBzdXBwbGllZCB0aHJvdWdoIENvZGVWaWV3Q29uZmlnLlxuLy9cbi8vIGdldFZpZXdUeXBlKCkgaXMgbGVmdCBhYnN0cmFjdCBvbiBwdXJwb3NlOiBPYnNpZGlhbidzIFZpZXcgY29uc3RydWN0b3Jcbi8vIGNhbGxzIGdldFZpZXdUeXBlKCkgKmR1cmluZyogc3VwZXIoKSwgYmVmb3JlIHN1YmNsYXNzIGNvbnN0cnVjdG9yIHBhcmFtc1xuLy8gYW5kIGZpZWxkIGluaXRpYWxpemVycyBoYXZlIHJ1biwgc28gaXQgY2Fubm90IHJlYWQgaW5zdGFuY2Ugc3RhdGUuIEVhY2hcbi8vIGNvbmNyZXRlIHN1YmNsYXNzIHJldHVybnMgYSBtb2R1bGUtbGV2ZWwgbGl0ZXJhbCBpbnN0ZWFkLlxuYWJzdHJhY3QgY2xhc3MgUW1kQ29kZUZpbGVWaWV3IGV4dGVuZHMgVGV4dEZpbGVWaWV3IHtcbiAgcHJpdmF0ZSBlZGl0b3JWaWV3OiBFZGl0b3JWaWV3IHwgbnVsbCA9IG51bGw7XG4gIHByaXZhdGUgc2V0dGluZ1ZpZXdEYXRhID0gZmFsc2U7XG5cbiAgY29uc3RydWN0b3IobGVhZjogV29ya3NwYWNlTGVhZiwgcHJpdmF0ZSByZWFkb25seSBjb25maWc6IENvZGVWaWV3Q29uZmlnKSB7XG4gICAgc3VwZXIobGVhZik7XG4gIH1cblxuICBhYnN0cmFjdCBnZXRWaWV3VHlwZSgpOiBzdHJpbmc7XG5cbiAgZ2V0RGlzcGxheVRleHQoKTogc3RyaW5nIHtcbiAgICByZXR1cm4gdGhpcy5maWxlPy5uYW1lID8/IHRoaXMuY29uZmlnLmxhYmVsO1xuICB9XG5cbiAgZ2V0SWNvbigpOiBzdHJpbmcge1xuICAgIHJldHVybiAnZmlsZS1jb2RlJztcbiAgfVxuXG4gIG9ubG9hZCgpOiB2b2lkIHtcbiAgICBzdXBlci5vbmxvYWQoKTtcbiAgICB0aGlzLmNvbnRlbnRFbC5lbXB0eSgpO1xuICAgIHRoaXMuY29udGVudEVsLmFkZENsYXNzKCdxbWQtY29kZS12aWV3Jyk7XG4gICAgdGhpcy5lZGl0b3JWaWV3ID0gbmV3IEVkaXRvclZpZXcoe1xuICAgICAgcGFyZW50OiB0aGlzLmNvbnRlbnRFbCxcbiAgICAgIHN0YXRlOiBFZGl0b3JTdGF0ZS5jcmVhdGUoe1xuICAgICAgICBkb2M6IHRoaXMuZGF0YSA/PyAnJyxcbiAgICAgICAgZXh0ZW5zaW9uczogW1xuICAgICAgICAgIEVkaXRvclN0YXRlLnRhYlNpemUub2YoMiksXG4gICAgICAgICAgdGhpcy5jb25maWcuaGlnaGxpZ2h0RmllbGQsXG4gICAgICAgICAgRWRpdG9yVmlldy5jb250ZW50QXR0cmlidXRlcy5vZih7XG4gICAgICAgICAgICAnYXJpYS1sYWJlbCc6IHRoaXMuY29uZmlnLmFyaWFMYWJlbCxcbiAgICAgICAgICAgIGF1dG9jYXBpdGFsaXplOiAnb2ZmJyxcbiAgICAgICAgICAgIGF1dG9jb21wbGV0ZTogJ29mZicsXG4gICAgICAgICAgICBzcGVsbGNoZWNrOiAnZmFsc2UnLFxuICAgICAgICAgIH0pLFxuICAgICAgICAgIGtleW1hcC5vZihbXG4gICAgICAgICAgICB7IGtleTogJ1RhYicsIHJ1bjogaW5kZW50TW9yZSB9LFxuICAgICAgICAgICAgeyBrZXk6ICdTaGlmdC1UYWInLCBydW46IGluZGVudExlc3MgfSxcbiAgICAgICAgICBdKSxcbiAgICAgICAgICBFZGl0b3JWaWV3LnVwZGF0ZUxpc3RlbmVyLm9mKCh1cGRhdGUpID0+IHtcbiAgICAgICAgICAgIGlmICghdXBkYXRlLmRvY0NoYW5nZWQgfHwgdGhpcy5zZXR0aW5nVmlld0RhdGEpIHJldHVybjtcbiAgICAgICAgICAgIHRoaXMuZGF0YSA9IHVwZGF0ZS5zdGF0ZS5kb2MudG9TdHJpbmcoKTtcbiAgICAgICAgICAgIHRoaXMucmVxdWVzdFNhdmUoKTtcbiAgICAgICAgICB9KSxcbiAgICAgICAgXSxcbiAgICAgIH0pLFxuICAgIH0pO1xuICAgIHRoaXMucmVnaXN0ZXIoKCkgPT4ge1xuICAgICAgdGhpcy5lZGl0b3JWaWV3Py5kZXN0cm95KCk7XG4gICAgICB0aGlzLmVkaXRvclZpZXcgPSBudWxsO1xuICAgIH0pO1xuICB9XG5cbiAgZ2V0Vmlld0RhdGEoKTogc3RyaW5nIHtcbiAgICByZXR1cm4gdGhpcy5lZGl0b3JWaWV3Py5zdGF0ZS5kb2MudG9TdHJpbmcoKSA/PyB0aGlzLmRhdGEgPz8gJyc7XG4gIH1cblxuICBzZXRWaWV3RGF0YShkYXRhOiBzdHJpbmcpOiB2b2lkIHtcbiAgICB0aGlzLmRhdGEgPSBkYXRhO1xuICAgIGNvbnN0IHZpZXcgPSB0aGlzLmVkaXRvclZpZXc7XG4gICAgaWYgKCF2aWV3KSByZXR1cm47XG4gICAgY29uc3QgY3VycmVudCA9IHZpZXcuc3RhdGUuZG9jLnRvU3RyaW5nKCk7XG4gICAgaWYgKGN1cnJlbnQgPT09IGRhdGEpIHJldHVybjtcbiAgICB0aGlzLnNldHRpbmdWaWV3RGF0YSA9IHRydWU7XG4gICAgdHJ5IHtcbiAgICAgIHZpZXcuZGlzcGF0Y2goe1xuICAgICAgICBjaGFuZ2VzOiB7XG4gICAgICAgICAgZnJvbTogMCxcbiAgICAgICAgICB0bzogY3VycmVudC5sZW5ndGgsXG4gICAgICAgICAgaW5zZXJ0OiBkYXRhLFxuICAgICAgICB9LFxuICAgICAgfSk7XG4gICAgfSBmaW5hbGx5IHtcbiAgICAgIHRoaXMuc2V0dGluZ1ZpZXdEYXRhID0gZmFsc2U7XG4gICAgfVxuICB9XG5cbiAgY2xlYXIoKTogdm9pZCB7XG4gICAgdGhpcy5zZXRWaWV3RGF0YSgnJyk7XG4gIH1cbn1cblxuZXhwb3J0IGNsYXNzIFFtZFlhbWxGaWxlVmlldyBleHRlbmRzIFFtZENvZGVGaWxlVmlldyB7XG4gIGNvbnN0cnVjdG9yKGxlYWY6IFdvcmtzcGFjZUxlYWYpIHtcbiAgICBzdXBlcihsZWFmLCBZQU1MX0NPREVfVklFVyk7XG4gIH1cblxuICBnZXRWaWV3VHlwZSgpOiBzdHJpbmcge1xuICAgIHJldHVybiBRTURfWUFNTF9WSUVXO1xuICB9XG59XG5cbmV4cG9ydCBjbGFzcyBRbWRMdWFGaWxlVmlldyBleHRlbmRzIFFtZENvZGVGaWxlVmlldyB7XG4gIGNvbnN0cnVjdG9yKGxlYWY6IFdvcmtzcGFjZUxlYWYpIHtcbiAgICBzdXBlcihsZWFmLCBMVUFfQ09ERV9WSUVXKTtcbiAgfVxuXG4gIGdldFZpZXdUeXBlKCk6IHN0cmluZyB7XG4gICAgcmV0dXJuIFFNRF9MVUFfVklFVztcbiAgfVxufVxuIiwiaW1wb3J0IHtcbiAgQXBwLFxuICBNb2RhbCxcbiAgTm90aWNlLFxuICBTZXR0aW5nLFxuICBTdWdnZXN0TW9kYWwsXG4gIFRGaWxlLFxuICBURm9sZGVyLFxuICBub3JtYWxpemVQYXRoLFxufSBmcm9tICdvYnNpZGlhbic7XG5cbnR5cGUgUHJlc2V0U291cmNlID0gJ2J1aWx0LWluJyB8ICd1c2VyJztcblxuZXhwb3J0IGludGVyZmFjZSBOZXdGaWxlUHJlc2V0IHtcbiAgaWQ6IHN0cmluZztcbiAgbmFtZTogc3RyaW5nO1xuICBkZXNjcmlwdGlvbjogc3RyaW5nO1xuICBib2R5OiBzdHJpbmc7XG4gIHNvdXJjZTogUHJlc2V0U291cmNlO1xufVxuXG4vLyBTaGFyZWQgVHlwc3Qgc3R5bGluZyBpbmplY3RlZCBpbnRvIGJvdGggYnVpbHQtaW4gVHlwc3QgcHJlc2V0cyB2aWFcbi8vIGBpbmNsdWRlLWluLWhlYWRlcmAuIEtlcHQgYXMgb25lIGNvbnN0YW50IHNvIHRoZSB0d28gcHJlc2V0cyBjYW5ub3QgZHJpZnRcbi8vIG91dCBvZiBzeW5jLiBJbmRlbnRhdGlvbiBtYXRjaGVzIHRoZSBZQU1MIHBvc2l0aW9uIHdoZXJlIGl0IGlzIGludGVycG9sYXRlZFxuLy8gKDEwIHNwYWNlcywgc2l0dGluZyBpbnNpZGUgYCAgICBpbmNsdWRlLWluLWhlYWRlcjpgIOKGkiBgICAgICAgLSB0ZXh0OiB8YCkuXG5jb25zdCBUWVBTVF9TVFlMSU5HX0hFQURFUiA9IGAgICAgaW5jbHVkZS1pbi1oZWFkZXI6XG4gICAgICAtIHRleHQ6IHxcbiAgICAgICAgICAvLyBTaW5nbGUgYWNjZW50IGNvbG9yIHVzZWQgdGhyb3VnaG91dFxuICAgICAgICAgICNsZXQgYWNjZW50ID0gcmdiKFwiIzJFNUM4QVwiKVxuXG4gICAgICAgICAgLy8gUGFnZSBoZWFkZXI6IGN1cnJlbnQgdG9wLWxldmVsIHNlY3Rpb24gKyBwYWdlIG51bWJlclxuICAgICAgICAgICNzZXQgcGFnZShoZWFkZXI6IGNvbnRleHQge1xuICAgICAgICAgICAgbGV0IGhlYWRzID0gcXVlcnkoc2VsZWN0b3IoaGVhZGluZy53aGVyZShsZXZlbDogMSkpLmJlZm9yZShoZXJlKCkpKVxuICAgICAgICAgICAgbGV0IHQgPSBpZiBoZWFkcy5sZW4oKSA+IDAgeyBoZWFkcy5sYXN0KCkuYm9keSB9IGVsc2UgWyBdXG4gICAgICAgICAgICB0ZXh0KHNpemU6IDlwdCwgZmlsbDogZ3JheSwgWyN0ICNoKDFmcikgI2NvdW50ZXIocGFnZSkuZGlzcGxheSgpXSlcbiAgICAgICAgICB9KVxuICAgICAgICAgIC8vIExpbmsgY29sb3JcbiAgICAgICAgICAjc2hvdyBsaW5rOiBzZXQgdGV4dChmaWxsOiBhY2NlbnQpXG4gICAgICAgICAgLy8gQm94ZWQgY29kZSBibG9ja3NcbiAgICAgICAgICAjc2hvdyByYXcud2hlcmUoYmxvY2s6IHRydWUpOiBpdCA9PiBibG9jayhcbiAgICAgICAgICAgIGZpbGw6IHJnYihcIiNmNWY1ZjVcIiksXG4gICAgICAgICAgICBpbnNldDogMTBwdCxcbiAgICAgICAgICAgIHJhZGl1czogNHB0LFxuICAgICAgICAgICAgd2lkdGg6IDEwMCUsXG4gICAgICAgICAgICBpdCxcbiAgICAgICAgICApXG4gICAgICAgICAgLy8gQmxvY2sgcXVvdGVzOiBjb2xvcmVkIGxlZnQgYmFyICsgbXV0ZWQgaXRhbGljXG4gICAgICAgICAgI3Nob3cgcXVvdGUud2hlcmUoYmxvY2s6IHRydWUpOiBpdCA9PiBibG9jayhcbiAgICAgICAgICAgIHN0cm9rZTogKGxlZnQ6IDNwdCArIGFjY2VudCksXG4gICAgICAgICAgICBpbnNldDogKGxlZnQ6IDEycHQsIHRvcDogNHB0LCBib3R0b206IDRwdCksXG4gICAgICAgICAgICB0ZXh0KHN0eWxlOiBcIml0YWxpY1wiLCBmaWxsOiBncmF5LmRhcmtlbigyMCUpLCBpdC5ib2R5KSxcbiAgICAgICAgICApXG4gICAgICAgICAgLy8gSDEgaGVhZGluZ3M6IHN1YnRsZSBhY2NlbnQgcnVsZVxuICAgICAgICAgICNzaG93IGhlYWRpbmcud2hlcmUobGV2ZWw6IDEpOiBpdCA9PiBbXG4gICAgICAgICAgICAjaXRcbiAgICAgICAgICAgICN2KC0wLjVlbSlcbiAgICAgICAgICAgICNsaW5lKGxlbmd0aDogMTAwJSwgc3Ryb2tlOiAwLjVwdCArIGFjY2VudC5saWdodGVuKDQwJSkpXG4gICAgICAgICAgXWA7XG5cbi8vIEJ1aWx0LWluIHByZXNldHMuIHYwOiBoYXJkLWNvZGVkOyBsYXRlciB2ZXJzaW9ucyB3aWxsIGxldCB1c2VycyBlZGl0L2FkZFxuLy8gdGhlc2UgaW4gcGx1Z2luIHNldHRpbmdzIChzYW1lIHNoYXBlLCBwZXJzaXN0ZWQgaW4gUW1kUGx1Z2luU2V0dGluZ3MpLlxuZXhwb3J0IGNvbnN0IERFRkFVTFRfUFJFU0VUUzogTmV3RmlsZVByZXNldFtdID0gW1xuICB7XG4gICAgaWQ6ICdlbXB0eScsXG4gICAgbmFtZTogJ0VtcHR5JyxcbiAgICBkZXNjcmlwdGlvbjogJ01pbmltYWwgZnJvbnQtbWF0dGVyLCBubyBmb3JtYXQgc3BlY2lmaWVkLicsXG4gICAgc291cmNlOiAnYnVpbHQtaW4nLFxuICAgIGJvZHk6IGAtLS1cbnRpdGxlOiBcIlwiXG4tLS1cblxuIyBVbnRpdGxlZFxuYCxcbiAgfSxcbiAge1xuICAgIGlkOiAnZG9jeCcsXG4gICAgbmFtZTogJ1dvcmQgKC5kb2N4KScsXG4gICAgZGVzY3JpcHRpb246ICdmb3JtYXQ6IGRvY3ggd2l0aCB0YWJsZSBvZiBjb250ZW50cyBhbmQgbnVtYmVyZWQgc2VjdGlvbnMuJyxcbiAgICBzb3VyY2U6ICdidWlsdC1pbicsXG4gICAgYm9keTogYC0tLVxudGl0bGU6IFwiXCJcbmF1dGhvcjogXCJcIlxuZGF0ZTogdG9kYXlcbmZvcm1hdDpcbiAgZG9jeDpcbiAgICB0b2M6IHRydWVcbiAgICBudW1iZXItc2VjdGlvbnM6IHRydWVcbiAgICAjIHJlZmVyZW5jZS1kb2M6IHJlZmVyZW5jZS5kb2N4XG4tLS1cblxuIyBVbnRpdGxlZFxuYCxcbiAgfSxcbiAge1xuICAgIGlkOiAndHlwc3Qtbm90ZXMnLFxuICAgIG5hbWU6ICdUeXBzdCBQREYg4oCUIE5vdGVzIChFaXN2b2dlbC1zdHlsZSknLFxuICAgIGRlc2NyaXB0aW9uOlxuICAgICAgJ0Vpc3ZvZ2VsLWluc3BpcmVkIFR5cHN0IFBERjogVE9DLCBudW1iZXJlZCBzZWN0aW9ucywgcGFnZSBoZWFkZXIsIGJveGVkIGNvZGUuJyxcbiAgICBzb3VyY2U6ICdidWlsdC1pbicsXG4gICAgYm9keTogYC0tLVxudGl0bGU6IFwiXCJcbmF1dGhvcjogXCJcIlxuZGF0ZTogdG9kYXlcbnRvYzogZmFsc2VcbnRvYy1kZXB0aDogMlxubnVtYmVyLXNlY3Rpb25zOiB0cnVlXG5mb3JtYXQ6XG4gIHR5cHN0OlxuICAgIHBhcGVyc2l6ZTogYTRcbiAgICBtYXJnaW46XG4gICAgICB4OiAyY21cbiAgICAgIHk6IDIuNWNtXG4gICAgZm9udHNpemU6IDExcHRcbiAgICBzZWN0aW9uLW51bWJlcmluZzogXCIxLjEuMVwiXG4gICAgIyBtYWluZm9udDogXCJMaWJlcnRpbnVzIFNlcmlmXCJcbiAgICAjIHNhbnNmb250OiBcIkxpYmVydGludXMgU2Fuc1wiXG4gICAgIyBtb25vZm9udDogXCJKZXRCcmFpbnMgTW9ub1wiXG4ke1RZUFNUX1NUWUxJTkdfSEVBREVSfVxuLS0tXG5cbiMgVW50aXRsZWRcbmAsXG4gIH0sXG4gIHtcbiAgICBpZDogJ3R5cHN0LXJlcG9ydCcsXG4gICAgbmFtZTogJ1R5cHN0IFBERiDigJQgUmVwb3J0IChFaXN2b2dlbC1zdHlsZSknLFxuICAgIGRlc2NyaXB0aW9uOlxuICAgICAgJ0Vpc3ZvZ2VsLWluc3BpcmVkIFR5cHN0IHJlcG9ydDogY292ZXIgbWV0YWRhdGEsIFRPQywgbnVtYmVyZWQgc2VjdGlvbnMsIGJveGVkIGNvZGUsIGJpYmxpb2dyYXBoeSBoaW50cy4nLFxuICAgIHNvdXJjZTogJ2J1aWx0LWluJyxcbiAgICBib2R5OiBgLS0tXG50aXRsZTogXCJcIlxuc3VidGl0bGU6IFwiXCJcbmF1dGhvcjogXCJcIlxuZGF0ZTogdG9kYXlcbiMgYWJzdHJhY3Q6IHxcbiMgICBTaG9ydCBzdW1tYXJ5IG9mIHRoZSByZXBvcnQuXG4jIGtleXdvcmRzOiBba2V5d29yZDEsIGtleXdvcmQyXVxuIyBiaWJsaW9ncmFwaHk6IHJlZmVyZW5jZXMuYmliXG4jIGNzbDogaWVlZS5jc2xcbnRvYzogdHJ1ZVxudG9jLWRlcHRoOiAzXG5udW1iZXItc2VjdGlvbnM6IHRydWVcbmZvcm1hdDpcbiAgdHlwc3Q6XG4gICAgcGFwZXJzaXplOiBhNFxuICAgIG1hcmdpbjpcbiAgICAgIHg6IDIuNWNtXG4gICAgICB5OiAyLjVjbVxuICAgIGZvbnRzaXplOiAxMXB0XG4gICAgc2VjdGlvbi1udW1iZXJpbmc6IFwiMS4xLjFcIlxuICAgICMgbWFpbmZvbnQ6IFwiTGliZXJ0aW51cyBTZXJpZlwiXG4gICAgIyBzYW5zZm9udDogXCJMaWJlcnRpbnVzIFNhbnNcIlxuICAgICMgbW9ub2ZvbnQ6IFwiSmV0QnJhaW5zIE1vbm9cIlxuJHtUWVBTVF9TVFlMSU5HX0hFQURFUn1cbi0tLVxuXG4jIFVudGl0bGVkXG5gLFxuICB9LFxuXTtcblxuY2xhc3MgUHJlc2V0U3VnZ2VzdE1vZGFsIGV4dGVuZHMgU3VnZ2VzdE1vZGFsPE5ld0ZpbGVQcmVzZXQ+IHtcbiAgY29uc3RydWN0b3IoXG4gICAgYXBwOiBBcHAsXG4gICAgcHJpdmF0ZSBwcmVzZXRzOiBOZXdGaWxlUHJlc2V0W10sXG4gICAgcHJpdmF0ZSBvbkNob29zZTogKHByZXNldDogTmV3RmlsZVByZXNldCkgPT4gdm9pZCxcbiAgKSB7XG4gICAgc3VwZXIoYXBwKTtcbiAgICB0aGlzLnNldFBsYWNlaG9sZGVyKCdQaWNrIGEgUXVhcnRvIGZpbGUgcHJlc2V04oCmJyk7XG4gIH1cblxuICBnZXRTdWdnZXN0aW9ucyhxdWVyeTogc3RyaW5nKTogTmV3RmlsZVByZXNldFtdIHtcbiAgICBjb25zdCBxID0gcXVlcnkudG9Mb3dlckNhc2UoKTtcbiAgICBpZiAoIXEpIHJldHVybiB0aGlzLnByZXNldHM7XG4gICAgcmV0dXJuIHRoaXMucHJlc2V0cy5maWx0ZXIoXG4gICAgICAocCkgPT5cbiAgICAgICAgcC5uYW1lLnRvTG93ZXJDYXNlKCkuaW5jbHVkZXMocSkgfHxcbiAgICAgICAgcC5kZXNjcmlwdGlvbi50b0xvd2VyQ2FzZSgpLmluY2x1ZGVzKHEpLFxuICAgICk7XG4gIH1cblxuICByZW5kZXJTdWdnZXN0aW9uKHByZXNldDogTmV3RmlsZVByZXNldCwgZWw6IEhUTUxFbGVtZW50KTogdm9pZCB7XG4gICAgY29uc3QgdGl0bGUgPSBwcmVzZXQuc291cmNlID09PSAnYnVpbHQtaW4nXG4gICAgICA/IGAke3ByZXNldC5uYW1lfSAgKGJ1aWx0LWluKWBcbiAgICAgIDogcHJlc2V0Lm5hbWU7XG4gICAgZWwuY3JlYXRlRWwoJ2RpdicsIHsgdGV4dDogdGl0bGUsIGNsczogJ3FtZC1wcmVzZXQtbmFtZScgfSk7XG4gICAgZWwuY3JlYXRlRWwoJ3NtYWxsJywgeyB0ZXh0OiBwcmVzZXQuZGVzY3JpcHRpb24sIGNsczogJ3FtZC1wcmVzZXQtZGVzYycgfSk7XG4gIH1cblxuICBvbkNob29zZVN1Z2dlc3Rpb24ocHJlc2V0OiBOZXdGaWxlUHJlc2V0KTogdm9pZCB7XG4gICAgdGhpcy5vbkNob29zZShwcmVzZXQpO1xuICB9XG59XG5cbmNsYXNzIEZpbGVuYW1lTW9kYWwgZXh0ZW5kcyBNb2RhbCB7XG4gIHByaXZhdGUgdmFsdWU6IHN0cmluZztcblxuICBjb25zdHJ1Y3RvcihcbiAgICBhcHA6IEFwcCxcbiAgICBwcml2YXRlIHByZXNldDogTmV3RmlsZVByZXNldCxcbiAgICBwcml2YXRlIGZvbGRlclBhdGg6IHN0cmluZyxcbiAgICBwcml2YXRlIG9uU3VibWl0OiAoZmlsZW5hbWU6IHN0cmluZykgPT4gdm9pZCxcbiAgKSB7XG4gICAgc3VwZXIoYXBwKTtcbiAgICB0aGlzLnZhbHVlID0gJ3VudGl0bGVkJztcbiAgfVxuXG4gIG9uT3BlbigpOiB2b2lkIHtcbiAgICBjb25zdCB7IGNvbnRlbnRFbCB9ID0gdGhpcztcbiAgICBjb250ZW50RWwuZW1wdHkoKTtcbiAgICBjb250ZW50RWwuY3JlYXRlRWwoJ2gzJywgeyB0ZXh0OiBgTmV3IFF1YXJ0byBmaWxlOiAke3RoaXMucHJlc2V0Lm5hbWV9YCB9KTtcbiAgICBjb250ZW50RWwuY3JlYXRlRWwoJ3AnLCB7XG4gICAgICB0ZXh0OiBgRm9sZGVyOiAke3RoaXMuZm9sZGVyUGF0aCB8fCAnKHZhdWx0IHJvb3QpJ31gLFxuICAgICAgY2xzOiAnc2V0dGluZy1pdGVtLWRlc2NyaXB0aW9uJyxcbiAgICB9KTtcblxuICAgIG5ldyBTZXR0aW5nKGNvbnRlbnRFbClcbiAgICAgIC5zZXROYW1lKCdGaWxlbmFtZScpXG4gICAgICAuc2V0RGVzYygnXCIucW1kXCIgaXMgYWRkZWQgYXV0b21hdGljYWxseSBpZiBvbWl0dGVkLicpXG4gICAgICAuYWRkVGV4dCgodGV4dCkgPT4ge1xuICAgICAgICB0ZXh0XG4gICAgICAgICAgLnNldFBsYWNlaG9sZGVyKCd1bnRpdGxlZCcpXG4gICAgICAgICAgLnNldFZhbHVlKHRoaXMudmFsdWUpXG4gICAgICAgICAgLm9uQ2hhbmdlKCh2KSA9PiAodGhpcy52YWx1ZSA9IHYpKTtcbiAgICAgICAgdGV4dC5pbnB1dEVsLmZvY3VzKCk7XG4gICAgICAgIHRleHQuaW5wdXRFbC5zZWxlY3QoKTtcbiAgICAgICAgdGV4dC5pbnB1dEVsLmFkZEV2ZW50TGlzdGVuZXIoJ2tleWRvd24nLCAoZSkgPT4ge1xuICAgICAgICAgIC8vIFNraXAgZHVyaW5nIElNRSBjb21wb3NpdGlvbiAoSmFwYW5lc2UvQ2hpbmVzZS9Lb3JlYW4gaW5wdXQpLFxuICAgICAgICAgIC8vIGFuZCBsZXQgbW9kaWZpZXIgY29tYm9zIHBhc3MgdGhyb3VnaCB0byB0aGUgT1MgLyBPYnNpZGlhbi5cbiAgICAgICAgICBpZiAoXG4gICAgICAgICAgICBlLmtleSA9PT0gJ0VudGVyJyAmJlxuICAgICAgICAgICAgIWUuaXNDb21wb3NpbmcgJiZcbiAgICAgICAgICAgICFlLnNoaWZ0S2V5ICYmXG4gICAgICAgICAgICAhZS5tZXRhS2V5ICYmXG4gICAgICAgICAgICAhZS5jdHJsS2V5ICYmXG4gICAgICAgICAgICAhZS5hbHRLZXlcbiAgICAgICAgICApIHtcbiAgICAgICAgICAgIGUucHJldmVudERlZmF1bHQoKTtcbiAgICAgICAgICAgIHRoaXMuc3VibWl0KCk7XG4gICAgICAgICAgfVxuICAgICAgICB9KTtcbiAgICAgIH0pO1xuXG4gICAgbmV3IFNldHRpbmcoY29udGVudEVsKS5hZGRCdXR0b24oKGJ0bikgPT5cbiAgICAgIGJ0blxuICAgICAgICAuc2V0QnV0dG9uVGV4dCgnQ3JlYXRlJylcbiAgICAgICAgLnNldEN0YSgpXG4gICAgICAgIC5vbkNsaWNrKCgpID0+IHRoaXMuc3VibWl0KCkpLFxuICAgICk7XG4gIH1cblxuICBwcml2YXRlIHN1Ym1pdCgpOiB2b2lkIHtcbiAgICBjb25zdCByYXcgPSB0aGlzLnZhbHVlLnRyaW0oKTtcbiAgICBpZiAoIXJhdykge1xuICAgICAgbmV3IE5vdGljZSgnRmlsZW5hbWUgaXMgcmVxdWlyZWQuJyk7XG4gICAgICByZXR1cm47XG4gICAgfVxuICAgIHRoaXMuY2xvc2UoKTtcbiAgICB0aGlzLm9uU3VibWl0KHJhdyk7XG4gIH1cblxuICBvbkNsb3NlKCk6IHZvaWQge1xuICAgIHRoaXMuY29udGVudEVsLmVtcHR5KCk7XG4gIH1cbn1cblxuLy8gU2FuaXRpemUgYSB1c2VyLXR5cGVkIGJhc2UgZmlsZW5hbWUgc28gdmF1bHQuY3JlYXRlIGNhbid0IGNob2tlIGFuZCB0aGVcbi8vIHJlc3VsdCBzdGlsbCB3b3JrcyBhZnRlciBzeW5jaW5nIHRvIFdpbmRvd3MgLyBtYWNPUyAvIExpbnV4OlxuLy8gICAtIHN0cmlwIGEgdHJhaWxpbmcgXCIucW1kXCIgKGNhc2UtaW5zZW5zaXRpdmUpOyBjYWxsZXIgcmUtYWRkcyBpdFxuLy8gICAtIHJlcGxhY2UgcGF0aCBzZXBhcmF0b3JzIHdpdGggXCItXCJcbi8vICAgLSBkcm9wIGNoYXJhY3RlcnMgV2luZG93cyBmb3JiaWRzIGluIGZpbGVuYW1lczogOiAqID8gXCIgPCA+IHxcbi8vICAgLSB0cmltIGxlYWRpbmcvdHJhaWxpbmcgd2hpdGVzcGFjZSBhbmQgZG90cyAoV2luZG93cyBzdHJpcHMgdGhlbSBhbnl3YXksXG4vLyAgICAgYW5kIGEgbGVhZGluZyBcIi5cIiB3b3VsZCBtYWtlIHRoZSBmaWxlIGhpZGRlbiBvbiBQT1NJWClcbmZ1bmN0aW9uIHNhbml0aXplQmFzZU5hbWUoaW5wdXQ6IHN0cmluZyk6IHN0cmluZyB7XG4gIGNvbnN0IHdpdGhvdXRFeHQgPSBpbnB1dC5yZXBsYWNlKC9cXC5xbWQkL2ksICcnKTtcbiAgY29uc3Qgbm9TbGFzaGVzID0gd2l0aG91dEV4dC5yZXBsYWNlKC9bXFxcXC9dL2csICctJyk7XG4gIGNvbnN0IG5vUmVzZXJ2ZWQgPSBub1NsYXNoZXMucmVwbGFjZSgvWzoqP1wiPD58XS9nLCAnJyk7XG4gIHJldHVybiBub1Jlc2VydmVkLnJlcGxhY2UoL15bXFxzLl0rfFtcXHMuXSskL2csICcnKTtcbn1cblxuLy8gUGljayBhIHRhcmdldCBwYXRoIG5leHQgdG8gdGhlIGFjdGl2ZSBmaWxlLCBmYWxsaW5nIGJhY2sgdG8gdmF1bHQgcm9vdC5cbi8vIEV4aXN0aW5nIGZpbGVzIGFyZSBub3Qgb3ZlcndyaXR0ZW4g4oCUIGFwcGVuZHMgXCItMVwiLCBcIi0yXCIsIOKApiB1bnRpbCBmcmVlLlxuYXN5bmMgZnVuY3Rpb24gYnVpbGRUYXJnZXRQYXRoKFxuICBhcHA6IEFwcCxcbiAgZm9sZGVyUGF0aDogc3RyaW5nLFxuICBiYXNlOiBzdHJpbmcsXG4pOiBQcm9taXNlPHN0cmluZz4ge1xuICBjb25zdCBzYWZlID0gc2FuaXRpemVCYXNlTmFtZShiYXNlKSB8fCAndW50aXRsZWQnO1xuICBsZXQgY2FuZGlkYXRlID0gbm9ybWFsaXplUGF0aChcbiAgICBmb2xkZXJQYXRoID8gYCR7Zm9sZGVyUGF0aH0vJHtzYWZlfS5xbWRgIDogYCR7c2FmZX0ucW1kYCxcbiAgKTtcbiAgbGV0IG4gPSAxO1xuICB3aGlsZSAoYXBwLnZhdWx0LmdldEFic3RyYWN0RmlsZUJ5UGF0aChjYW5kaWRhdGUpKSB7XG4gICAgY2FuZGlkYXRlID0gbm9ybWFsaXplUGF0aChcbiAgICAgIGZvbGRlclBhdGggPyBgJHtmb2xkZXJQYXRofS8ke3NhZmV9LSR7bn0ucW1kYCA6IGAke3NhZmV9LSR7bn0ucW1kYCxcbiAgICApO1xuICAgIG4gKz0gMTtcbiAgfVxuICByZXR1cm4gY2FuZGlkYXRlO1xufVxuXG5mdW5jdGlvbiBhY3RpdmVGb2xkZXJQYXRoKGFwcDogQXBwKTogc3RyaW5nIHtcbiAgY29uc3QgYWN0aXZlID0gYXBwLndvcmtzcGFjZS5nZXRBY3RpdmVGaWxlKCk7XG4gIGlmIChhY3RpdmU/LnBhcmVudCkgcmV0dXJuIGFjdGl2ZS5wYXJlbnQucGF0aCA9PT0gJy8nID8gJycgOiBhY3RpdmUucGFyZW50LnBhdGg7XG4gIHJldHVybiAnJztcbn1cblxuLy8gUmVhZCBldmVyeSB0b3AtbGV2ZWwgLnFtZCBpbnNpZGUgYGZvbGRlclBhdGhgIGFuZCB0dXJuIGVhY2ggaW50byBhIHVzZXJcbi8vIHByZXNldC4gU3ViZm9sZGVycyBhcmUgc2tpcHBlZCDigJQga2VlcHMgdGhlIHBpY2tlciBmbGF0IGFuZCBwcmVkaWN0YWJsZS5cbi8vIE1pc3NpbmcgZm9sZGVyIG9yIHJlYWQgZXJyb3JzIGRlZ3JhZGUgc2lsZW50bHkgdG8gXCJubyB1c2VyIHByZXNldHNcIjsgdGhlXG4vLyBidWlsdC1pbnMgc3RpbGwgc2hvdyB1cCBzbyB0aGUgY29tbWFuZCBuZXZlciBhcHBlYXJzIGJyb2tlbi5cbmFzeW5jIGZ1bmN0aW9uIGxvYWRVc2VyUHJlc2V0cyhcbiAgYXBwOiBBcHAsXG4gIGZvbGRlclBhdGg6IHN0cmluZyxcbik6IFByb21pc2U8TmV3RmlsZVByZXNldFtdPiB7XG4gIGNvbnN0IHRyaW1tZWQgPSBmb2xkZXJQYXRoLnRyaW0oKTtcbiAgaWYgKCF0cmltbWVkKSByZXR1cm4gW107XG4gIGNvbnN0IGZvbGRlciA9IGFwcC52YXVsdC5nZXRBYnN0cmFjdEZpbGVCeVBhdGgobm9ybWFsaXplUGF0aCh0cmltbWVkKSk7XG4gIGlmICghKGZvbGRlciBpbnN0YW5jZW9mIFRGb2xkZXIpKSByZXR1cm4gW107XG4gIGNvbnN0IG91dDogTmV3RmlsZVByZXNldFtdID0gW107XG4gIGZvciAoY29uc3QgY2hpbGQgb2YgZm9sZGVyLmNoaWxkcmVuKSB7XG4gICAgaWYgKCEoY2hpbGQgaW5zdGFuY2VvZiBURmlsZSkpIGNvbnRpbnVlO1xuICAgIGlmIChjaGlsZC5leHRlbnNpb24udG9Mb3dlckNhc2UoKSAhPT0gJ3FtZCcpIGNvbnRpbnVlO1xuICAgIHRyeSB7XG4gICAgICBjb25zdCBib2R5ID0gYXdhaXQgYXBwLnZhdWx0LmNhY2hlZFJlYWQoY2hpbGQpO1xuICAgICAgb3V0LnB1c2goe1xuICAgICAgICBpZDogYHVzZXI6JHtjaGlsZC5wYXRofWAsXG4gICAgICAgIG5hbWU6IGNoaWxkLmJhc2VuYW1lLFxuICAgICAgICBkZXNjcmlwdGlvbjogYEZyb20gJHtjaGlsZC5wYXRofWAsXG4gICAgICAgIHNvdXJjZTogJ3VzZXInLFxuICAgICAgICBib2R5LFxuICAgICAgfSk7XG4gICAgfSBjYXRjaCAoZXJyKSB7XG4gICAgICBjb25zb2xlLndhcm4oYFtxbWQtYXMtbWRdIFNraXBwZWQgdGVtcGxhdGUgJHtjaGlsZC5wYXRofTpgLCBlcnIpO1xuICAgIH1cbiAgfVxuICBvdXQuc29ydCgoYSwgYikgPT4gYS5uYW1lLmxvY2FsZUNvbXBhcmUoYi5uYW1lKSk7XG4gIHJldHVybiBvdXQ7XG59XG5cbmV4cG9ydCBhc3luYyBmdW5jdGlvbiBuZXdRbWRGcm9tUHJlc2V0KFxuICBhcHA6IEFwcCxcbiAgdGVtcGxhdGVzRm9sZGVyOiBzdHJpbmcsXG4pOiBQcm9taXNlPHZvaWQ+IHtcbiAgY29uc3QgdXNlclByZXNldHMgPSBhd2FpdCBsb2FkVXNlclByZXNldHMoYXBwLCB0ZW1wbGF0ZXNGb2xkZXIpO1xuICAvLyBVc2VyIHByZXNldHMgZmlyc3Qg4oCUIHRoZXkncmUgdGhlIG9uZXMgdGhlIHVzZXIgYWN0aXZlbHkgY3VyYXRlZDsgYnVpbHQtaW5zXG4gIC8vIGZhbGwgdG8gdGhlIGJvdHRvbSBhcyBmYWxsYmFjayBvcHRpb25zLlxuICBjb25zdCBwcmVzZXRzID0gWy4uLnVzZXJQcmVzZXRzLCAuLi5ERUZBVUxUX1BSRVNFVFNdO1xuXG4gIGNvbnN0IG1vZGFsID0gbmV3IFByZXNldFN1Z2dlc3RNb2RhbChhcHAsIHByZXNldHMsIChwcmVzZXQpID0+IHtcbiAgICBjb25zdCBmb2xkZXIgPSBhY3RpdmVGb2xkZXJQYXRoKGFwcCk7XG4gICAgbmV3IEZpbGVuYW1lTW9kYWwoYXBwLCBwcmVzZXQsIGZvbGRlciwgYXN5bmMgKGZpbGVuYW1lKSA9PiB7XG4gICAgICB0cnkge1xuICAgICAgICBjb25zdCB0YXJnZXQgPSBhd2FpdCBidWlsZFRhcmdldFBhdGgoYXBwLCBmb2xkZXIsIGZpbGVuYW1lKTtcbiAgICAgICAgY29uc3QgcGFyZW50ID0gdGFyZ2V0LmluY2x1ZGVzKCcvJylcbiAgICAgICAgICA/IHRhcmdldC5zbGljZSgwLCB0YXJnZXQubGFzdEluZGV4T2YoJy8nKSlcbiAgICAgICAgICA6ICcnO1xuICAgICAgICBpZiAocGFyZW50KSB7XG4gICAgICAgICAgY29uc3QgZXhpc3RpbmcgPSBhcHAudmF1bHQuZ2V0QWJzdHJhY3RGaWxlQnlQYXRoKHBhcmVudCk7XG4gICAgICAgICAgaWYgKGV4aXN0aW5nIGluc3RhbmNlb2YgVEZpbGUpIHtcbiAgICAgICAgICAgIC8vIEEgZmlsZSBhbHJlYWR5IG9jY3VwaWVzIHRoZSB3b3VsZC1iZSBwYXJlbnQgZm9sZGVyIHBhdGgg4oCUXG4gICAgICAgICAgICAvLyBjcmVhdGVGb2xkZXIgd291bGQgc3VyZmFjZSBhIGNvbmZ1c2luZyBsb3ctbGV2ZWwgZXJyb3I7IGJhaWxcbiAgICAgICAgICAgIC8vIHdpdGggYSBwcmVjaXNlIG1lc3NhZ2UgaW5zdGVhZC5cbiAgICAgICAgICAgIG5ldyBOb3RpY2UoXG4gICAgICAgICAgICAgIGBDYW5ub3QgY3JlYXRlIGZvbGRlciBcIiR7cGFyZW50fVwiOiBhIGZpbGUgd2l0aCB0aGF0IG5hbWUgYWxyZWFkeSBleGlzdHMuYCxcbiAgICAgICAgICAgICk7XG4gICAgICAgICAgICByZXR1cm47XG4gICAgICAgICAgfVxuICAgICAgICAgIGlmICghKGV4aXN0aW5nIGluc3RhbmNlb2YgVEZvbGRlcikpIHtcbiAgICAgICAgICAgIGF3YWl0IGFwcC52YXVsdC5jcmVhdGVGb2xkZXIocGFyZW50KTtcbiAgICAgICAgICB9XG4gICAgICAgIH1cbiAgICAgICAgY29uc3QgZmlsZSA9IGF3YWl0IGFwcC52YXVsdC5jcmVhdGUodGFyZ2V0LCBwcmVzZXQuYm9keSk7XG4gICAgICAgIGlmIChmaWxlIGluc3RhbmNlb2YgVEZpbGUpIHtcbiAgICAgICAgICBhd2FpdCBhcHAud29ya3NwYWNlLmdldExlYWYoZmFsc2UpLm9wZW5GaWxlKGZpbGUpO1xuICAgICAgICB9XG4gICAgICAgIG5ldyBOb3RpY2UoYENyZWF0ZWQgJHt0YXJnZXR9YCk7XG4gICAgICB9IGNhdGNoIChlcnIpIHtcbiAgICAgICAgY29uc3QgbXNnID0gZXJyIGluc3RhbmNlb2YgRXJyb3IgPyBlcnIubWVzc2FnZSA6IFN0cmluZyhlcnIpO1xuICAgICAgICBuZXcgTm90aWNlKGBGYWlsZWQgdG8gY3JlYXRlIGZpbGU6ICR7bXNnfWApO1xuICAgICAgfVxuICAgIH0pLm9wZW4oKTtcbiAgfSk7XG4gIG1vZGFsLm9wZW4oKTtcbn0iLCJpbXBvcnQge1xuICBQbHVnaW4sXG4gIE5vdGljZSxcbiAgVEZpbGUsXG4gIEZpbGVWaWV3LFxuICBNYXJrZG93blZpZXcsXG4gIFBsdWdpblNldHRpbmdUYWIsXG4gIEFwcCxcbiAgU2V0dGluZyxcbiAgRmlsZVN5c3RlbUFkYXB0ZXIsXG4gIFdvcmtzcGFjZUxlYWYsXG4gIGRlYm91bmNlLFxuICBub3JtYWxpemVQYXRoLFxufSBmcm9tICdvYnNpZGlhbic7XG5pbXBvcnQgeyBzcGF3biwgQ2hpbGRQcm9jZXNzIH0gZnJvbSAnY2hpbGRfcHJvY2Vzcyc7XG5pbXBvcnQgKiBhcyBwYXRoIGZyb20gJ3BhdGgnO1xuaW1wb3J0IHsgc2hlbGwgfSBmcm9tICdlbGVjdHJvbic7XG5pbXBvcnQgeyBRTURfT1VUTElORV9WSUVXLCBRbWRPdXRsaW5lVmlldyB9IGZyb20gJy4vb3V0bGluZSc7XG5pbXBvcnQge1xuICBRTURfWUFNTF9WSUVXLFxuICBRTURfTFVBX1ZJRVcsXG4gIFFtZFlhbWxGaWxlVmlldyxcbiAgUW1kTHVhRmlsZVZpZXcsXG59IGZyb20gJy4vY29kZS12aWV3JztcbmltcG9ydCB7IG5ld1FtZEZyb21QcmVzZXQgfSBmcm9tICcuL25ldy1maWxlJztcblxuLy8gLS0tIFF1YXJ0byBvdXRwdXQgcGx1bWJpbmcgLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS0tLS1cbi8vXG4vLyBOb2RlJ3Mgc3Bhd24tc3RyZWFtIGNodW5rcyBkb24ndCBhbGlnbiB3aXRoIGxpbmUgYm91bmRhcmllcyDigJQgYSBzaW5nbGVcbi8vIGRhdGEgZXZlbnQgY2FuIGNvbnRhaW4gYSBwYXJ0aWFsIGxpbmUsIGFuZCBhIGxvZ2ljYWwgbGluZSBjYW4gYmUgc3BsaXRcbi8vIGFjcm9zcyB0d28gZXZlbnRzLiBCdWlsZCBhIHBlci1zdHJlYW0gcHJvY2Vzc29yIHRoYXQgYnVmZmVycyB0aGVcbi8vIHRyYWlsaW5nIHBhcnRpYWwgbGluZSBhbmQgb25seSBlbWl0cyB3aG9sZSBsaW5lcy4gQ2FsbCAuZmx1c2goKSBvbiB0aGVcbi8vIGNsb3NlIGhhbmRsZXIgdG8gcmVsZWFzZSBhbnkgZmluYWwgcGFydGlhbCBsaW5lLlxuLy9cbi8vIGxvZ1F1YXJ0b0xpbmUgcm91dGVzIGEgc2luZ2xlIGxpbmUgdG8gY29uc29sZSBieSBzZXZlcml0eSBwcmVmaXg6XG4vLyBcIkVSUk9SOlwiIC0+IGNvbnNvbGUuZXJyb3IsIFwiV0FSTklORzpcIi9cIldBUk46XCIgLT4gY29uc29sZS53YXJuLFxuLy8gZXZlcnl0aGluZyBlbHNlIC0+IGNvbnNvbGUubG9nLiBDZW50cmFsaXNlZCBzbyBib3RoIHRoZSBwcmV2aWV3IGFuZFxuLy8gcmVuZGVyIHBhdGhzIHN0YXkgaW4gc3luYyBhbmQgbmV3IHByZWZpeGVzIG9ubHkgbmVlZCBoYW5kbGluZyBoZXJlLlxuXG5mdW5jdGlvbiBsb2dRdWFydG9MaW5lKHByZWZpeDogc3RyaW5nLCBsaW5lOiBzdHJpbmcpOiB2b2lkIHtcbiAgaWYgKC9eRVJST1I6Ly50ZXN0KGxpbmUpKSB7XG4gICAgY29uc29sZS5lcnJvcihgJHtwcmVmaXh9OiAke2xpbmV9YCk7XG4gIH0gZWxzZSBpZiAoL15XQVJOKElORyk/Oi8udGVzdChsaW5lKSkge1xuICAgIGNvbnNvbGUud2FybihgJHtwcmVmaXh9OiAke2xpbmV9YCk7XG4gIH0gZWxzZSB7XG4gICAgY29uc29sZS5sb2coYCR7cHJlZml4fTogJHtsaW5lfWApO1xuICB9XG59XG5cbmludGVyZmFjZSBMaW5lUHJvY2Vzc29yIHtcbiAgKGNodW5rOiBzdHJpbmcpOiB2b2lkO1xuICBmbHVzaCgpOiB2b2lkO1xufVxuXG5mdW5jdGlvbiBtYWtlTGluZVByb2Nlc3NvcihoYW5kbGU6IChsaW5lOiBzdHJpbmcpID0+IHZvaWQpOiBMaW5lUHJvY2Vzc29yIHtcbiAgbGV0IGJ1ZmZlciA9ICcnO1xuICBjb25zdCBwcm9jID0gKChjaHVuazogc3RyaW5nKSA9PiB7XG4gICAgYnVmZmVyICs9IGNodW5rO1xuICAgIGNvbnN0IGxpbmVzID0gYnVmZmVyLnNwbGl0KC9cXHI/XFxuLyk7XG4gICAgLy8gTGFzdCBlbGVtZW50IGlzIHRoZSB0cmFpbGluZyBmcmFnbWVudCBhZnRlciB0aGUgZmluYWwgbmV3bGluZVxuICAgIC8vIChvciB0aGUgd2hvbGUgY2h1bmsgaWYgdGhlcmUgd2FzIG5vIG5ld2xpbmUgYXQgYWxsKS4gS2VlcCBpdFxuICAgIC8vIGZvciB0aGUgbmV4dCBjaHVuay5cbiAgICBidWZmZXIgPSBsaW5lcy5wb3AoKSA/PyAnJztcbiAgICBmb3IgKGNvbnN0IGxpbmUgb2YgbGluZXMpIHtcbiAgICAgIGlmIChsaW5lKSBoYW5kbGUobGluZSk7XG4gICAgfVxuICB9KSBhcyBMaW5lUHJvY2Vzc29yO1xuICBwcm9jLmZsdXNoID0gKCkgPT4ge1xuICAgIGlmIChidWZmZXIpIHtcbiAgICAgIGhhbmRsZShidWZmZXIpO1xuICAgICAgYnVmZmVyID0gJyc7XG4gICAgfVxuICB9O1xuICByZXR1cm4gcHJvYztcbn1cblxuY29uc3QgQU5TSV9FU0NBUEVfUkUgPSBuZXcgUmVnRXhwKFN0cmluZy5mcm9tQ2hhckNvZGUoMjcpICsgJ1xcXFxbWzAtP10qWyAtL10qW0Atfl0nLCAnZycpO1xuZnVuY3Rpb24gc3RyaXBBbnNpQ29kZXModGV4dDogc3RyaW5nKTogc3RyaW5nIHtcbiAgcmV0dXJuIHRleHQucmVwbGFjZShBTlNJX0VTQ0FQRV9SRSwgJycpO1xufVxuXG5mdW5jdGlvbiBwcmV2aWV3VXJsRnJvbUxpbmUobGluZTogc3RyaW5nKTogc3RyaW5nIHwgbnVsbCB7XG4gIGNvbnN0IG1hdGNoID0gc3RyaXBBbnNpQ29kZXMobGluZSkubWF0Y2goL0Jyb3dzZSBhdFxccysoaHR0cHM/OlxcL1xcL1xcUyspLyk7XG4gIHJldHVybiBtYXRjaD8uWzFdID8/IG51bGw7XG59XG5cbmludGVyZmFjZSBRbWRQbHVnaW5TZXR0aW5ncyB7XG4gIHF1YXJ0b1BhdGg6IHN0cmluZztcbiAgZW5hYmxlUW1kTGlua2luZzogYm9vbGVhbjtcbiAgcXVhcnRvVHlwc3Q6IHN0cmluZztcbiAgb3BlblBkZkluT2JzaWRpYW46IGJvb2xlYW47XG4gIHByZXZpZXdJbk9ic2lkaWFuOiBib29sZWFuO1xuICBwcmV2aWV3TWFya2Rvd25GaWxlczogYm9vbGVhbjtcbiAgb3V0bGluZU1hcmtkb3duRmlsZXM6IGJvb2xlYW47XG4gIHNob3dZYW1sRmlsZXM6IGJvb2xlYW47XG4gIHNob3dMdWFGaWxlczogYm9vbGVhbjtcbiAgc2hvd091dGxpbmU6IGJvb2xlYW47XG4gIHRlbXBsYXRlc0ZvbGRlcjogc3RyaW5nO1xufVxuXG50eXBlIFByZXZpZXdNb2RlID0gJ29ic2lkaWFuJyB8ICdleHRlcm5hbCc7XG5cbmludGVyZmFjZSBBY3RpdmVQcmV2aWV3IHtcbiAgcHJvY2VzczogQ2hpbGRQcm9jZXNzO1xuICBtb2RlOiBQcmV2aWV3TW9kZTtcbn1cblxuY29uc3QgREVGQVVMVF9TRVRUSU5HUzogUW1kUGx1Z2luU2V0dGluZ3MgPSB7XG4gIHF1YXJ0b1BhdGg6ICdxdWFydG8nLFxuICBlbmFibGVRbWRMaW5raW5nOiB0cnVlLFxuICBxdWFydG9UeXBzdDogJycsXG4gIG9wZW5QZGZJbk9ic2lkaWFuOiBmYWxzZSxcbiAgcHJldmlld0luT2JzaWRpYW46IHRydWUsXG4gIHByZXZpZXdNYXJrZG93bkZpbGVzOiBmYWxzZSxcbiAgb3V0bGluZU1hcmtkb3duRmlsZXM6IGZhbHNlLFxuICBzaG93WWFtbEZpbGVzOiBmYWxzZSxcbiAgc2hvd0x1YUZpbGVzOiBmYWxzZSxcbiAgc2hvd091dGxpbmU6IGZhbHNlLFxuICB0ZW1wbGF0ZXNGb2xkZXI6ICcnLFxufTtcblxuZXhwb3J0IGRlZmF1bHQgY2xhc3MgUW1kQXNNZFBsdWdpbiBleHRlbmRzIFBsdWdpbiB7XG4gIHNldHRpbmdzOiBRbWRQbHVnaW5TZXR0aW5ncztcbiAgYWN0aXZlUHJldmlld1Byb2Nlc3NlczogTWFwPHN0cmluZywgQWN0aXZlUHJldmlldz4gPSBuZXcgTWFwKCk7XG4gIC8vIFRoZSAucW1kIGZpbGUgdGhlIG91dGxpbmUgc2hvdWxkIGRlc2NyaWJlLiBUcmFja2VkIHNlcGFyYXRlbHkgZnJvbSB0aGVcbiAgLy8gYWN0aXZlIGxlYWY6IGNsaWNraW5nIGluc2lkZSB0aGUgb3V0bGluZSBzaWRlYmFyIG1ha2VzICppdCogdGhlIGFjdGl2ZVxuICAvLyBsZWFmLCBzbyB0aGUgb3V0bGluZSBtdXN0IHJlbWVtYmVyIHRoZSBsYXN0IHJlYWwgLnFtZCByYXRoZXIgdGhhbiBhc2tcbiAgLy8gXCJ3aGF0IGlzIGFjdGl2ZSBub3c/XCIgZWFjaCByZW5kZXIuXG4gIGxhc3RBY3RpdmVRdWFydG9GaWxlOiBURmlsZSB8IG51bGwgPSBudWxsO1xuXG4gIGFzeW5jIG9ubG9hZCgpIHtcbiAgICB0cnkge1xuICAgICAgYXdhaXQgdGhpcy5sb2FkU2V0dGluZ3MoKTtcblxuICAgICAgdGhpcy5yZWdpc3RlclZpZXcoUU1EX09VVExJTkVfVklFVywgKGxlYWYpID0+IG5ldyBRbWRPdXRsaW5lVmlldyhsZWFmLCB0aGlzKSk7XG4gICAgICB0aGlzLnJlZ2lzdGVyVmlldyhRTURfWUFNTF9WSUVXLCAobGVhZikgPT4gbmV3IFFtZFlhbWxGaWxlVmlldyhsZWFmKSk7XG4gICAgICB0aGlzLnJlZ2lzdGVyVmlldyhRTURfTFVBX1ZJRVcsIChsZWFmKSA9PiBuZXcgUW1kTHVhRmlsZVZpZXcobGVhZikpO1xuXG4gICAgICBpZiAodGhpcy5zZXR0aW5ncy5lbmFibGVRbWRMaW5raW5nKSB7XG4gICAgICAgIHRoaXMucmVnaXN0ZXJRbWRFeHRlbnNpb24oKTtcbiAgICAgIH1cbiAgICAgIGlmICh0aGlzLnNldHRpbmdzLnNob3dZYW1sRmlsZXMpIHtcbiAgICAgICAgdGhpcy5yZWdpc3RlcllhbWxFeHRlbnNpb25zKCk7XG4gICAgICB9XG4gICAgICBpZiAodGhpcy5zZXR0aW5ncy5zaG93THVhRmlsZXMpIHtcbiAgICAgICAgdGhpcy5yZWdpc3Rlckx1YUV4dGVuc2lvbnMoKTtcbiAgICAgIH1cblxuICAgICAgdGhpcy5hZGRTZXR0aW5nVGFiKG5ldyBRbWRTZXR0aW5nVGFiKHRoaXMuYXBwLCB0aGlzKSk7XG5cbiAgICAgIHRoaXMuYWRkUmliYm9uSWNvbignZXllJywgJ1RvZ2dsZSBRdWFydG8gcHJldmlldycsIGFzeW5jICgpID0+IHtcbiAgICAgICAgY29uc3QgZmlsZSA9IHRoaXMuZ2V0QWN0aXZlUXVhcnRvQ29tbWFuZEZpbGUoKTtcbiAgICAgICAgaWYgKGZpbGUpIGF3YWl0IHRoaXMudG9nZ2xlUHJldmlldyhmaWxlKTtcbiAgICAgIH0pO1xuXG4gICAgICB0aGlzLmFkZENvbW1hbmQoe1xuICAgICAgICBpZDogJ3RvZ2dsZS1xdWFydG8tcHJldmlldycsXG4gICAgICAgIG5hbWU6ICdUb2dnbGUgUXVhcnRvIHByZXZpZXcnLFxuICAgICAgICBjYWxsYmFjazogYXN5bmMgKCkgPT4ge1xuICAgICAgICAgIGNvbnN0IGZpbGUgPSB0aGlzLmdldEFjdGl2ZVF1YXJ0b0NvbW1hbmRGaWxlKCk7XG4gICAgICAgICAgaWYgKGZpbGUpIGF3YWl0IHRoaXMudG9nZ2xlUHJldmlldyhmaWxlKTtcbiAgICAgICAgfSxcbiAgICAgIH0pO1xuXG4gICAgICB0aGlzLmFkZENvbW1hbmQoe1xuICAgICAgICBpZDogJ3RvZ2dsZS1xdWFydG8tcHJldmlldy1pbi1vYnNpZGlhbicsXG4gICAgICAgIG5hbWU6ICdUb2dnbGUgUXVhcnRvIHByZXZpZXcgaW4gT2JzaWRpYW4nLFxuICAgICAgICBjYWxsYmFjazogYXN5bmMgKCkgPT4ge1xuICAgICAgICAgIGNvbnN0IGZpbGUgPSB0aGlzLmdldEFjdGl2ZVF1YXJ0b0NvbW1hbmRGaWxlKCk7XG4gICAgICAgICAgaWYgKGZpbGUpIGF3YWl0IHRoaXMudG9nZ2xlUHJldmlldyhmaWxlLCAnb2JzaWRpYW4nKTtcbiAgICAgICAgfSxcbiAgICAgIH0pO1xuXG4gICAgICB0aGlzLmFkZENvbW1hbmQoe1xuICAgICAgICBpZDogJ3RvZ2dsZS1xdWFydG8tcHJldmlldy1leHRlcm5hbCcsXG4gICAgICAgIG5hbWU6ICdUb2dnbGUgUXVhcnRvIHByZXZpZXcgaW4gZXh0ZXJuYWwgYnJvd3NlcicsXG4gICAgICAgIGNhbGxiYWNrOiBhc3luYyAoKSA9PiB7XG4gICAgICAgICAgY29uc3QgZmlsZSA9IHRoaXMuZ2V0QWN0aXZlUXVhcnRvQ29tbWFuZEZpbGUoKTtcbiAgICAgICAgICBpZiAoZmlsZSkgYXdhaXQgdGhpcy50b2dnbGVQcmV2aWV3KGZpbGUsICdleHRlcm5hbCcpO1xuICAgICAgICB9LFxuICAgICAgfSk7XG5cbiAgICAgIHRoaXMuYWRkUmliYm9uSWNvbignZmlsZS1vdXRwdXQnLCAnUmVuZGVyIFF1YXJ0byB0byBQREYnLCBhc3luYyAoKSA9PiB7XG4gICAgICAgIGNvbnN0IGZpbGUgPSB0aGlzLmdldEFjdGl2ZVF1YXJ0b0NvbW1hbmRGaWxlKCk7XG4gICAgICAgIGlmIChmaWxlKSBhd2FpdCB0aGlzLnJlbmRlclBkZihmaWxlKTtcbiAgICAgIH0pO1xuXG4gICAgICB0aGlzLnJlZ2lzdGVyUmVuZGVyQ29tbWFuZCgncmVuZGVyLXF1YXJ0by1wZGYnLCAnUmVuZGVyIFF1YXJ0byAodXNlIGZvcm1hdCBkZWZpbmVkIGluIFlBTUwpJyk7XG4gICAgICB0aGlzLnJlZ2lzdGVyUmVuZGVyQ29tbWFuZCgncmVuZGVyLXF1YXJ0by1wZGYtdHlwc3QnLCAnUmVuZGVyIFF1YXJ0byB0byBQREYgKFR5cHN0IGVuZ2luZSknLCAndHlwc3QnKTtcbiAgICAgIHRoaXMucmVnaXN0ZXJSZW5kZXJDb21tYW5kKCdyZW5kZXItcXVhcnRvLXBkZi1sYXRleCcsICdSZW5kZXIgUXVhcnRvIHRvIFBERiAoTGFUZVggZW5naW5lKScsICdwZGYnKTtcblxuICAgICAgdGhpcy5hZGRDb21tYW5kKHtcbiAgICAgICAgaWQ6ICdvcGVuLXF1YXJ0by1vdXRsaW5lJyxcbiAgICAgICAgbmFtZTogJ09wZW4gUXVhcnRvIG91dGxpbmUnLFxuICAgICAgICBjYWxsYmFjazogKCkgPT4gdGhpcy5hY3RpdmF0ZU91dGxpbmVWaWV3KCksXG4gICAgICB9KTtcblxuICAgICAgdGhpcy5hZGRDb21tYW5kKHtcbiAgICAgICAgaWQ6ICduZXctcXVhcnRvLWZpbGUtZnJvbS1wcmVzZXQnLFxuICAgICAgICBuYW1lOiAnTmV3IFF1YXJ0byBmaWxlIGZyb20gcHJlc2V0JyxcbiAgICAgICAgaWNvbjogJ2ZpbGUtcGx1cy0yJyxcbiAgICAgICAgY2FsbGJhY2s6ICgpID0+IG5ld1FtZEZyb21QcmVzZXQodGhpcy5hcHAsIHRoaXMuc2V0dGluZ3MudGVtcGxhdGVzRm9sZGVyKSxcbiAgICAgIH0pO1xuXG4gICAgICAvLyBLZWVwIGFueSBvcGVuIG91dGxpbmUgdmlldyBpbiBzeW5jIHdpdGggdGhlIGZvY3VzZWQgZmlsZSBhbmQgaXRzXG4gICAgICAvLyBlZGl0cy4gRGVib3VuY2VkIHNvIGEgYnVyc3Qgb2Yga2V5c3Ryb2tlcyByZS1wYXJzZXMgb25jZSBpdCBzZXR0bGVzLlxuICAgICAgY29uc3QgcmVmcmVzaCA9IGRlYm91bmNlKCgpID0+IHtcbiAgICAgICAgdGhpcy50cmFja0FjdGl2ZVF1YXJ0b0ZpbGUoKTtcbiAgICAgICAgdGhpcy5yZWZyZXNoT3V0bGluZVZpZXdzKCk7XG4gICAgICB9LCAyNTAsIHRydWUpO1xuICAgICAgdGhpcy5yZWdpc3RlckV2ZW50KHRoaXMuYXBwLndvcmtzcGFjZS5vbignYWN0aXZlLWxlYWYtY2hhbmdlJywgcmVmcmVzaCkpO1xuICAgICAgdGhpcy5yZWdpc3RlckV2ZW50KHRoaXMuYXBwLndvcmtzcGFjZS5vbignZWRpdG9yLWNoYW5nZScsIHJlZnJlc2gpKTtcblxuICAgICAgLy8gT3B0LWluOiBvbmx5IGF1dG8tb3BlbiB0aGUgb3V0bGluZSB3aGVuIHRoZSB1c2VyIGVuYWJsZWQgaXQuIFRoZVxuICAgICAgLy8gY29tbWFuZCBhYm92ZSBhbHdheXMgd29ya3MgcmVnYXJkbGVzcyBvZiB0aGlzIHNldHRpbmcuXG4gICAgICBpZiAodGhpcy5zZXR0aW5ncy5zaG93T3V0bGluZSkge1xuICAgICAgICB0aGlzLmFwcC53b3Jrc3BhY2Uub25MYXlvdXRSZWFkeSgoKSA9PiB0aGlzLmFjdGl2YXRlT3V0bGluZVZpZXcoKSk7XG4gICAgICB9XG4gICAgfSBjYXRjaCAoZXJyb3IpIHtcbiAgICAgIGNvbnNvbGUuZXJyb3IoJ0Vycm9yIGxvYWRpbmcgcGx1Z2luOicsIGVycm9yKTtcbiAgICAgIG5ldyBOb3RpY2UoJ0ZhaWxlZCB0byBsb2FkIHRoZSBRTUQgYXMgbWQgcGx1Z2luLiBDaGVjayB0aGUgZGV2ZWxvcGVyIGNvbnNvbGUgZm9yIGRldGFpbHMuJyk7XG4gICAgfVxuICB9XG5cbiAgb251bmxvYWQoKSB7XG4gICAgdGhpcy5zdG9wQWxsUHJldmlld3MoKTtcbiAgfVxuXG4gIGFzeW5jIGxvYWRTZXR0aW5ncygpIHtcbiAgICBjb25zdCBsb2FkZWQgPSAoYXdhaXQgdGhpcy5sb2FkRGF0YSgpKSBhcyBQYXJ0aWFsPFFtZFBsdWdpblNldHRpbmdzPiB8IG51bGw7XG4gICAgdGhpcy5zZXR0aW5ncyA9IE9iamVjdC5hc3NpZ24oe30sIERFRkFVTFRfU0VUVElOR1MsIGxvYWRlZCk7XG4gIH1cblxuICBhc3luYyBzYXZlU2V0dGluZ3MoKSB7XG4gICAgYXdhaXQgdGhpcy5zYXZlRGF0YSh0aGlzLnNldHRpbmdzKTtcbiAgfVxuXG4gIGlzUXVhcnRvRmlsZShmaWxlOiBURmlsZSk6IGJvb2xlYW4ge1xuICAgIHJldHVybiBmaWxlLmV4dGVuc2lvbiA9PT0gJ3FtZCc7XG4gIH1cblxuICBpc01hcmtkb3duRmlsZShmaWxlOiBURmlsZSk6IGJvb2xlYW4ge1xuICAgIHJldHVybiBmaWxlLmV4dGVuc2lvbiA9PT0gJ21kJztcbiAgfVxuXG4gIGhhc1F1YXJ0b1Byb2plY3RDb25maWdJblBhdGgoZmlsZTogVEZpbGUpOiBib29sZWFuIHtcbiAgICBsZXQgZGlyID0gZmlsZS5wYXJlbnQ/LnBhdGggPz8gJyc7XG4gICAgd2hpbGUgKHRydWUpIHtcbiAgICAgIGNvbnN0IGNvbmZpZ1BhdGggPSBub3JtYWxpemVQYXRoKGRpciA/IGAke2Rpcn0vX3F1YXJ0by55bWxgIDogJ19xdWFydG8ueW1sJyk7XG4gICAgICBpZiAodGhpcy5hcHAudmF1bHQuZ2V0QWJzdHJhY3RGaWxlQnlQYXRoKGNvbmZpZ1BhdGgpIGluc3RhbmNlb2YgVEZpbGUpIHtcbiAgICAgICAgcmV0dXJuIHRydWU7XG4gICAgICB9XG4gICAgICBpZiAoIWRpcikgcmV0dXJuIGZhbHNlO1xuICAgICAgY29uc3Qgc2xhc2ggPSBkaXIubGFzdEluZGV4T2YoJy8nKTtcbiAgICAgIGRpciA9IHNsYXNoID09PSAtMSA/ICcnIDogZGlyLnNsaWNlKDAsIHNsYXNoKTtcbiAgICB9XG4gIH1cblxuICBnZXRBY3RpdmVRdWFydG9Db21tYW5kRmlsZSgpOiBURmlsZSB8IG51bGwge1xuICAgIGNvbnN0IGFjdGl2ZVZpZXcgPSB0aGlzLmFwcC53b3Jrc3BhY2UuZ2V0QWN0aXZlVmlld09mVHlwZShNYXJrZG93blZpZXcpO1xuICAgIGNvbnN0IGZpbGUgPSBhY3RpdmVWaWV3Py5maWxlO1xuICAgIGlmICghZmlsZSkge1xuICAgICAgbmV3IE5vdGljZShcbiAgICAgICAgdGhpcy5zZXR0aW5ncy5wcmV2aWV3TWFya2Rvd25GaWxlc1xuICAgICAgICAgID8gJ1F1YXJ0byBjb21tYW5kcyByZXF1aXJlIGFuIGFjdGl2ZSAucW1kIG9yIC5tZCBmaWxlLidcbiAgICAgICAgICA6ICdRdWFydG8gY29tbWFuZHMgcmVxdWlyZSBhbiBhY3RpdmUgLnFtZCBmaWxlLidcbiAgICAgICk7XG4gICAgICByZXR1cm4gbnVsbDtcbiAgICB9XG5cbiAgICBpZiAodGhpcy5pc1F1YXJ0b0ZpbGUoZmlsZSkpIHtcbiAgICAgIHJldHVybiBmaWxlO1xuICAgIH1cblxuICAgIGlmICh0aGlzLmlzTWFya2Rvd25GaWxlKGZpbGUpKSB7XG4gICAgICBpZiAoIXRoaXMuc2V0dGluZ3MucHJldmlld01hcmtkb3duRmlsZXMpIHtcbiAgICAgICAgbmV3IE5vdGljZSgnTWFya2Rvd24gc3VwcG9ydCBpcyBvZmYuIEVuYWJsZSBpdCBpbiBzZXR0aW5ncyB0byBwcmV2aWV3IG9yIHJlbmRlciAubWQgZmlsZXMgd2l0aCBRdWFydG8uJyk7XG4gICAgICAgIHJldHVybiBudWxsO1xuICAgICAgfVxuICAgICAgaWYgKCF0aGlzLmhhc1F1YXJ0b1Byb2plY3RDb25maWdJblBhdGgoZmlsZSkpIHtcbiAgICAgICAgbmV3IE5vdGljZSgnTWFya2Rvd24gUXVhcnRvIGNvbW1hbmRzIHJlcXVpcmUgX3F1YXJ0by55bWwgaW4gdGhpcyBmaWxlIGZvbGRlciBvciBhbiBhbmNlc3RvciB1cCB0byB0aGUgdmF1bHQgcm9vdC4nKTtcbiAgICAgICAgcmV0dXJuIG51bGw7XG4gICAgICB9XG4gICAgICByZXR1cm4gZmlsZTtcbiAgICB9XG5cbiAgICBpZiAodGhpcy5zZXR0aW5ncy5wcmV2aWV3TWFya2Rvd25GaWxlcykge1xuICAgICAgbmV3IE5vdGljZSgnUXVhcnRvIGNvbW1hbmRzIHJlcXVpcmUgYW4gYWN0aXZlIC5xbWQgb3IgLm1kIGZpbGUuJyk7XG4gICAgfSBlbHNlIHtcbiAgICAgIG5ldyBOb3RpY2UoJ1F1YXJ0byBjb21tYW5kcyByZXF1aXJlIGFuIGFjdGl2ZSAucW1kIGZpbGUuJyk7XG4gICAgfVxuICAgIHJldHVybiBudWxsO1xuICB9XG5cbiAgZ2V0VmF1bHRGdWxsUGF0aChmaWxlOiBURmlsZSk6IHN0cmluZyB8IG51bGwge1xuICAgIGNvbnN0IGFkYXB0ZXIgPSB0aGlzLmFwcC52YXVsdC5hZGFwdGVyO1xuICAgIGlmIChhZGFwdGVyIGluc3RhbmNlb2YgRmlsZVN5c3RlbUFkYXB0ZXIpIHtcbiAgICAgIHJldHVybiBhZGFwdGVyLmdldEZ1bGxQYXRoKGZpbGUucGF0aCk7XG4gICAgfVxuICAgIG5ldyBOb3RpY2UoJ1ZhdWx0IGlzIG5vdCBvbiBhIGxvY2FsIGZpbGVzeXN0ZW07IGNhbm5vdCBydW4gUXVhcnRvLicpO1xuICAgIHJldHVybiBudWxsO1xuICB9XG5cbiAgcGRmUGF0aEZvcihmaWxlOiBURmlsZSk6IHN0cmluZyB7XG4gICAgcmV0dXJuIGZpbGUucGF0aC5yZXBsYWNlKC9cXC4ocW1kfG1kKSQvaSwgJy5wZGYnKTtcbiAgfVxuXG4gIC8vIFB1bGwgdGhlIFRGaWxlIGEgbGVhZiBjdXJyZW50bHkgc2hvd3MsIHdpdGhvdXQgcmVzb3J0aW5nIHRvXG4gIC8vIGBhcyBhbnlgLiBUaGUgYnVpbHQtaW4gUERGIHZpZXcgKGFuZCBtb3N0IGZpbGUtYmFja2VkIHZpZXdzKVxuICAvLyBleHRlbmQgRmlsZVZpZXcsIHdoaWNoIGV4cG9zZXMgYSB0eXBlZCBgZmlsZTogVEZpbGUgfCBudWxsYC5cbiAgcHJpdmF0ZSBsZWFmRmlsZShsZWFmOiBXb3Jrc3BhY2VMZWFmKTogVEZpbGUgfCBudWxsIHtcbiAgICByZXR1cm4gbGVhZi52aWV3IGluc3RhbmNlb2YgRmlsZVZpZXcgPyBsZWFmLnZpZXcuZmlsZSA6IG51bGw7XG4gIH1cblxuICAvLyBPcGVuIChvciByZXVzZSkgYSBsZWFmIHNob3dpbmcgdGhlIGdpdmVuIHZhdWx0LXJlbGF0aXZlIFBERiBwYXRoIGluXG4gIC8vIE9ic2lkaWFuJ3MgbmF0aXZlIFBERiB2aWV3ZXIuIFJldHVybnMgdGhlIGxlYWYgc28gY2FsbGVycyBjYW4ga2VlcFxuICAvLyByZWZyZXNoaW5nIGl0IG9uIHN1YnNlcXVlbnQgcHJldmlldyBjb21waWxlcy5cbiAgLy9cbiAgLy8gTGVhZi1yZXNvbHV0aW9uIG9yZGVyOlxuICAvLyAgIDEuIENhbGxlcidzIGNhcHR1cmVkIHJlZiwgaWYgc3RpbGwgYXR0YWNoZWQgdG8gdGhlIHdvcmtzcGFjZS5cbiAgLy8gICAyLiBBbnkgb3BlbiAncGRmJyBsZWFmIGFscmVhZHkgc2hvd2luZyB0aGlzIGV4YWN0IGZpbGUgKHVzZXIgbWF5XG4gIC8vICAgICAgaGF2ZSBvcGVuZWQgaXQgbWFudWFsbHksIG9yIHJlbmRlclBkZiBtYXkgaGF2ZSBvcGVuZWQgaXQpLlxuICAvLyAgIDMuIE5ldyB2ZXJ0aWNhbCBzcGxpdC5cbiAgYXN5bmMgb3Blbk9yUmVmcmVzaFBkZlByZXZpZXcoXG4gICAgdmF1bHRQYXRoOiBzdHJpbmcsXG4gICAgZXhpc3RpbmdMZWFmOiBXb3Jrc3BhY2VMZWFmIHwgbnVsbFxuICApOiBQcm9taXNlPFdvcmtzcGFjZUxlYWYgfCBudWxsPiB7XG4gICAgY29uc3QgcGRmVEZpbGUgPSBhd2FpdCB0aGlzLndhaXRGb3JWYXVsdEZpbGUodmF1bHRQYXRoKTtcbiAgICBpZiAoIXBkZlRGaWxlKSB7XG4gICAgICBuZXcgTm90aWNlKFxuICAgICAgICBgUXVhcnRvIHByZXZpZXcgcHJvZHVjZWQgJHt2YXVsdFBhdGh9IGJ1dCBpdCBkaWQgbm90IGFwcGVhciBpbiB0aGUgdmF1bHQgd2l0aGluIHRoZSB0aW1lb3V0LmBcbiAgICAgICk7XG4gICAgICByZXR1cm4gbnVsbDtcbiAgICB9XG4gICAgdHJ5IHtcbiAgICAgIGNvbnN0IHJldXNhYmxlID1cbiAgICAgICAgZXhpc3RpbmdMZWFmPy5wYXJlbnQgIT0gbnVsbFxuICAgICAgICAgID8gZXhpc3RpbmdMZWFmXG4gICAgICAgICAgOiB0aGlzLmFwcC53b3Jrc3BhY2VcbiAgICAgICAgICAgICAgLmdldExlYXZlc09mVHlwZSgncGRmJylcbiAgICAgICAgICAgICAgLmZpbmQoKGwpID0+IHRoaXMubGVhZkZpbGUobCk/LnBhdGggPT09IHBkZlRGaWxlLnBhdGgpID8/IG51bGw7XG4gICAgICBjb25zdCBsZWFmID0gcmV1c2FibGUgPz8gdGhpcy5hcHAud29ya3NwYWNlLmdldExlYWYoJ3NwbGl0JywgJ3ZlcnRpY2FsJyk7XG4gICAgICAvLyBTa2lwIHRoZSBvcGVuRmlsZSBjYWxsIHdoZW4gdGhlIGxlYWYgYWxyZWFkeSBzaG93cyB0aGlzIGZpbGUg4oCUXG4gICAgICAvLyBjYWxsaW5nIG9wZW5GaWxlIGluIHRoYXQgY2FzZSBpcyBoYXJtbGVzcyBmb3IgdGhlIGZpbGUgZGlzcGxheVxuICAgICAgLy8gYnV0IHN0aWxsIHRyaWdnZXJzIGEgcmV2ZWFsL2ZvY3VzIHNodWZmbGUgdGhlIHVzZXIgZG9lcyBub3Qgd2FudC5cbiAgICAgIC8vIE9ic2lkaWFuJ3MgUERGIHZpZXdlciBwaWNrcyB1cCB0aGUgZmlsZSByZXdyaXRlIHZpYSBpdHMgb3duXG4gICAgICAvLyBtdGltZSB3YXRjaGVyLCBzbyBsaXZlIHJlbG9hZCBzdGlsbCB3b3JrcyB3aXRob3V0IG91ciBoZWxwLlxuICAgICAgY29uc3QgY3VycmVudEZpbGUgPSB0aGlzLmxlYWZGaWxlKGxlYWYpO1xuICAgICAgaWYgKCFjdXJyZW50RmlsZSB8fCBjdXJyZW50RmlsZS5wYXRoICE9PSBwZGZURmlsZS5wYXRoKSB7XG4gICAgICAgIGF3YWl0IGxlYWYub3BlbkZpbGUocGRmVEZpbGUsIHsgYWN0aXZlOiBmYWxzZSB9KTtcbiAgICAgIH1cbiAgICAgIGF3YWl0IHRoaXMuYXBwLndvcmtzcGFjZS5yZXZlYWxMZWFmKGxlYWYpO1xuICAgICAgcmV0dXJuIGxlYWY7XG4gICAgfSBjYXRjaCAoZXJyKSB7XG4gICAgICBjb25zb2xlLmVycm9yKCdbcW1kLWFzLW1kXSBGYWlsZWQgdG8gb3BlbiBQREYgcHJldmlldyBpbiBuYXRpdmUgdmlld2VyOicsIGVycik7XG4gICAgICBuZXcgTm90aWNlKGBDb3VsZCBub3Qgb3BlbiAke3ZhdWx0UGF0aH0gaW4gT2JzaWRpYW4ncyBQREYgdmlld2VyLmApO1xuICAgICAgcmV0dXJuIG51bGw7XG4gICAgfVxuICB9XG5cbiAgYXN5bmMgb3BlblByZXZpZXdVcmwodXJsOiBzdHJpbmcsIG1vZGU6IFByZXZpZXdNb2RlKSB7XG4gICAgY29uc29sZS5sb2coJ1txbWQtYXMtbWRdW2RpYWddIG9wZW5QcmV2aWV3VXJsIGNhbGxlZC4gdXJsOicsIHVybCwgJ21vZGU6JywgbW9kZSk7XG4gICAgbmV3IE5vdGljZShgUHJldmlldyBhdmFpbGFibGUgYXQgJHt1cmx9YCk7XG5cbiAgICBpZiAobW9kZSA9PT0gJ2V4dGVybmFsJykge1xuICAgICAgLy8gUXVhcnRvIGlzIGxhdW5jaGVkIHdpdGggLS1uby1icm93c2VyIGluIGV2ZXJ5IG1vZGU7IHRoaXMgb3BlbnMgdGhlXG4gICAgICAvLyBjYXB0dXJlZCBVUkwgb25jZSB3aGlsZSBsZWF2aW5nIFF1YXJ0bydzIGxpdmUtcmVsb2FkIGNsaWVudCBpbiBjaGFyZ2VcbiAgICAgIC8vIGFmdGVyIHRoZSBwYWdlIGlzIGxvYWRlZC5cbiAgICAgIHRyeSB7XG4gICAgICAgIGF3YWl0IHNoZWxsLm9wZW5FeHRlcm5hbCh1cmwpO1xuICAgICAgfSBjYXRjaCAoZXJyKSB7XG4gICAgICAgIGNvbnNvbGUuZXJyb3IoJ1txbWQtYXMtbWRdIEZhaWxlZCB0byBvcGVuIGV4dGVybmFsIHByZXZpZXc6JywgZXJyKTtcbiAgICAgICAgbmV3IE5vdGljZShgQ291bGQgbm90IG9wZW4gZXh0ZXJuYWwgYnJvd3Nlci4gUHJldmlldyBVUkw6ICR7dXJsfWAsIDEwMDAwKTtcbiAgICAgIH1cbiAgICAgIHJldHVybjtcbiAgICB9XG5cbiAgICAvLyBUaGUgXCJXZWIgdmlld2VyXCIgY29yZSBwbHVnaW4gKE9ic2lkaWFuIDEuOCspIHJlZ2lzdGVycyB0aGVcbiAgICAvLyAnd2Vidmlld2VyJyB2aWV3IHR5cGUuIElmIHRoZSB1c2VyIGhhcyBpdCBkaXNhYmxlZCwgc2V0Vmlld1N0YXRlXG4gICAgLy8gc2lsZW50bHkgZmFpbHMgLyBsZWF2ZXMgYW4gZW1wdHkgbGVhZiwgYW5kIHRoZSB1c2VyIGlzIGxlZnRcbiAgICAvLyB3b25kZXJpbmcgd2h5IG5vdGhpbmcgb3BlbmVkLiBEZXRlY3QgYW5kIHJlcG9ydCBpbnN0ZWFkLlxuICAgIGNvbnN0IGludGVybmFsUGx1Z2lucyA9IChcbiAgICAgIHRoaXMuYXBwIGFzIEFwcCAmIHtcbiAgICAgICAgaW50ZXJuYWxQbHVnaW5zPzoge1xuICAgICAgICAgIGdldEVuYWJsZWRQbHVnaW5CeUlkPzogKGlkOiBzdHJpbmcpID0+IHVua25vd247XG4gICAgICAgICAgcGx1Z2lucz86IFJlY29yZDxzdHJpbmcsIHsgZW5hYmxlZD86IGJvb2xlYW4gfSB8IHVuZGVmaW5lZD47XG4gICAgICAgIH07XG4gICAgICB9XG4gICAgKS5pbnRlcm5hbFBsdWdpbnM7XG4gICAgY29uc3Qgd2Vidmlld2VyT24gPVxuICAgICAgaW50ZXJuYWxQbHVnaW5zPy5nZXRFbmFibGVkUGx1Z2luQnlJZD8uKCd3ZWJ2aWV3ZXInKSAhPSBudWxsIHx8XG4gICAgICBpbnRlcm5hbFBsdWdpbnM/LnBsdWdpbnM/LndlYnZpZXdlcj8uZW5hYmxlZCA9PT0gdHJ1ZTtcblxuICAgIGlmICghd2Vidmlld2VyT24pIHtcbiAgICAgIG5ldyBOb3RpY2UoXG4gICAgICAgICdPYnNpZGlhbiBjb3JlIHBsdWdpbiBcIldlYiB2aWV3ZXJcIiBpcyBkaXNhYmxlZCDigJQgY2Fubm90IHNob3cgcHJldmlldyBpbi1hcHAuICcgK1xuICAgICAgICAgICdFbmFibGUgaXQgaW4gU2V0dGluZ3Mg4oaSIENvcmUgcGx1Z2lucywgb3IgdXNlIFwiVG9nZ2xlIFF1YXJ0byBwcmV2aWV3IGluIGV4dGVybmFsIGJyb3dzZXJcIiBpbnN0ZWFkLiAnICtcbiAgICAgICAgICAnRmFsbGluZyBiYWNrIHRvIHlvdXIgZXh0ZXJuYWwgYnJvd3Nlci4nLFxuICAgICAgICAxMDAwMFxuICAgICAgKTtcbiAgICAgIGNvbnNvbGUud2FybihcbiAgICAgICAgJ1txbWQtYXMtbWRdIHdlYnZpZXdlciBjb3JlIHBsdWdpbiBkaXNhYmxlZDsgcHJldmlldyBVUkwgd2FzOicsXG4gICAgICAgIHVybFxuICAgICAgKTtcbiAgICAgIHZvaWQgc2hlbGwub3BlbkV4dGVybmFsKHVybCk7XG4gICAgICByZXR1cm47XG4gICAgfVxuXG4gICAgdHJ5IHtcbiAgICAgIGNvbnN0IGxlYWYgPSB0aGlzLmFwcC53b3Jrc3BhY2UuZ2V0TGVhZigndGFiJyk7XG4gICAgICBhd2FpdCBsZWFmLnNldFZpZXdTdGF0ZSh7XG4gICAgICAgIHR5cGU6ICd3ZWJ2aWV3ZXInLFxuICAgICAgICBhY3RpdmU6IHRydWUsXG4gICAgICAgIHN0YXRlOiB7IHVybCB9LFxuICAgICAgfSk7XG4gICAgICBhd2FpdCB0aGlzLmFwcC53b3Jrc3BhY2UucmV2ZWFsTGVhZihsZWFmKTtcbiAgICB9IGNhdGNoIChlcnIpIHtcbiAgICAgIGNvbnNvbGUuZXJyb3IoJ1txbWQtYXMtbWRdIEZhaWxlZCB0byBvcGVuIHByZXZpZXcgaW4gd2Vidmlld2VyOicsIGVycik7XG4gICAgICBuZXcgTm90aWNlKFxuICAgICAgICBcIkNvdWxkIG5vdCBvcGVuIHByZXZpZXcgaW4gT2JzaWRpYW4ncyB3ZWIgdmlld2VyLiBGYWxsaW5nIGJhY2sgdG8gZXh0ZXJuYWwgYnJvd3Nlci5cIlxuICAgICAgKTtcbiAgICAgIHZvaWQgc2hlbGwub3BlbkV4dGVybmFsKHVybCk7XG4gICAgfVxuICB9XG5cbiAgcmVnaXN0ZXJSZW5kZXJDb21tYW5kKGlkOiBzdHJpbmcsIG5hbWU6IHN0cmluZywgdG9Gb3JtYXQ/OiAncGRmJyB8ICd0eXBzdCcpIHtcbiAgICB0aGlzLmFkZENvbW1hbmQoe1xuICAgICAgaWQsXG4gICAgICBuYW1lLFxuICAgICAgaWNvbjogJ2ZpbGUtb3V0cHV0JyxcbiAgICAgIGNhbGxiYWNrOiBhc3luYyAoKSA9PiB7XG4gICAgICAgIGNvbnN0IGZpbGUgPSB0aGlzLmdldEFjdGl2ZVF1YXJ0b0NvbW1hbmRGaWxlKCk7XG4gICAgICAgIGlmIChmaWxlKSBhd2FpdCB0aGlzLnJlbmRlclBkZihmaWxlLCB0b0Zvcm1hdCk7XG4gICAgICB9LFxuICAgIH0pO1xuICB9XG5cbiAgcmVnaXN0ZXJRbWRFeHRlbnNpb24oKSB7XG4gICAgdGhpcy5yZWdpc3RlckV4dGVuc2lvbnMoWydxbWQnXSwgJ21hcmtkb3duJyk7XG4gIH1cblxuICByZWdpc3RlcllhbWxFeHRlbnNpb25zKCkge1xuICAgIHRoaXMucmVnaXN0ZXJFeHRlbnNpb25zKFsneW1sJywgJ3lhbWwnXSwgUU1EX1lBTUxfVklFVyk7XG4gIH1cblxuICByZWdpc3Rlckx1YUV4dGVuc2lvbnMoKSB7XG4gICAgdGhpcy5yZWdpc3RlckV4dGVuc2lvbnMoWydsdWEnXSwgUU1EX0xVQV9WSUVXKTtcbiAgfVxuXG4gIC8vIE9wZW4gdGhlIFF1YXJ0byBvdXRsaW5lIGluIHRoZSByaWdodCBzaWRlYmFyLCByZXVzaW5nIGFuIGV4aXN0aW5nXG4gIC8vIG91dGxpbmUgbGVhZiBpZiBvbmUgaXMgYWxyZWFkeSBvcGVuLlxuICBhc3luYyBhY3RpdmF0ZU91dGxpbmVWaWV3KCk6IFByb21pc2U8dm9pZD4ge1xuICAgIGNvbnN0IHsgd29ya3NwYWNlIH0gPSB0aGlzLmFwcDtcbiAgICAvLyBDYXB0dXJlIHRoZSBjdXJyZW50IC5xbWQgYmVmb3JlIG9wZW5pbmcgdGhlIG91dGxpbmUg4oCUIHNldFZpZXdTdGF0ZVxuICAgIC8vIHdpdGggYWN0aXZlOnRydWUgbWFrZXMgdGhlIG91dGxpbmUgdGhlIGFjdGl2ZSBsZWFmLCBhZnRlciB3aGljaCB0aGVcbiAgICAvLyBhY3RpdmUgbWFya2Rvd24gdmlldyBpcyBnb25lLlxuICAgIHRoaXMudHJhY2tBY3RpdmVRdWFydG9GaWxlKCk7XG4gICAgbGV0IGxlYWY6IFdvcmtzcGFjZUxlYWYgfCBudWxsID0gd29ya3NwYWNlLmdldExlYXZlc09mVHlwZShRTURfT1VUTElORV9WSUVXKVswXSA/PyBudWxsO1xuICAgIGlmICghbGVhZikge1xuICAgICAgbGVhZiA9IHdvcmtzcGFjZS5nZXRSaWdodExlYWYoZmFsc2UpO1xuICAgICAgYXdhaXQgbGVhZj8uc2V0Vmlld1N0YXRlKHsgdHlwZTogUU1EX09VVExJTkVfVklFVywgYWN0aXZlOiB0cnVlIH0pO1xuICAgIH1cbiAgICBpZiAobGVhZikgYXdhaXQgd29ya3NwYWNlLnJldmVhbExlYWYobGVhZik7XG4gICAgdGhpcy5yZWZyZXNoT3V0bGluZVZpZXdzKCk7XG4gIH1cblxuICAvLyBSZW1lbWJlciB0aGUgYWN0aXZlIGZpbGUgdGhlIG91dGxpbmUgc2hvdWxkIGRlc2NyaWJlLiBDYWxsZWQgd2hlbmV2ZXIgdGhlXG4gIC8vIGFjdGl2ZSBsZWFmIGNoYW5nZXM7IGFuIGFjdGl2ZSBsZWFmIHRoZSBvdXRsaW5lIGNhbm5vdCBkZXNjcmliZSAoYSBub24tLnFtZFxuICAvLyBmaWxlLCBvciBhIC5tZCBmaWxlIHdoZW4gb3V0bGluZU1hcmtkb3duRmlsZXMgaXMgb2ZmLCBpbmNsdWRpbmcgdGhlIG91dGxpbmVcbiAgLy8gc2lkZWJhciBpdHNlbGYpIGxlYXZlcyB0aGUgbGFzdCB2YWx1ZSB1bnRvdWNoZWQgc28gdGhlIG91dGxpbmUga2VlcHNcbiAgLy8gZGVzY3JpYmluZyB0aGF0IGZpbGUuXG4gIHRyYWNrQWN0aXZlUXVhcnRvRmlsZSgpOiB2b2lkIHtcbiAgICBjb25zdCB2aWV3ID0gdGhpcy5hcHAud29ya3NwYWNlLmdldEFjdGl2ZVZpZXdPZlR5cGUoTWFya2Rvd25WaWV3KTtcbiAgICBpZiAoIXZpZXc/LmZpbGUpIHJldHVybjtcbiAgICBpZiAoXG4gICAgICB0aGlzLmlzUXVhcnRvRmlsZSh2aWV3LmZpbGUpIHx8XG4gICAgICAodGhpcy5zZXR0aW5ncy5vdXRsaW5lTWFya2Rvd25GaWxlcyAmJiB0aGlzLmlzTWFya2Rvd25GaWxlKHZpZXcuZmlsZSkpXG4gICAgKSB7XG4gICAgICB0aGlzLmxhc3RBY3RpdmVRdWFydG9GaWxlID0gdmlldy5maWxlO1xuICAgIH1cbiAgfVxuXG4gIC8vIFJlLXJlbmRlciBldmVyeSBvcGVuIG91dGxpbmUgdmlldy4gTm8tb3Agd2hlbiBub25lIGFyZSBvcGVuLlxuICByZWZyZXNoT3V0bGluZVZpZXdzKCk6IHZvaWQge1xuICAgIGZvciAoY29uc3QgbGVhZiBvZiB0aGlzLmFwcC53b3Jrc3BhY2UuZ2V0TGVhdmVzT2ZUeXBlKFFNRF9PVVRMSU5FX1ZJRVcpKSB7XG4gICAgICBpZiAobGVhZi52aWV3IGluc3RhbmNlb2YgUW1kT3V0bGluZVZpZXcpIHtcbiAgICAgICAgbGVhZi52aWV3LnJlbmRlcigpO1xuICAgICAgfVxuICAgIH1cbiAgfVxuXG4gIC8vIENsb3NlIGFueSBvcGVuIG91dGxpbmUgdmlld3Mg4oCUIHVzZWQgd2hlbiB0aGUgdXNlciB0dXJucyB0aGUgc2V0dGluZyBvZmYuXG4gIGRldGFjaE91dGxpbmVWaWV3cygpOiB2b2lkIHtcbiAgICBmb3IgKGNvbnN0IGxlYWYgb2YgdGhpcy5hcHAud29ya3NwYWNlLmdldExlYXZlc09mVHlwZShRTURfT1VUTElORV9WSUVXKSkge1xuICAgICAgbGVhZi5kZXRhY2goKTtcbiAgICB9XG4gIH1cblxuICBwcml2YXRlIGRlZmF1bHRQcmV2aWV3TW9kZSgpOiBQcmV2aWV3TW9kZSB7XG4gICAgcmV0dXJuIHRoaXMuc2V0dGluZ3MucHJldmlld0luT2JzaWRpYW4gPyAnb2JzaWRpYW4nIDogJ2V4dGVybmFsJztcbiAgfVxuXG4gIGFzeW5jIHRvZ2dsZVByZXZpZXcoZmlsZTogVEZpbGUsIG1vZGU6IFByZXZpZXdNb2RlID0gdGhpcy5kZWZhdWx0UHJldmlld01vZGUoKSkge1xuICAgIGNvbnN0IGFjdGl2ZVByZXZpZXcgPSB0aGlzLmFjdGl2ZVByZXZpZXdQcm9jZXNzZXMuZ2V0KGZpbGUucGF0aCk7XG4gICAgaWYgKGFjdGl2ZVByZXZpZXc/Lm1vZGUgPT09IG1vZGUpIHtcbiAgICAgIGF3YWl0IHRoaXMuc3RvcFByZXZpZXcoZmlsZSk7XG4gICAgfSBlbHNlIHtcbiAgICAgIGlmIChhY3RpdmVQcmV2aWV3KSB7XG4gICAgICAgIGF3YWl0IHRoaXMuc3RvcFByZXZpZXcoZmlsZSk7XG4gICAgICB9XG4gICAgICBhd2FpdCB0aGlzLnN0YXJ0UHJldmlldyhmaWxlLCBtb2RlKTtcbiAgICB9XG4gIH1cblxuICBhc3luYyBzdGFydFByZXZpZXcoZmlsZTogVEZpbGUsIG1vZGU6IFByZXZpZXdNb2RlID0gdGhpcy5kZWZhdWx0UHJldmlld01vZGUoKSkge1xuICAgIGNvbnN0IGFjdGl2ZVByZXZpZXcgPSB0aGlzLmFjdGl2ZVByZXZpZXdQcm9jZXNzZXMuZ2V0KGZpbGUucGF0aCk7XG4gICAgaWYgKGFjdGl2ZVByZXZpZXc/Lm1vZGUgPT09IG1vZGUpIHtcbiAgICAgIHJldHVybjsgLy8gUHJldmlldyBhbHJlYWR5IHJ1bm5pbmcgZm9yIHRoaXMgZmlsZSBpbiB0aGlzIG1vZGUuXG4gICAgfVxuICAgIGlmIChhY3RpdmVQcmV2aWV3KSB7XG4gICAgICBhd2FpdCB0aGlzLnN0b3BQcmV2aWV3KGZpbGUpO1xuICAgIH1cblxuICAgIHRyeSB7XG4gICAgICBjb25zdCBhYnN0cmFjdEZpbGUgPSB0aGlzLmFwcC52YXVsdC5nZXRBYnN0cmFjdEZpbGVCeVBhdGgoZmlsZS5wYXRoKTtcbiAgICAgIGlmICghYWJzdHJhY3RGaWxlIHx8ICEoYWJzdHJhY3RGaWxlIGluc3RhbmNlb2YgVEZpbGUpKSB7XG4gICAgICAgIG5ldyBOb3RpY2UoYEZpbGUgJHtmaWxlLnBhdGh9IG5vdCBmb3VuZGApO1xuICAgICAgICByZXR1cm47XG4gICAgICB9XG4gICAgICBjb25zdCBmaWxlUGF0aCA9IHRoaXMuZ2V0VmF1bHRGdWxsUGF0aChhYnN0cmFjdEZpbGUpO1xuICAgICAgaWYgKCFmaWxlUGF0aCkgcmV0dXJuO1xuICAgICAgY29uc3Qgd29ya2luZ0RpciA9IHBhdGguZGlybmFtZShmaWxlUGF0aCk7XG5cbiAgICAgIGNvbnN0IGVudlZhcnM6IE5vZGVKUy5Qcm9jZXNzRW52ID0geyAuLi5wcm9jZXNzLmVudiB9O1xuICAgICAgaWYgKHRoaXMuc2V0dGluZ3MucXVhcnRvVHlwc3QudHJpbSgpKSB7XG4gICAgICAgIGVudlZhcnMuUVVBUlRPX1RZUFNUID0gdGhpcy5zZXR0aW5ncy5xdWFydG9UeXBzdC50cmltKCk7XG4gICAgICB9XG5cbiAgICAgIC8vIEFsd2F5cyBzdXBwcmVzcyBRdWFydG8ncyBvd24gYnJvd3NlciBsYXVuY2guIFRoZSBwbHVnaW4gb3BlbnMgdGhlXG4gICAgICAvLyBjYXB0dXJlZCBVUkwgZXhhY3RseSBvbmNlIGZvciB0aGUgc2VsZWN0ZWQgdGFyZ2V0LCB3aGljaCBhdm9pZHNcbiAgICAgIC8vIGR1cGxpY2F0ZSB0YWJzIGFuZCBhdm9pZHMgUXVhcnRvLW1hbmFnZWQgYnJvd3NlciBuYXZpZ2F0aW9uIGNsb3NpbmdcbiAgICAgIC8vIHRoZSBwcmV2aWV3IHByb2Nlc3Mgb24gc3Vic2VxdWVudCBzb3VyY2UgY2hhbmdlcy5cbiAgICAgIGNvbnN0IGFyZ3MgPSBbJ3ByZXZpZXcnLCBmaWxlUGF0aCwgJy0tbm8tYnJvd3NlciddO1xuXG4gICAgICAvLyBkZXRhY2hlZDogYHF1YXJ0byBwcmV2aWV3YCBmb3JrcyBhIHNlcGFyYXRlIGxvbmctbGl2ZWQgc2VydmVyXG4gICAgICAvLyBwcm9jZXNzLiBNYWtpbmcgdGhlIHNwYXduZWQgcHJvY2VzcyBhIHByb2Nlc3MtZ3JvdXAgbGVhZGVyIChQT1NJWClcbiAgICAgIC8vIGxldHMga2lsbFByZXZpZXdQcm9jZXNzIHNpZ25hbCB0aGUgd2hvbGUgZ3JvdXAg4oCUIGEgcGxhaW4ga2lsbCgpIG9mXG4gICAgICAvLyB0aGUgd3JhcHBlciB3b3VsZCBvcnBoYW4gdGhlIHNlcnZlciwgbGVhdmluZyBpdCBzZXJ2aW5nIGFuZFxuICAgICAgLy8gcmVjb21waWxpbmcgYWZ0ZXIgXCJzdG9wXCIuIE5vIHByb2Nlc3MgZ3JvdXBzIG9uIFdpbmRvd3M7IHRoZSBraWxsXG4gICAgICAvLyB0aGVyZSBnb2VzIHRocm91Z2ggdGFza2tpbGwgaW5zdGVhZC5cbiAgICAgIGNvbnN0IHF1YXJ0b1Byb2Nlc3MgPSBzcGF3bihcbiAgICAgICAgdGhpcy5zZXR0aW5ncy5xdWFydG9QYXRoLFxuICAgICAgICBhcmdzLFxuICAgICAgICB7XG4gICAgICAgICAgY3dkOiB3b3JraW5nRGlyLFxuICAgICAgICAgIGVudjogZW52VmFycyxcbiAgICAgICAgICBkZXRhY2hlZDogcHJvY2Vzcy5wbGF0Zm9ybSAhPT0gJ3dpbjMyJyxcbiAgICAgICAgfVxuICAgICAgKTtcblxuICAgICAgbGV0IHByZXZpZXdVcmw6IHN0cmluZyB8IG51bGwgPSBudWxsO1xuXG4gICAgICAvLyBQREYtcHJldmlldyBzdGF0ZS4gUXVhcnRvIGVtaXRzIFwiT3V0cHV0IGNyZWF0ZWQ6IGZvby5wZGZcIiBvblxuICAgICAgLy8gZXZlcnkgcmVjb21waWxlIChvZnRlbiBzZXZlcmFsIHRpbWVzIHBlciByZWNvbXBpbGUpOyB0aGVcbiAgICAgIC8vIGhhbmRsZXIgZGVkdXBzIHNvIHdlIGRvbid0IHNwYXduIHRhYnMgb24gZXZlcnkgc2F2ZS5cbiAgICAgIC8vXG4gICAgICAvLyAgbGVhZjogIGN1cnJlbnQgUERGIHRhYiwgaWYgYW55LlxuICAgICAgLy8gIHBhdGg6ICB0aGUgcGF0aCB0aGF0IGxlYWYgaXMgc2hvd2luZyAoYW5kIHRoZSBwYXRoIG9mIGFuXG4gICAgICAvLyAgICAgICAgIGluLWZsaWdodCBvcGVuIGNhbGwsIHJlY29yZGVkIHN5bmNocm9ub3VzbHkgd2hlbiBpdFxuICAgICAgLy8gICAgICAgICBpcyBzY2hlZHVsZWQpLiBBbHNvIGdhdGVzIHRoZSBwcmV2aWV3LVVSTCBza2lwIGxvZ2ljXG4gICAgICAvLyAgICAgICAgIG9uIHRoZSBcIkJyb3dzZSBhdFwiIGJyYW5jaCBiZWxvdyDigJQgd2hlbiBhIFBERlxuICAgICAgLy8gICAgICAgICBwcmV2aWV3IGlzIGFjdGl2ZSwgd2UgZG9uJ3Qgb3BlbiBRdWFydG8ncyBQREYuanNcbiAgICAgIC8vICAgICAgICAgd3JhcHBlciBwYWdlIGluIHRoZSB3ZWJ2aWV3ZXIgdG9vLlxuICAgICAgLy8gIGJ1c3k6ICBhIGNhbGwgdG8gb3Blbk9yUmVmcmVzaFBkZlByZXZpZXcgaXMgaW4gZmxpZ2h0LlxuICAgICAgLy9cbiAgICAgIC8vIFNjaGVkdWxlIG9wZW4gd2hlbiBhbnkgb2Y6XG4gICAgICAvLyAgIC0gbGVhZiBpcyBkZXRhY2hlZCAodXNlciBjbG9zZWQgdGhlIHRhYiBtYW51YWxseSlcbiAgICAgIC8vICAgLSB0aGUgbmV3IG91dHB1dCBwYXRoIGRpZmZlcnMgZnJvbSB0aGUgdHJhY2tlZCBvbmVcbiAgICAgIC8vICAgICAobXVsdGktZm9ybWF0IHByb2plY3QsIG9yIHJlbmFtZSlcbiAgICAgIC8vICAgLSB3ZSBuZXZlciBvcGVuZWQgaW4gdGhpcyBzZXNzaW9uXG4gICAgICAvLyBTa2lwIHdoZW4gYnVzeSAoYnVyc3RzIG9mIGVtaXNzaW9ucyBkdXJpbmcgcmVjb21waWxlIGRlZHVwXG4gICAgICAvLyBhdXRvbWF0aWNhbGx5OyB0aGUgZmluYWwgZW1pc3Npb24gb2YgYSBidXJzdCB3aW5zIGJlY2F1c2UgaXRcbiAgICAgIC8vIGFycml2ZXMgYWZ0ZXIgYnVzeSBjbGVhcnMpLlxuICAgICAgbGV0IHBkZlByZXZpZXdMZWFmOiBXb3Jrc3BhY2VMZWFmIHwgbnVsbCA9IG51bGw7XG4gICAgICBsZXQgcGRmUHJldmlld1BhdGg6IHN0cmluZyB8IG51bGwgPSBudWxsO1xuICAgICAgbGV0IHBkZlByZXZpZXdCdXN5ID0gZmFsc2U7XG5cbiAgICAgIGNvbnN0IHNjaGVkdWxlUGRmUHJldmlldyA9ICh2YXVsdFBhdGg6IHN0cmluZyk6IHZvaWQgPT4ge1xuICAgICAgICBpZiAocGRmUHJldmlld0J1c3kpIHJldHVybjtcbiAgICAgICAgY29uc3QgbGVhZkF0dGFjaGVkID0gcGRmUHJldmlld0xlYWY/LnBhcmVudCAhPSBudWxsO1xuICAgICAgICBjb25zdCBwYXRoU2FtZSA9IHBkZlByZXZpZXdQYXRoID09PSB2YXVsdFBhdGg7XG4gICAgICAgIGlmIChsZWFmQXR0YWNoZWQgJiYgcGF0aFNhbWUpIHJldHVybjtcblxuICAgICAgICBwZGZQcmV2aWV3QnVzeSA9IHRydWU7XG4gICAgICAgIHBkZlByZXZpZXdQYXRoID0gdmF1bHRQYXRoO1xuICAgICAgICB0aGlzLm9wZW5PclJlZnJlc2hQZGZQcmV2aWV3KHZhdWx0UGF0aCwgcGRmUHJldmlld0xlYWYpXG4gICAgICAgICAgLnRoZW4oKGxlYWYpID0+IHtcbiAgICAgICAgICAgIGlmIChsZWFmKSBwZGZQcmV2aWV3TGVhZiA9IGxlYWY7XG4gICAgICAgICAgfSlcbiAgICAgICAgICAuY2F0Y2goKGVycikgPT4ge1xuICAgICAgICAgICAgY29uc29sZS5lcnJvcignW3FtZC1hcy1tZF0gUERGIHByZXZpZXcgb3BlbiBmYWlsZWQ6JywgZXJyKTtcbiAgICAgICAgICAgIHBkZlByZXZpZXdMZWFmID0gbnVsbDtcbiAgICAgICAgICAgIHBkZlByZXZpZXdQYXRoID0gbnVsbDtcbiAgICAgICAgICB9KVxuICAgICAgICAgIC5maW5hbGx5KCgpID0+IHtcbiAgICAgICAgICAgIHBkZlByZXZpZXdCdXN5ID0gZmFsc2U7XG4gICAgICAgICAgfSk7XG4gICAgICB9O1xuXG4gICAgICAvLyBRdWFydG8gXCJFUlJPUjpcIiBsaW5lcyBmcm9tIHRoaXMgcHJldmlldyBydW4uIFVzZWQgYm90aCB0byBzdXJmYWNlXG4gICAgICAvLyByZWNvbXBpbGUgZmFpbHVyZXMgbGl2ZSAocHJldmlldyBrZWVwcyBydW5uaW5nLCBzbyB0aGUgY2xvc2VcbiAgICAgIC8vIGhhbmRsZXIgbmV2ZXIgZmlyZXMpIGFuZCB0byBleHBsYWluIGEgc3RhcnR1cCBleGl0IGluIHRoZSBOb3RpY2UuXG4gICAgICBjb25zdCBlcnJvckxpbmVzOiBzdHJpbmdbXSA9IFtdO1xuICAgICAgLy8gRGVkdXBlOiBhIHNpbmdsZSBmYWlsZWQgcmVjb21waWxlIGVtaXRzIHRoZSBzYW1lIEVSUk9SOiBibG9jayBvblxuICAgICAgLy8gZXZlcnkgc2F2ZSB1bnRpbCBmaXhlZCDigJQgb25seSBOb3RpY2Ugd2hlbiB0aGUgZXJyb3IgdGV4dCBjaGFuZ2VzLlxuICAgICAgbGV0IGxhc3RFcnJvclNob3duID0gJyc7XG5cbiAgICAgIC8vIFBlci1saW5lIGhhbmRsZXI6IGxvZyB0aGUgbGluZSwgdGhlbiBsb29rIGZvciB0aGUgdHdvIG1hcmtlcnNcbiAgICAgIC8vIHdlIGNhcmUgYWJvdXQgKFwiT3V0cHV0IGNyZWF0ZWQ6XCIgYW5kIFwiQnJvd3NlIGF0XCIpLlxuICAgICAgY29uc3QgaGFuZGxlUHJldmlld0xpbmUgPSAobGluZTogc3RyaW5nKSA9PiB7XG4gICAgICAgIGxvZ1F1YXJ0b0xpbmUoJ1F1YXJ0byBQcmV2aWV3JywgbGluZSk7XG5cbiAgICAgICAgaWYgKC9eRVJST1I6Ly50ZXN0KGxpbmUpKSB7XG4gICAgICAgICAgZXJyb3JMaW5lcy5wdXNoKGxpbmUpO1xuICAgICAgICAgIGlmIChsaW5lICE9PSBsYXN0RXJyb3JTaG93bikge1xuICAgICAgICAgICAgbGFzdEVycm9yU2hvd24gPSBsaW5lO1xuICAgICAgICAgICAgbmV3IE5vdGljZShgUXVhcnRvIHByZXZpZXcgZXJyb3I6XFxuJHtsaW5lfWAsIDE1MDAwKTtcbiAgICAgICAgICB9XG4gICAgICAgICAgcmV0dXJuO1xuICAgICAgICB9XG4gICAgICAgIC8vIEEgY2xlYW4gY29tcGlsZSBjbGVhcnMgdGhlIGRlZHVwZSBndWFyZCBzbyB0aGUgc2FtZSBlcnJvclxuICAgICAgICAvLyByZWFwcGVhcmluZyBhZnRlciBhIGdvb2QgYnVpbGQgaXMgc3VyZmFjZWQgYWdhaW4uXG4gICAgICAgIGlmIChsaW5lLmluY2x1ZGVzKCdPdXRwdXQgY3JlYXRlZDonKSkge1xuICAgICAgICAgIGxhc3RFcnJvclNob3duID0gJyc7XG4gICAgICAgIH1cblxuICAgICAgICAvLyBEZXRlY3QgXCJPdXRwdXQgY3JlYXRlZDogPHBhdGg+XCIg4oCUIHF1YXJ0byBwcmludHMgdGhpcyBvbiBldmVyeVxuICAgICAgICAvLyBjb21waWxlIGluIHByZXZpZXcgbW9kZS4gSWYgdGhlIG91dHB1dCBpcyBhIFBERiwgcm91dGUgdG9cbiAgICAgICAgLy8gT2JzaWRpYW4ncyBuYXRpdmUgUERGIHZpZXdlciByYXRoZXIgdGhhbiB0aGUgd2Vidmlld2VyIHBhZ2VcbiAgICAgICAgLy8gUXVhcnRvIHNlcnZlcyBhdCAvd2ViL3ZpZXdlci5odG1sLiBTdWJzZXF1ZW50IGNvbXBpbGVzIHJlZnJlc2hcbiAgICAgICAgLy8gdGhlIHNhbWUgbGVhZiBzbyBsaXZlIHJlbG9hZCBzdGlsbCB3b3Jrcy5cbiAgICAgICAgY29uc3Qgb3V0TWF0Y2ggPSBsaW5lLm1hdGNoKC9PdXRwdXQgY3JlYXRlZDpcXHMqKC4rPylcXHMqJC8pO1xuICAgICAgICBpZiAob3V0TWF0Y2ggJiYgL1xcLnBkZiQvaS50ZXN0KG91dE1hdGNoWzFdLnRyaW0oKSkgJiYgbW9kZSA9PT0gJ29ic2lkaWFuJykge1xuICAgICAgICAgIGNvbnN0IG91dEJhc2VuYW1lID0gcGF0aC5iYXNlbmFtZShvdXRNYXRjaFsxXS50cmltKCkpO1xuICAgICAgICAgIGNvbnN0IHNvdXJjZURpciA9IGZpbGUucGFyZW50Py5wYXRoID8/ICcnO1xuICAgICAgICAgIGNvbnN0IHZhdWx0UGF0aCA9IG5vcm1hbGl6ZVBhdGgoc291cmNlRGlyID8gYCR7c291cmNlRGlyfS8ke291dEJhc2VuYW1lfWAgOiBvdXRCYXNlbmFtZSk7XG4gICAgICAgICAgc2NoZWR1bGVQZGZQcmV2aWV3KHZhdWx0UGF0aCk7XG4gICAgICAgICAgcmV0dXJuO1xuICAgICAgICB9XG5cbiAgICAgICAgY29uc3QgbWF0Y2hlZFByZXZpZXdVcmwgPSBwcmV2aWV3VXJsRnJvbUxpbmUobGluZSk7XG4gICAgICAgIGlmICghcHJldmlld1VybCAmJiBtYXRjaGVkUHJldmlld1VybCkge1xuICAgICAgICAgIGNvbnNvbGUubG9nKFxuICAgICAgICAgICAgJ1txbWQtYXMtbWRdW2RpYWddIEJyb3dzZS1hdCBsaW5lIHNlZW4uJyxcbiAgICAgICAgICAgICdtYXRjaGVkOicsIG1hdGNoZWRQcmV2aWV3VXJsLFxuICAgICAgICAgICAgJ3BkZlByZXZpZXdQYXRoOicsIHBkZlByZXZpZXdQYXRoLFxuICAgICAgICAgICAgJ21vZGU6JywgbW9kZVxuICAgICAgICAgICk7XG4gICAgICAgICAgcHJldmlld1VybCA9IG1hdGNoZWRQcmV2aWV3VXJsO1xuICAgICAgICAgIC8vIElmIHdlIGFscmVhZHkgb3BlbmVkIGEgbmF0aXZlIFBERiBwcmV2aWV3LCBza2lwIHRoZVxuICAgICAgICAgIC8vIHdlYnZpZXdlciBVUkwg4oCUIFF1YXJ0bydzIFBERi5qcyB3cmFwcGVyIHdvdWxkIGp1c3QgYmVcbiAgICAgICAgICAvLyBhIHdvcnNlIHZlcnNpb24gb2YgdGhlIHNhbWUgY29udGVudC5cbiAgICAgICAgICBpZiAocGRmUHJldmlld1BhdGgpIHtcbiAgICAgICAgICAgIG5ldyBOb3RpY2UoYFBERiBwcmV2aWV3IG9wZW5lZCBuYXRpdmVseS4gU2VydmVyIFVSTDogJHtwcmV2aWV3VXJsfWApO1xuICAgICAgICAgIH0gZWxzZSB7XG4gICAgICAgICAgICB2b2lkIHRoaXMub3BlblByZXZpZXdVcmwocHJldmlld1VybCwgbW9kZSk7XG4gICAgICAgICAgfVxuICAgICAgICB9XG4gICAgICB9O1xuXG4gICAgICAvLyBPbmUgYnVmZmVyZWQgcHJvY2Vzc29yIHBlciBzdHJlYW0g4oCUIHN0ZG91dCBhbmQgc3RkZXJyIGVhY2hcbiAgICAgIC8vIG5lZWQgdGhlaXIgb3duIHBhcnRpYWwtbGluZSBidWZmZXIsIG9yIGludGVybGVhdmVkIGZyYWdtZW50c1xuICAgICAgLy8gZnJvbSB0aGUgdHdvIHN0cmVhbXMgd291bGQgYmUgc3BsaWNlZCBpbnRvIHN5bnRoZXRpYyBsaW5lcy5cbiAgICAgIGNvbnN0IHByZXZpZXdTdGRvdXQgPSBtYWtlTGluZVByb2Nlc3NvcihoYW5kbGVQcmV2aWV3TGluZSk7XG4gICAgICBjb25zdCBwcmV2aWV3U3RkZXJyID0gbWFrZUxpbmVQcm9jZXNzb3IoaGFuZGxlUHJldmlld0xpbmUpO1xuICAgICAgcXVhcnRvUHJvY2Vzcy5zdGRvdXQ/Lm9uKCdkYXRhJywgKGRhdGE6IEJ1ZmZlcikgPT4gcHJldmlld1N0ZG91dChkYXRhLnRvU3RyaW5nKCkpKTtcbiAgICAgIHF1YXJ0b1Byb2Nlc3Muc3RkZXJyPy5vbignZGF0YScsIChkYXRhOiBCdWZmZXIpID0+IHByZXZpZXdTdGRlcnIoZGF0YS50b1N0cmluZygpKSk7XG5cbiAgICAgIC8vIGNoaWxkX3Byb2Nlc3Muc3Bhd24gZG9lcyBub3QgdGhyb3cgb24gYSBtaXNzaW5nIGJpbmFyeTsgaXQgZW1pdHNcbiAgICAgIC8vIGFuICdlcnJvcicgZXZlbnQgbGF0ZXIuIFdpdGhvdXQgdGhpcyBsaXN0ZW5lciBhbiBFTk9FTlQganVzdFxuICAgICAgLy8gcHJvZHVjZWQgYSBzaWxlbnQgXCJleGl0IDFcIiBjbG9zZSB3aXRoIG5vIG91dHB1dCB0byBjb25zb2xlLlxuICAgICAgcXVhcnRvUHJvY2Vzcy5vbignZXJyb3InLCAoZXJyKSA9PiB7XG4gICAgICAgIGNvbnNvbGUuZXJyb3IoJ1txbWQtYXMtbWRdIEZhaWxlZCB0byBzcGF3biBxdWFydG8gZm9yIHByZXZpZXc6JywgZXJyKTtcbiAgICAgICAgbmV3IE5vdGljZShcbiAgICAgICAgICBgRmFpbGVkIHRvIHNwYXduICcke3RoaXMuc2V0dGluZ3MucXVhcnRvUGF0aH0nOiAke2Vyci5tZXNzYWdlfS4gYCArXG4gICAgICAgICAgICAnQ2hlY2sgdGhlIFF1YXJ0byBwYXRoIHNldHRpbmcgYW5kIHRoYXQgUXVhcnRvIGlzIG9uIFBBVEguJ1xuICAgICAgICApO1xuICAgICAgICBpZiAodGhpcy5hY3RpdmVQcmV2aWV3UHJvY2Vzc2VzLmdldChmaWxlLnBhdGgpPy5wcm9jZXNzID09PSBxdWFydG9Qcm9jZXNzKSB7XG4gICAgICAgICAgdGhpcy5hY3RpdmVQcmV2aWV3UHJvY2Vzc2VzLmRlbGV0ZShmaWxlLnBhdGgpO1xuICAgICAgICB9XG4gICAgICB9KTtcblxuICAgICAgcXVhcnRvUHJvY2Vzcy5vbignY2xvc2UnLCAoY29kZTogbnVtYmVyIHwgbnVsbCwgc2lnbmFsOiBOb2RlSlMuU2lnbmFscyB8IG51bGwpID0+IHtcbiAgICAgICAgcHJldmlld1N0ZG91dC5mbHVzaCgpOyAvLyByZWxlYXNlIGFueSBmaW5hbCBwYXJ0aWFsIGxpbmVcbiAgICAgICAgcHJldmlld1N0ZGVyci5mbHVzaCgpO1xuICAgICAgICBpZiAoY29kZSAhPT0gbnVsbCAmJiBjb2RlICE9PSAwKSB7XG4gICAgICAgICAgY29uc3QgcmVhc29uID0gZXJyb3JMaW5lcy5sZW5ndGggPiAwXG4gICAgICAgICAgICA/IGVycm9yTGluZXMuam9pbignXFxuJylcbiAgICAgICAgICAgIDogJ0NoZWNrIHRoZSBkZXZlbG9wZXIgY29uc29sZSBmb3IgZGV0YWlscy4nO1xuICAgICAgICAgIG5ldyBOb3RpY2UoYFF1YXJ0byBwcmV2aWV3IGV4aXRlZCB3aXRoIGNvZGUgJHtjb2RlfS5cXG4ke3JlYXNvbn1gLCAxNTAwMCk7XG4gICAgICAgIH0gZWxzZSBpZiAoY29kZSA9PT0gbnVsbCAmJiBzaWduYWwgJiYgc2lnbmFsICE9PSAnU0lHVEVSTScgJiYgc2lnbmFsICE9PSAnU0lHS0lMTCcpIHtcbiAgICAgICAgICAvLyBTSUdURVJNL1NJR0tJTEwgY29tZSBmcm9tIG91ciBvd24gc3RvcFByZXZpZXcgLyBvbnVubG9hZCDigJQgc2lsZW50LlxuICAgICAgICAgIG5ldyBOb3RpY2UoYFF1YXJ0byBwcmV2aWV3IHByb2Nlc3Mgd2FzIHRlcm1pbmF0ZWQgYnkgJHtzaWduYWx9YCk7XG4gICAgICAgIH1cbiAgICAgICAgaWYgKHRoaXMuYWN0aXZlUHJldmlld1Byb2Nlc3Nlcy5nZXQoZmlsZS5wYXRoKT8ucHJvY2VzcyA9PT0gcXVhcnRvUHJvY2Vzcykge1xuICAgICAgICAgIHRoaXMuYWN0aXZlUHJldmlld1Byb2Nlc3Nlcy5kZWxldGUoZmlsZS5wYXRoKTtcbiAgICAgICAgfVxuICAgICAgfSk7XG5cbiAgICAgIHRoaXMuYWN0aXZlUHJldmlld1Byb2Nlc3Nlcy5zZXQoZmlsZS5wYXRoLCB7IHByb2Nlc3M6IHF1YXJ0b1Byb2Nlc3MsIG1vZGUgfSk7XG4gICAgICBuZXcgTm90aWNlKGBRdWFydG8gcHJldmlldyBzdGFydGVkICgke21vZGUgPT09ICdvYnNpZGlhbicgPyAnT2JzaWRpYW4nIDogJ2V4dGVybmFsIGJyb3dzZXInfSlgKTtcbiAgICB9IGNhdGNoIChlcnJvcikge1xuICAgICAgY29uc29sZS5lcnJvcignRmFpbGVkIHRvIHN0YXJ0IFF1YXJ0byBwcmV2aWV3OicsIGVycm9yKTtcbiAgICAgIG5ldyBOb3RpY2UoJ0ZhaWxlZCB0byBzdGFydCBRdWFydG8gcHJldmlldycpO1xuICAgIH1cbiAgfVxuXG4gIC8vIGBxdWFydG8gcHJldmlld2AgZm9ya3MgYSBsb25nLWxpdmVkIHNlcnZlciBhcyBhIGNoaWxkIG9mIHRoZSBzcGF3bmVkXG4gIC8vIHByb2Nlc3MsIHNvIGtpbGxpbmcgb25seSB0aGUgd3JhcHBlciBsZWF2ZXMgdGhhdCBzZXJ2ZXIgcnVubmluZy4gU2lnbmFsXG4gIC8vIHRoZSB3aG9sZSBwcm9jZXNzIHRyZWU6IHRoZSBwcm9jZXNzIGdyb3VwIG9uIFBPU0lYICh0aGUgY2hpbGQgd2FzXG4gIC8vIHNwYXduZWQgZGV0YWNoZWQsIHNlZSBzdGFydFByZXZpZXcpLCBvciB0YXNra2lsbCAvdCBvbiBXaW5kb3dzLlxuICBwcml2YXRlIGtpbGxQcmV2aWV3UHJvY2VzcyhxdWFydG9Qcm9jZXNzOiBDaGlsZFByb2Nlc3MpOiB2b2lkIHtcbiAgICBpZiAocXVhcnRvUHJvY2Vzcy5raWxsZWQgfHwgcXVhcnRvUHJvY2Vzcy5waWQgPT09IHVuZGVmaW5lZCkgcmV0dXJuO1xuICAgIGlmIChwcm9jZXNzLnBsYXRmb3JtID09PSAnd2luMzInKSB7XG4gICAgICBzcGF3bigndGFza2tpbGwnLCBbJy9waWQnLCBTdHJpbmcocXVhcnRvUHJvY2Vzcy5waWQpLCAnL3QnLCAnL2YnXSk7XG4gICAgICByZXR1cm47XG4gICAgfVxuICAgIHRyeSB7XG4gICAgICAvLyBOZWdhdGl2ZSBQSUQgdGFyZ2V0cyB0aGUgd2hvbGUgcHJvY2VzcyBncm91cC5cbiAgICAgIHByb2Nlc3Mua2lsbCgtcXVhcnRvUHJvY2Vzcy5waWQsICdTSUdURVJNJyk7XG4gICAgfSBjYXRjaCB7XG4gICAgICAvLyBHcm91cCBhbHJlYWR5IGdvbmUsIG9yIG5ldmVyIGJlY2FtZSBhIGxlYWRlciDigJQgYmVzdC1lZmZvcnQgZGlyZWN0IGtpbGwuXG4gICAgICB0cnkge1xuICAgICAgICBxdWFydG9Qcm9jZXNzLmtpbGwoJ1NJR1RFUk0nKTtcbiAgICAgIH0gY2F0Y2gge1xuICAgICAgICAvKiBhbHJlYWR5IGRlYWQgKi9cbiAgICAgIH1cbiAgICB9XG4gIH1cblxuICBhc3luYyBzdG9wUHJldmlldyhmaWxlOiBURmlsZSkge1xuICAgIGNvbnN0IGFjdGl2ZVByZXZpZXcgPSB0aGlzLmFjdGl2ZVByZXZpZXdQcm9jZXNzZXMuZ2V0KGZpbGUucGF0aCk7XG4gICAgaWYgKGFjdGl2ZVByZXZpZXcpIHtcbiAgICAgIHRoaXMua2lsbFByZXZpZXdQcm9jZXNzKGFjdGl2ZVByZXZpZXcucHJvY2Vzcyk7XG4gICAgICB0aGlzLmFjdGl2ZVByZXZpZXdQcm9jZXNzZXMuZGVsZXRlKGZpbGUucGF0aCk7XG4gICAgICBuZXcgTm90aWNlKCdRdWFydG8gcHJldmlldyBzdG9wcGVkJyk7XG4gICAgfVxuICB9XG5cbiAgc3RvcEFsbFByZXZpZXdzKCkge1xuICAgIGNvbnN0IGhhZFByZXZpZXdzID0gdGhpcy5hY3RpdmVQcmV2aWV3UHJvY2Vzc2VzLnNpemUgPiAwO1xuICAgIHRoaXMuYWN0aXZlUHJldmlld1Byb2Nlc3Nlcy5mb3JFYWNoKChhY3RpdmVQcmV2aWV3LCBmaWxlUGF0aCkgPT4ge1xuICAgICAgdGhpcy5raWxsUHJldmlld1Byb2Nlc3MoYWN0aXZlUHJldmlldy5wcm9jZXNzKTtcbiAgICAgIHRoaXMuYWN0aXZlUHJldmlld1Byb2Nlc3Nlcy5kZWxldGUoZmlsZVBhdGgpO1xuICAgIH0pO1xuICAgIGlmIChoYWRQcmV2aWV3cykge1xuICAgICAgbmV3IE5vdGljZSgnQWxsIFF1YXJ0byBwcmV2aWV3cyBzdG9wcGVkJyk7XG4gICAgfVxuICB9XG5cbiAgYXN5bmMgcmVuZGVyUGRmKGZpbGU6IFRGaWxlLCB0b0Zvcm1hdD86ICdwZGYnIHwgJ3R5cHN0Jykge1xuICAgIHRyeSB7XG4gICAgICBjb25zdCBhYnN0cmFjdEZpbGUgPSB0aGlzLmFwcC52YXVsdC5nZXRBYnN0cmFjdEZpbGVCeVBhdGgoZmlsZS5wYXRoKTtcbiAgICAgIGlmICghYWJzdHJhY3RGaWxlIHx8ICEoYWJzdHJhY3RGaWxlIGluc3RhbmNlb2YgVEZpbGUpKSB7XG4gICAgICAgIG5ldyBOb3RpY2UoYEZpbGUgJHtmaWxlLnBhdGh9IG5vdCBmb3VuZGApO1xuICAgICAgICByZXR1cm47XG4gICAgICB9XG5cbiAgICAgIC8vIEEgcnVubmluZyBgcXVhcnRvIHByZXZpZXdgIGtlZXBzIHJlY29tcGlsaW5nIHRoZSBzYW1lIHNvdXJjZSBhbmRcbiAgICAgIC8vIHdyaXRlcyB0byBvdmVybGFwcGluZyBvdXRwdXQgcGF0aHMuIFN0b3AgaXQgYmVmb3JlIGEgb25lLXNob3RcbiAgICAgIC8vIHJlbmRlciBzbyB0aGUgdHdvIFF1YXJ0byBwcm9jZXNzZXMgZG8gbm90IGZpZ2h0IG92ZXIgdGhlIG91dHB1dC5cbiAgICAgIGlmICh0aGlzLmFjdGl2ZVByZXZpZXdQcm9jZXNzZXMuaGFzKGZpbGUucGF0aCkpIHtcbiAgICAgICAgYXdhaXQgdGhpcy5zdG9wUHJldmlldyhmaWxlKTtcbiAgICAgIH1cblxuICAgICAgY29uc3QgZmlsZVBhdGggPSB0aGlzLmdldFZhdWx0RnVsbFBhdGgoYWJzdHJhY3RGaWxlKTtcbiAgICAgIGlmICghZmlsZVBhdGgpIHJldHVybjtcbiAgICAgIGNvbnN0IHdvcmtpbmdEaXIgPSBwYXRoLmRpcm5hbWUoZmlsZVBhdGgpO1xuXG4gICAgICBjb25zdCBlbnZWYXJzOiBOb2RlSlMuUHJvY2Vzc0VudiA9IHsgLi4ucHJvY2Vzcy5lbnYgfTtcbiAgICAgIGlmICh0aGlzLnNldHRpbmdzLnF1YXJ0b1R5cHN0LnRyaW0oKSkge1xuICAgICAgICBlbnZWYXJzLlFVQVJUT19UWVBTVCA9IHRoaXMuc2V0dGluZ3MucXVhcnRvVHlwc3QudHJpbSgpO1xuICAgICAgfVxuXG4gICAgICBjb25zdCBlbmdpbmVMYWJlbCA9IHRvRm9ybWF0ID09PSAndHlwc3QnID8gJ1R5cHN0JyA6IHRvRm9ybWF0ID09PSAncGRmJyA/ICdMYVRlWCcgOiAnZm9ybWF0IGRlZmluZWQgaW4gWUFNTCc7XG4gICAgICBuZXcgTm90aWNlKGBSZW5kZXJpbmcgUXVhcnRvICgke2VuZ2luZUxhYmVsfSkuLi5gKTtcblxuICAgICAgLy8gQmVzdC1ndWVzcyBwYXRoIHVzZWQgZm9yIHRoZSBwcmUtcmVuZGVyIGxlYWYtY2FwdHVyZSAoc28gd2UgY2FuXG4gICAgICAvLyByZXVzZSBhbiBleGlzdGluZyBQREYgdGFiIG9uIHJlY29tcGlsZSkuIFRoZSBhdXRob3JpdGF0aXZlIHBhdGhcbiAgICAgIC8vIGNvbWVzIGZyb20gcXVhcnRvJ3MgXCJPdXRwdXQgY3JlYXRlZDpcIiBzdGRvdXQgbGluZSwgcGFyc2VkIGJlbG93LlxuICAgICAgY29uc3QgZ3Vlc3NlZFBkZlBhdGggPSB0aGlzLnBkZlBhdGhGb3IoZmlsZSk7XG4gICAgICBjb25zdCBleGlzdGluZ0xlYWYgPSB0aGlzLmFwcC53b3Jrc3BhY2VcbiAgICAgICAgLmdldExlYXZlc09mVHlwZSgncGRmJylcbiAgICAgICAgLmZpbmQoKGwpID0+IHRoaXMubGVhZkZpbGUobCk/LnBhdGggPT09IGd1ZXNzZWRQZGZQYXRoKTtcblxuICAgICAgY29uc3QgYXJncyA9IFsncmVuZGVyJywgZmlsZVBhdGhdO1xuICAgICAgaWYgKHRvRm9ybWF0KSBhcmdzLnB1c2goJy0tdG8nLCB0b0Zvcm1hdCk7XG5cbiAgICAgIGNvbnN0IHF1YXJ0b1Byb2Nlc3MgPSBzcGF3bih0aGlzLnNldHRpbmdzLnF1YXJ0b1BhdGgsIGFyZ3MsIHtcbiAgICAgICAgY3dkOiB3b3JraW5nRGlyLFxuICAgICAgICBlbnY6IGVudlZhcnMsXG4gICAgICB9KTtcblxuICAgICAgbGV0IGRldGVjdGVkT3V0cHV0QmFzZW5hbWU6IHN0cmluZyB8IG51bGwgPSBudWxsO1xuICAgICAgLy8gUXVhcnRvIHByaW50cyB0aGUgaHVtYW4tcmVhZGFibGUgY2F1c2Ugb24gXCJFUlJPUjpcIiBsaW5lcyAoYmFkIFlBTUwsXG4gICAgICAvLyBtaXNzaW5nIGVuZ2luZSwgZXRjLikuIEtlZXAgdGhlbSBzbyBhIGZhaWxpbmcgY2xvc2UgY2FuIHN1cmZhY2UgdGhlXG4gICAgICAvLyByZWFsIHJlYXNvbiBpbiB0aGUgTm90aWNlIGluc3RlYWQgb2YgYSBiYXJlIGV4aXQgY29kZS5cbiAgICAgIGNvbnN0IGVycm9yTGluZXM6IHN0cmluZ1tdID0gW107XG5cbiAgICAgIC8vIFBlci1saW5lIGhhbmRsZXI6IGxvZyB0aGUgbGluZSwgdGhlbiB3YXRjaCBmb3IgXCJPdXRwdXQgY3JlYXRlZDpcIi5cbiAgICAgIGNvbnN0IGhhbmRsZVJlbmRlckxpbmUgPSAobGluZTogc3RyaW5nKSA9PiB7XG4gICAgICAgIGxvZ1F1YXJ0b0xpbmUoJ1F1YXJ0bycsIGxpbmUpO1xuICAgICAgICBjb25zdCBtYXRjaCA9IGxpbmUubWF0Y2goL091dHB1dCBjcmVhdGVkOlxccyooLis/KVxccyokLyk7XG4gICAgICAgIGlmIChtYXRjaCkge1xuICAgICAgICAgIGRldGVjdGVkT3V0cHV0QmFzZW5hbWUgPSBwYXRoLmJhc2VuYW1lKG1hdGNoWzFdLnRyaW0oKSk7XG4gICAgICAgIH1cbiAgICAgICAgaWYgKC9eRVJST1I6Ly50ZXN0KGxpbmUpKSB7XG4gICAgICAgICAgZXJyb3JMaW5lcy5wdXNoKGxpbmUpO1xuICAgICAgICB9XG4gICAgICB9O1xuXG4gICAgICAvLyBPbmUgYnVmZmVyZWQgcHJvY2Vzc29yIHBlciBzdHJlYW0g4oCUIHN0ZG91dCBhbmQgc3RkZXJyIGVhY2hcbiAgICAgIC8vIG5lZWQgdGhlaXIgb3duIHBhcnRpYWwtbGluZSBidWZmZXIsIG9yIGludGVybGVhdmVkIGZyYWdtZW50c1xuICAgICAgLy8gZnJvbSB0aGUgdHdvIHN0cmVhbXMgd291bGQgYmUgc3BsaWNlZCBpbnRvIHN5bnRoZXRpYyBsaW5lcy5cbiAgICAgIGNvbnN0IHJlbmRlclN0ZG91dCA9IG1ha2VMaW5lUHJvY2Vzc29yKGhhbmRsZVJlbmRlckxpbmUpO1xuICAgICAgY29uc3QgcmVuZGVyU3RkZXJyID0gbWFrZUxpbmVQcm9jZXNzb3IoaGFuZGxlUmVuZGVyTGluZSk7XG4gICAgICBxdWFydG9Qcm9jZXNzLnN0ZG91dD8ub24oJ2RhdGEnLCAoZGF0YTogQnVmZmVyKSA9PiByZW5kZXJTdGRvdXQoZGF0YS50b1N0cmluZygpKSk7XG4gICAgICBxdWFydG9Qcm9jZXNzLnN0ZGVycj8ub24oJ2RhdGEnLCAoZGF0YTogQnVmZmVyKSA9PiByZW5kZXJTdGRlcnIoZGF0YS50b1N0cmluZygpKSk7XG5cbiAgICAgIC8vIGNoaWxkX3Byb2Nlc3Muc3Bhd24gZG9lcyBub3QgdGhyb3cgb24gYSBtaXNzaW5nIGJpbmFyeTsgaXQgZW1pdHNcbiAgICAgIC8vIGFuICdlcnJvcicgZXZlbnQgbGF0ZXIuIFdpdGhvdXQgdGhpcyBsaXN0ZW5lciBhbiBFTk9FTlQganVzdFxuICAgICAgLy8gcHJvZHVjZWQgYSBzaWxlbnQgXCJleGl0IDFcIiBjbG9zZSB3aXRoIG5vIG91dHB1dCB0byBjb25zb2xlLlxuICAgICAgcXVhcnRvUHJvY2Vzcy5vbignZXJyb3InLCAoZXJyKSA9PiB7XG4gICAgICAgIGNvbnNvbGUuZXJyb3IoJ1txbWQtYXMtbWRdIEZhaWxlZCB0byBzcGF3biBxdWFydG8gZm9yIHJlbmRlcjonLCBlcnIpO1xuICAgICAgICBuZXcgTm90aWNlKFxuICAgICAgICAgIGBGYWlsZWQgdG8gc3Bhd24gJyR7dGhpcy5zZXR0aW5ncy5xdWFydG9QYXRofSc6ICR7ZXJyLm1lc3NhZ2V9LiBgICtcbiAgICAgICAgICAgICdDaGVjayB0aGUgUXVhcnRvIHBhdGggc2V0dGluZyBhbmQgdGhhdCBRdWFydG8gaXMgb24gUEFUSC4nXG4gICAgICAgICk7XG4gICAgICB9KTtcblxuICAgICAgcXVhcnRvUHJvY2Vzcy5vbignY2xvc2UnLCAoY29kZTogbnVtYmVyIHwgbnVsbCwgc2lnbmFsOiBOb2RlSlMuU2lnbmFscyB8IG51bGwpID0+IHtcbiAgICAgICAgdm9pZCAoYXN5bmMgKCkgPT4ge1xuICAgICAgICAgIHJlbmRlclN0ZG91dC5mbHVzaCgpOyAvLyByZWxlYXNlIGFueSBmaW5hbCBwYXJ0aWFsIGxpbmVcbiAgICAgICAgICByZW5kZXJTdGRlcnIuZmx1c2goKTtcblxuICAgICAgICAgIC8vIEEgY2xlYW4gZXhpdCBpcyBjb2RlIDAuIEFueXRoaW5nIGVsc2UgaXMgYSBmYWlsdXJlLCBleGNlcHQgYVxuICAgICAgICAgIC8vIHRlcm1pbmF0aW9uIGJ5IFNJR1RFUk0vU0lHS0lMTCDigJQgdGhhdCBtZWFucyB0aGUgcHJvY2VzcyB3YXNcbiAgICAgICAgICAvLyBpbnRlbnRpb25hbGx5IGNhbmNlbGxlZCAobWF0Y2hpbmcgdGhlIHByZXZpZXcgaGFuZGxlciwgd2hpY2hcbiAgICAgICAgICAvLyBzdXBwcmVzc2VzIG5vdGljZXMgZm9yIHRob3NlIHNpZ25hbHMpLiBTdGF5IHF1aWV0IHRoZW4uXG4gICAgICAgICAgaWYgKGNvZGUgPT09IDApIHtcbiAgICAgICAgICAgIC8vIGZhbGwgdGhyb3VnaCB0byB0aGUgc3VjY2VzcyBwYXRoIGJlbG93XG4gICAgICAgICAgfSBlbHNlIGlmIChjb2RlID09PSBudWxsICYmIChzaWduYWwgPT09ICdTSUdURVJNJyB8fCBzaWduYWwgPT09ICdTSUdLSUxMJykpIHtcbiAgICAgICAgICAgIGNvbnNvbGUuZXJyb3IoYFtxbWQtYXMtbWRdIFF1YXJ0byByZW5kZXIgY2FuY2VsbGVkICgke3NpZ25hbH0pLmApO1xuICAgICAgICAgICAgcmV0dXJuO1xuICAgICAgICAgIH0gZWxzZSB7XG4gICAgICAgICAgICBjb25zdCBleGl0TGFiZWwgPSBjb2RlICE9PSBudWxsXG4gICAgICAgICAgICAgID8gYGV4aXQgJHtjb2RlfWBcbiAgICAgICAgICAgICAgOiBzaWduYWxcbiAgICAgICAgICAgICAgICA/IGB0ZXJtaW5hdGVkIGJ5ICR7c2lnbmFsfWBcbiAgICAgICAgICAgICAgICA6ICd0ZXJtaW5hdGVkJztcbiAgICAgICAgICAgIC8vIFRoZSBmdWxsIG91dHB1dCB3YXMgYWxyZWFkeSBzdHJlYW1lZCBsaW5lLWJ5LWxpbmUgdGhyb3VnaFxuICAgICAgICAgICAgLy8gY29uc29sZS5sb2cgLyBjb25zb2xlLmVycm9yIGFzIGl0IGFycml2ZWQg4oCUIG5vIG5lZWQgdG9cbiAgICAgICAgICAgIC8vIHJlLWR1bXAgaXQuIFN1cmZhY2UgdGhlIGFjdHVhbCBFUlJPUjogbGluZShzKSBpbiB0aGUgTm90aWNlIHNvXG4gICAgICAgICAgICAvLyB0aGUgdXNlciBzZWVzIHRoZSBjYXVzZSAoYmFkIFlBTUwsIG1pc3NpbmcgZW5naW5lLCAuLi4pIHdpdGhvdXRcbiAgICAgICAgICAgIC8vIGhhdmluZyB0byBvcGVuIHRoZSBkZXZlbG9wZXIgY29uc29sZS5cbiAgICAgICAgICAgIGNvbnNvbGUuZXJyb3IoYFtxbWQtYXMtbWRdIFF1YXJ0byByZW5kZXIgZmFpbGVkICgke2V4aXRMYWJlbH0pLmApO1xuICAgICAgICAgICAgY29uc3QgcmVhc29uID0gZXJyb3JMaW5lcy5sZW5ndGggPiAwXG4gICAgICAgICAgICAgID8gZXJyb3JMaW5lcy5qb2luKCdcXG4nKVxuICAgICAgICAgICAgICA6ICdDaGVjayB0aGUgZGV2ZWxvcGVyIGNvbnNvbGUgZm9yIGRldGFpbHMuJztcbiAgICAgICAgICAgIG5ldyBOb3RpY2UoYFF1YXJ0byByZW5kZXIgZmFpbGVkICgke2V4aXRMYWJlbH0pLlxcbiR7cmVhc29ufWAsIDE1MDAwKTtcbiAgICAgICAgICAgIHJldHVybjtcbiAgICAgICAgICB9XG5cbiAgICAgICAgICBjb25zdCBzb3VyY2VEaXIgPSBmaWxlLnBhcmVudD8ucGF0aCA/PyAnJztcbiAgICAgICAgICBjb25zdCBvdXRwdXRWYXVsdFBhdGggPSBub3JtYWxpemVQYXRoKFxuICAgICAgICAgICAgZGV0ZWN0ZWRPdXRwdXRCYXNlbmFtZVxuICAgICAgICAgICAgICA/IChzb3VyY2VEaXIgPyBgJHtzb3VyY2VEaXJ9LyR7ZGV0ZWN0ZWRPdXRwdXRCYXNlbmFtZX1gIDogZGV0ZWN0ZWRPdXRwdXRCYXNlbmFtZSlcbiAgICAgICAgICAgICAgOiBndWVzc2VkUGRmUGF0aFxuICAgICAgICAgICk7XG5cbiAgICAgICAgICBjb25zdCBvdXRwdXRURmlsZSA9IGF3YWl0IHRoaXMud2FpdEZvclZhdWx0RmlsZShvdXRwdXRWYXVsdFBhdGgpO1xuXG4gICAgICAgICAgaWYgKCFvdXRwdXRURmlsZSkge1xuICAgICAgICAgICAgbmV3IE5vdGljZShcbiAgICAgICAgICAgICAgYFF1YXJ0byByZW5kZXJlZCwgYnV0ICR7b3V0cHV0VmF1bHRQYXRofSBkaWQgbm90IGFwcGVhciBpbiB0aGUgdmF1bHQgd2l0aGluIHRoZSB0aW1lb3V0LiBDaGVjayBRdWFydG8ncyBvdXRwdXQtZGlyIG9yIHZhdWx0IHN5bmMuYFxuICAgICAgICAgICAgKTtcbiAgICAgICAgICAgIHJldHVybjtcbiAgICAgICAgICB9XG5cbiAgICAgICAgICBjb25zdCBpc1BkZiA9IG91dHB1dFZhdWx0UGF0aC50b0xvd2VyQ2FzZSgpLmVuZHNXaXRoKCcucGRmJyk7XG5cbiAgICAgICAgICBpZiAoIXRoaXMuc2V0dGluZ3Mub3BlblBkZkluT2JzaWRpYW4gfHwgIWlzUGRmKSB7XG4gICAgICAgICAgICBuZXcgTm90aWNlKFxuICAgICAgICAgICAgICBpc1BkZlxuICAgICAgICAgICAgICAgID8gYFBERiByZW5kZXJlZDogJHtvdXRwdXRWYXVsdFBhdGh9YFxuICAgICAgICAgICAgICAgIDogYFJlbmRlcmVkOiAke291dHB1dFZhdWx0UGF0aH0gKE9ic2lkaWFuJ3MgYnVpbHQtaW4gdmlld2VyIG9ubHkgaGFuZGxlcyBQREZzKS5gXG4gICAgICAgICAgICApO1xuICAgICAgICAgICAgcmV0dXJuO1xuICAgICAgICAgIH1cblxuICAgICAgICAgIHRyeSB7XG4gICAgICAgICAgICBjb25zdCBsZWFmID0gZXhpc3RpbmdMZWFmPy5wYXJlbnQgIT0gbnVsbFxuICAgICAgICAgICAgICA/IGV4aXN0aW5nTGVhZlxuICAgICAgICAgICAgICA6IHRoaXMuYXBwLndvcmtzcGFjZS5nZXRMZWFmKCdzcGxpdCcsICd2ZXJ0aWNhbCcpO1xuICAgICAgICAgICAgYXdhaXQgbGVhZi5vcGVuRmlsZShvdXRwdXRURmlsZSwgeyBhY3RpdmU6IGZhbHNlIH0pO1xuICAgICAgICAgICAgYXdhaXQgdGhpcy5hcHAud29ya3NwYWNlLnJldmVhbExlYWYobGVhZik7XG4gICAgICAgICAgICBuZXcgTm90aWNlKGBPcGVuZWQgJHtvdXRwdXRWYXVsdFBhdGh9YCk7XG4gICAgICAgICAgfSBjYXRjaCAoZXJyKSB7XG4gICAgICAgICAgICBjb25zb2xlLmVycm9yKCdGYWlsZWQgdG8gb3BlbiBQREYgaW4gT2JzaWRpYW46JywgZXJyKTtcbiAgICAgICAgICAgIG5ldyBOb3RpY2UoXG4gICAgICAgICAgICAgIGBQREYgcmVuZGVyZWQgYXQgJHtvdXRwdXRWYXVsdFBhdGh9LCBidXQgT2JzaWRpYW4gY291bGQgbm90IG9wZW4gaXQgKG5vIFBERiB2aWV3ZXIgcmVnaXN0ZXJlZD8pLmBcbiAgICAgICAgICAgICk7XG4gICAgICAgICAgfVxuICAgICAgICB9KSgpLmNhdGNoKChlcnIpID0+IHtcbiAgICAgICAgICBjb25zb2xlLmVycm9yKCdbcW1kLWFzLW1kXSBRdWFydG8gcmVuZGVyIGNsb3NlIGhhbmRsZXIgZmFpbGVkOicsIGVycik7XG4gICAgICAgIH0pO1xuICAgICAgfSk7XG4gICAgfSBjYXRjaCAoZXJyb3IpIHtcbiAgICAgIGNvbnNvbGUuZXJyb3IoJ0ZhaWxlZCB0byByZW5kZXIgUXVhcnRvIFBERjonLCBlcnJvcik7XG4gICAgICBuZXcgTm90aWNlKCdGYWlsZWQgdG8gcmVuZGVyIFF1YXJ0byBQREYnKTtcbiAgICB9XG4gIH1cblxuICBhc3luYyB3YWl0Rm9yVmF1bHRGaWxlKHZhdWx0UGF0aDogc3RyaW5nLCB0aW1lb3V0TXMgPSA1MDAwKTogUHJvbWlzZTxURmlsZSB8IG51bGw+IHtcbiAgICBjb25zdCBzdGFydCA9IERhdGUubm93KCk7XG4gICAgd2hpbGUgKERhdGUubm93KCkgLSBzdGFydCA8IHRpbWVvdXRNcykge1xuICAgICAgY29uc3QgZiA9IHRoaXMuYXBwLnZhdWx0LmdldEFic3RyYWN0RmlsZUJ5UGF0aCh2YXVsdFBhdGgpO1xuICAgICAgaWYgKGYgaW5zdGFuY2VvZiBURmlsZSkgcmV0dXJuIGY7XG4gICAgICBhd2FpdCBuZXcgUHJvbWlzZSgocikgPT4gd2luZG93LnNldFRpbWVvdXQociwgMjAwKSk7XG4gICAgfVxuICAgIHJldHVybiBudWxsO1xuICB9XG59XG5cbmNsYXNzIFFtZFNldHRpbmdUYWIgZXh0ZW5kcyBQbHVnaW5TZXR0aW5nVGFiIHtcbiAgcGx1Z2luOiBRbWRBc01kUGx1Z2luO1xuXG4gIGNvbnN0cnVjdG9yKGFwcDogQXBwLCBwbHVnaW46IFFtZEFzTWRQbHVnaW4pIHtcbiAgICBzdXBlcihhcHAsIHBsdWdpbik7XG4gICAgdGhpcy5wbHVnaW4gPSBwbHVnaW47XG4gIH1cblxuICBkaXNwbGF5KCk6IHZvaWQge1xuICAgIGNvbnN0IHsgY29udGFpbmVyRWwgfSA9IHRoaXM7XG4gICAgY29udGFpbmVyRWwuZW1wdHkoKTtcblxuICAgIG5ldyBTZXR0aW5nKGNvbnRhaW5lckVsKVxuICAgICAgLnNldE5hbWUoJ1F1YXJ0byBwYXRoJylcbiAgICAgIC5zZXREZXNjKCdQYXRoIHRvIHRoZSBRdWFydG8gZXhlY3V0YWJsZSAoZS5nLiBxdWFydG8sIC91c3IvbG9jYWwvYmluL3F1YXJ0byknKVxuICAgICAgLmFkZFRleHQoKHRleHQpID0+XG4gICAgICAgIHRleHRcbiAgICAgICAgICAuc2V0UGxhY2Vob2xkZXIoJ3F1YXJ0bycpXG4gICAgICAgICAgLnNldFZhbHVlKHRoaXMucGx1Z2luLnNldHRpbmdzLnF1YXJ0b1BhdGgpXG4gICAgICAgICAgLm9uQ2hhbmdlKGFzeW5jICh2YWx1ZSkgPT4ge1xuICAgICAgICAgICAgdGhpcy5wbHVnaW4uc2V0dGluZ3MucXVhcnRvUGF0aCA9IHZhbHVlO1xuICAgICAgICAgICAgYXdhaXQgdGhpcy5wbHVnaW4uc2F2ZVNldHRpbmdzKCk7XG4gICAgICAgICAgfSlcbiAgICAgICk7XG5cbiAgICBuZXcgU2V0dGluZyhjb250YWluZXJFbClcbiAgICAgIC5zZXROYW1lKCdUZW1wbGF0ZXMgZm9sZGVyJylcbiAgICAgIC5zZXREZXNjKFxuICAgICAgICAnVmF1bHQgZm9sZGVyIGNvbnRhaW5pbmcgeW91ciBvd24gLnFtZCB0ZW1wbGF0ZSBmaWxlcyBmb3IgXCJOZXcgUXVhcnRvIGZpbGUgZnJvbSBwcmVzZXRcIi4gJyArXG4gICAgICAgICAgJ0VhY2ggLnFtZCBpbiB0aGlzIGZvbGRlciAodG9wIGxldmVsIG9ubHkg4oCUIHN1YmZvbGRlcnMgYXJlIGlnbm9yZWQpIGJlY29tZXMgYSBwcmVzZXQ7ICcgK1xuICAgICAgICAgICd0aGUgZmlsZSBuYW1lIGlzIHVzZWQgYXMgdGhlIHByZXNldCBuYW1lIGFuZCB0aGUgZmlsZSBjb250ZW50cyBhcmUgaW5zZXJ0ZWQgdmVyYmF0aW0uICcgK1xuICAgICAgICAgICdMZWF2ZSBlbXB0eSB0byBzaG93IG9ubHkgdGhlIGJ1aWx0LWluIHByZXNldHMuJ1xuICAgICAgKVxuICAgICAgLmFkZFRleHQoKHRleHQpID0+XG4gICAgICAgIHRleHRcbiAgICAgICAgICAuc2V0UGxhY2Vob2xkZXIoJ2UuZy4gX3F1YXJ0by10ZW1wbGF0ZXMnKVxuICAgICAgICAgIC5zZXRWYWx1ZSh0aGlzLnBsdWdpbi5zZXR0aW5ncy50ZW1wbGF0ZXNGb2xkZXIpXG4gICAgICAgICAgLm9uQ2hhbmdlKGFzeW5jICh2YWx1ZSkgPT4ge1xuICAgICAgICAgICAgdGhpcy5wbHVnaW4uc2V0dGluZ3MudGVtcGxhdGVzRm9sZGVyID0gdmFsdWU7XG4gICAgICAgICAgICBhd2FpdCB0aGlzLnBsdWdpbi5zYXZlU2V0dGluZ3MoKTtcbiAgICAgICAgICB9KVxuICAgICAgKTtcblxuICAgIG5ldyBTZXR0aW5nKGNvbnRhaW5lckVsKVxuICAgICAgLnNldE5hbWUoJ0VuYWJsZSBlZGl0aW5nIFF1YXJ0byBmaWxlcycpXG4gICAgICAuc2V0RGVzYyhcbiAgICAgICAgJ1doZW4gb24sIC5xbWQgZmlsZXMgb3BlbiBpbiB0aGUgTWFya2Rvd24gZWRpdG9yLiBUdXJuIG9mZiBpZiBhbm90aGVyIHBsdWdpbiBoYW5kbGVzIC5xbWQgZWRpdGluZy4nXG4gICAgICApXG4gICAgICAuYWRkVG9nZ2xlKCh0b2dnbGUpID0+XG4gICAgICAgIHRvZ2dsZVxuICAgICAgICAgIC5zZXRWYWx1ZSh0aGlzLnBsdWdpbi5zZXR0aW5ncy5lbmFibGVRbWRMaW5raW5nKVxuICAgICAgICAgIC5vbkNoYW5nZShhc3luYyAodmFsdWUpID0+IHtcbiAgICAgICAgICAgIHRoaXMucGx1Z2luLnNldHRpbmdzLmVuYWJsZVFtZExpbmtpbmcgPSB2YWx1ZTtcbiAgICAgICAgICAgIGlmICh2YWx1ZSkge1xuICAgICAgICAgICAgICB0aGlzLnBsdWdpbi5yZWdpc3RlclFtZEV4dGVuc2lvbigpO1xuICAgICAgICAgICAgfVxuICAgICAgICAgICAgYXdhaXQgdGhpcy5wbHVnaW4uc2F2ZVNldHRpbmdzKCk7XG4gICAgICAgICAgfSlcbiAgICAgICk7XG5cbiAgICBuZXcgU2V0dGluZyhjb250YWluZXJFbClcbiAgICAgIC5zZXROYW1lKCdRVUFSVE9fVFlQU1QgdmFyaWFibGUnKVxuICAgICAgLnNldERlc2MoJ1ZhbHVlIGZvciB0aGUgUVVBUlRPX1RZUFNUIGVudmlyb25tZW50IHZhcmlhYmxlIChsZWF2ZSBlbXB0eSB0byB1bnNldCkuJylcbiAgICAgIC5hZGRUZXh0KCh0ZXh0KSA9PlxuICAgICAgICB0ZXh0XG4gICAgICAgICAgLnNldFBsYWNlaG9sZGVyKCdlLmcuIHR5cHN0X3BhdGgnKVxuICAgICAgICAgIC5zZXRWYWx1ZSh0aGlzLnBsdWdpbi5zZXR0aW5ncy5xdWFydG9UeXBzdClcbiAgICAgICAgICAub25DaGFuZ2UoYXN5bmMgKHZhbHVlKSA9PiB7XG4gICAgICAgICAgICB0aGlzLnBsdWdpbi5zZXR0aW5ncy5xdWFydG9UeXBzdCA9IHZhbHVlO1xuICAgICAgICAgICAgYXdhaXQgdGhpcy5wbHVnaW4uc2F2ZVNldHRpbmdzKCk7XG4gICAgICAgICAgfSlcbiAgICAgICk7XG5cbiAgICBuZXcgU2V0dGluZyhjb250YWluZXJFbClcbiAgICAgIC5zZXROYW1lKCdPcGVuIGNvbXBpbGVkIFBERiBpbiBPYnNpZGlhbicpXG4gICAgICAuc2V0RGVzYyhcbiAgICAgICAgXCJXaGVuIHJlbmRlcmluZyB0byBQREYsIG9wZW4gdGhlIHJlc3VsdGluZyBmaWxlIGluc2lkZSBPYnNpZGlhbiB1c2luZyB0aGUgYnVpbHQtaW4gUERGIHZpZXdlci4gVGhlIC5xbWQgc291cmNlIG11c3QgbGl2ZSBpbiB0aGUgdmF1bHQgc28gdGhlIHJlbmRlcmVkIFBERiBpcyBhY2Nlc3NpYmxlLlwiXG4gICAgICApXG4gICAgICAuYWRkVG9nZ2xlKCh0b2dnbGUpID0+XG4gICAgICAgIHRvZ2dsZVxuICAgICAgICAgIC5zZXRWYWx1ZSh0aGlzLnBsdWdpbi5zZXR0aW5ncy5vcGVuUGRmSW5PYnNpZGlhbilcbiAgICAgICAgICAub25DaGFuZ2UoYXN5bmMgKHZhbHVlKSA9PiB7XG4gICAgICAgICAgICB0aGlzLnBsdWdpbi5zZXR0aW5ncy5vcGVuUGRmSW5PYnNpZGlhbiA9IHZhbHVlO1xuICAgICAgICAgICAgYXdhaXQgdGhpcy5wbHVnaW4uc2F2ZVNldHRpbmdzKCk7XG4gICAgICAgICAgfSlcbiAgICAgICk7XG5cbiAgICBuZXcgU2V0dGluZyhjb250YWluZXJFbClcbiAgICAgIC5zZXROYW1lKCdPcGVuIFF1YXJ0byBwcmV2aWV3IGluIE9ic2lkaWFuJylcbiAgICAgIC5zZXREZXNjKFxuICAgICAgICAnRGVmYXVsdCB0YXJnZXQgZm9yIHRoZSBnZW5lcmljIFRvZ2dsZSBRdWFydG8gcHJldmlldyBjb21tYW5kIGFuZCByaWJib24gaWNvbi4gJyArXG4gICAgICAgICAgXCJXaGVuIG9uOiBQREYgcHJldmlld3MgKGZvcm1hdDogdHlwc3QgLyBwZGYpIG9wZW4gaW4gT2JzaWRpYW4ncyBuYXRpdmUgUERGIHZpZXdlcjsgXCIgK1xuICAgICAgICAgIFwibm9uLVBERiBwcmV2aWV3cyAoSFRNTCwgZXRjLikgb3BlbiBpbiBPYnNpZGlhbiAxLjgncyBidWlsdC1pbiB3ZWIgdmlld2VyLiBcIiArXG4gICAgICAgICAgJ1doZW4gb2ZmLCB0aGUgcGx1Z2luIG9wZW5zIFF1YXJ0b1xcJ3MgcHJldmlldyBVUkwgaW4geW91ciBkZWZhdWx0IGV4dGVybmFsIGJyb3dzZXIuICcgK1xuICAgICAgICAgICdVc2UgdGhlIGV4cGxpY2l0IHByZXZpZXcgY29tbWFuZHMgdG8gY2hvb3NlIGEgdGFyZ2V0IHdpdGhvdXQgY2hhbmdpbmcgdGhpcyBzZXR0aW5nLidcbiAgICAgIClcbiAgICAgIC5hZGRUb2dnbGUoKHRvZ2dsZSkgPT5cbiAgICAgICAgdG9nZ2xlXG4gICAgICAgICAgLnNldFZhbHVlKHRoaXMucGx1Z2luLnNldHRpbmdzLnByZXZpZXdJbk9ic2lkaWFuKVxuICAgICAgICAgIC5vbkNoYW5nZShhc3luYyAodmFsdWUpID0+IHtcbiAgICAgICAgICAgIHRoaXMucGx1Z2luLnNldHRpbmdzLnByZXZpZXdJbk9ic2lkaWFuID0gdmFsdWU7XG4gICAgICAgICAgICBhd2FpdCB0aGlzLnBsdWdpbi5zYXZlU2V0dGluZ3MoKTtcbiAgICAgICAgICB9KVxuICAgICAgKTtcblxuICAgIG5ldyBTZXR0aW5nKGNvbnRhaW5lckVsKVxuICAgICAgLnNldE5hbWUoJ1ByZXZpZXcgYW5kIHJlbmRlciBNYXJrZG93biBmaWxlcyB3aXRoIFF1YXJ0bycpXG4gICAgICAuc2V0RGVzYyhcbiAgICAgICAgJ1doZW4gb24sIFF1YXJ0byBwcmV2aWV3IGFuZCByZW5kZXIgY29tbWFuZHMgYWxzbyBhY2NlcHQgLm1kIGZpbGVzIHRoYXQgaGF2ZSBfcXVhcnRvLnltbCBpbiB0aGVpciBmb2xkZXIgb3IgYW4gYW5jZXN0b3IgdXAgdG8gdGhlIHZhdWx0IHJvb3QuIExlYXZlIG9mZiB0byByZXN0cmljdCBjb21tYW5kcyB0byAucW1kIGZpbGVzLidcbiAgICAgIClcbiAgICAgIC5hZGRUb2dnbGUoKHRvZ2dsZSkgPT5cbiAgICAgICAgdG9nZ2xlXG4gICAgICAgICAgLnNldFZhbHVlKHRoaXMucGx1Z2luLnNldHRpbmdzLnByZXZpZXdNYXJrZG93bkZpbGVzKVxuICAgICAgICAgIC5vbkNoYW5nZShhc3luYyAodmFsdWUpID0+IHtcbiAgICAgICAgICAgIHRoaXMucGx1Z2luLnNldHRpbmdzLnByZXZpZXdNYXJrZG93bkZpbGVzID0gdmFsdWU7XG4gICAgICAgICAgICBhd2FpdCB0aGlzLnBsdWdpbi5zYXZlU2V0dGluZ3MoKTtcbiAgICAgICAgICB9KVxuICAgICAgKTtcblxuICAgIG5ldyBTZXR0aW5nKGNvbnRhaW5lckVsKVxuICAgICAgLnNldE5hbWUoJ1Nob3cgWUFNTCBmaWxlcycpXG4gICAgICAuc2V0RGVzYyhcbiAgICAgICAgJ1doZW4gb24sIC55bWwgYW5kIC55YW1sIGZpbGVzIGFwcGVhciBpbiBPYnNpZGlhbiB1c2luZyBhIENvZGVNaXJyb3IgZWRpdG9yIHdpdGggUXVhcnRvLW9yaWVudGVkIFlBTUwgaGlnaGxpZ2h0aW5nLiBUdXJuIG9mZiBhbmQgcmVsb2FkIHRoZSBwbHVnaW4gdG8gaGlkZSB0aGVtIGFnYWluLidcbiAgICAgIClcbiAgICAgIC5hZGRUb2dnbGUoKHRvZ2dsZSkgPT5cbiAgICAgICAgdG9nZ2xlXG4gICAgICAgICAgLnNldFZhbHVlKHRoaXMucGx1Z2luLnNldHRpbmdzLnNob3dZYW1sRmlsZXMpXG4gICAgICAgICAgLm9uQ2hhbmdlKGFzeW5jICh2YWx1ZSkgPT4ge1xuICAgICAgICAgICAgdGhpcy5wbHVnaW4uc2V0dGluZ3Muc2hvd1lhbWxGaWxlcyA9IHZhbHVlO1xuICAgICAgICAgICAgaWYgKHZhbHVlKSB7XG4gICAgICAgICAgICAgIHRoaXMucGx1Z2luLnJlZ2lzdGVyWWFtbEV4dGVuc2lvbnMoKTtcbiAgICAgICAgICAgIH0gZWxzZSB7XG4gICAgICAgICAgICAgIG5ldyBOb3RpY2UoJ1JlbG9hZCB0aGUgcGx1Z2luIHRvIGhpZGUgWUFNTCBmaWxlcyBhZ2Fpbi4nKTtcbiAgICAgICAgICAgIH1cbiAgICAgICAgICAgIGF3YWl0IHRoaXMucGx1Z2luLnNhdmVTZXR0aW5ncygpO1xuICAgICAgICAgIH0pXG4gICAgICApO1xuXG4gICAgbmV3IFNldHRpbmcoY29udGFpbmVyRWwpXG4gICAgICAuc2V0TmFtZSgnU2hvdyBMdWEgZmlsZXMnKVxuICAgICAgLnNldERlc2MoXG4gICAgICAgICdXaGVuIG9uLCAubHVhIGZpbGVzIGFwcGVhciBpbiBPYnNpZGlhbiB1c2luZyBhIENvZGVNaXJyb3IgZWRpdG9yIHdpdGggbWluaW1hbCBMdWEgc3ludGF4IGhpZ2hsaWdodGluZyDigJQgaGFuZHkgZm9yIGVkaXRpbmcgUXVhcnRvL3BhbmRvYyBmaWx0ZXIgc2NyaXB0cy4gVHVybiBvZmYgYW5kIHJlbG9hZCB0aGUgcGx1Z2luIHRvIGhpZGUgdGhlbSBhZ2Fpbi4nXG4gICAgICApXG4gICAgICAuYWRkVG9nZ2xlKCh0b2dnbGUpID0+XG4gICAgICAgIHRvZ2dsZVxuICAgICAgICAgIC5zZXRWYWx1ZSh0aGlzLnBsdWdpbi5zZXR0aW5ncy5zaG93THVhRmlsZXMpXG4gICAgICAgICAgLm9uQ2hhbmdlKGFzeW5jICh2YWx1ZSkgPT4ge1xuICAgICAgICAgICAgdGhpcy5wbHVnaW4uc2V0dGluZ3Muc2hvd0x1YUZpbGVzID0gdmFsdWU7XG4gICAgICAgICAgICBpZiAodmFsdWUpIHtcbiAgICAgICAgICAgICAgdGhpcy5wbHVnaW4ucmVnaXN0ZXJMdWFFeHRlbnNpb25zKCk7XG4gICAgICAgICAgICB9IGVsc2Uge1xuICAgICAgICAgICAgICBuZXcgTm90aWNlKCdSZWxvYWQgdGhlIHBsdWdpbiB0byBoaWRlIEx1YSBmaWxlcyBhZ2Fpbi4nKTtcbiAgICAgICAgICAgIH1cbiAgICAgICAgICAgIGF3YWl0IHRoaXMucGx1Z2luLnNhdmVTZXR0aW5ncygpO1xuICAgICAgICAgIH0pXG4gICAgICApO1xuXG4gICAgbmV3IFNldHRpbmcoY29udGFpbmVyRWwpXG4gICAgICAuc2V0TmFtZSgnU2hvdyBRdWFydG8gb3V0bGluZScpXG4gICAgICAuc2V0RGVzYyhcbiAgICAgICAgXCJBZGQgYSBzaWRlYmFyIG91dGxpbmUgb2YgdGhlIGFjdGl2ZSAucW1kIGZpbGUncyBoZWFkaW5ncyAoT2JzaWRpYW4ncyBcIiArXG4gICAgICAgICAgJ2NvcmUgT3V0bGluZSBwYW5lbCBjYW5ub3QgcmVhZCAucW1kIGZpbGVzKS4gQWN0aXZlIGZpbGUgb25seSDigJQgJyArXG4gICAgICAgICAgJ2hlYWRpbmdzIGZyb20gaW5jbHVkZWQgZmlsZXMgYXJlIG5vdCBsaXN0ZWQuIFRoZSBcIk9wZW4gUXVhcnRvICcgK1xuICAgICAgICAgICdvdXRsaW5lXCIgY29tbWFuZCB3b3JrcyByZWdhcmRsZXNzIG9mIHRoaXMgdG9nZ2xlLidcbiAgICAgIClcbiAgICAgIC5hZGRUb2dnbGUoKHRvZ2dsZSkgPT5cbiAgICAgICAgdG9nZ2xlXG4gICAgICAgICAgLnNldFZhbHVlKHRoaXMucGx1Z2luLnNldHRpbmdzLnNob3dPdXRsaW5lKVxuICAgICAgICAgIC5vbkNoYW5nZShhc3luYyAodmFsdWUpID0+IHtcbiAgICAgICAgICAgIHRoaXMucGx1Z2luLnNldHRpbmdzLnNob3dPdXRsaW5lID0gdmFsdWU7XG4gICAgICAgICAgICBhd2FpdCB0aGlzLnBsdWdpbi5zYXZlU2V0dGluZ3MoKTtcbiAgICAgICAgICAgIGlmICh2YWx1ZSkge1xuICAgICAgICAgICAgICBhd2FpdCB0aGlzLnBsdWdpbi5hY3RpdmF0ZU91dGxpbmVWaWV3KCk7XG4gICAgICAgICAgICB9IGVsc2Uge1xuICAgICAgICAgICAgICB0aGlzLnBsdWdpbi5kZXRhY2hPdXRsaW5lVmlld3MoKTtcbiAgICAgICAgICAgIH1cbiAgICAgICAgICB9KVxuICAgICAgKTtcblxuICAgIG5ldyBTZXR0aW5nKGNvbnRhaW5lckVsKVxuICAgICAgLnNldE5hbWUoJ091dGxpbmUgTWFya2Rvd24gZmlsZXMnKVxuICAgICAgLnNldERlc2MoXG4gICAgICAgICdXaGVuIG9uLCB0aGUgUXVhcnRvIG91dGxpbmUgYWxzbyBsaXN0cyBoZWFkaW5ncyBvZiB0aGUgYWN0aXZlIC5tZCBmaWxlLCBub3QganVzdCAucW1kIGZpbGVzLiAnICtcbiAgICAgICAgICBcIk9ic2lkaWFuJ3MgY29yZSBPdXRsaW5lIHBhbmVsIGFscmVhZHkgY292ZXJzIC5tZCBmaWxlcyDigJQgZW5hYmxlIHRoaXMgb25seSBpZiB5b3UgcHJlZmVyIHRoZSBRdWFydG8gb3V0bGluZSBmb3IgYm90aC5cIlxuICAgICAgKVxuICAgICAgLmFkZFRvZ2dsZSgodG9nZ2xlKSA9PlxuICAgICAgICB0b2dnbGVcbiAgICAgICAgICAuc2V0VmFsdWUodGhpcy5wbHVnaW4uc2V0dGluZ3Mub3V0bGluZU1hcmtkb3duRmlsZXMpXG4gICAgICAgICAgLm9uQ2hhbmdlKGFzeW5jICh2YWx1ZSkgPT4ge1xuICAgICAgICAgICAgdGhpcy5wbHVnaW4uc2V0dGluZ3Mub3V0bGluZU1hcmtkb3duRmlsZXMgPSB2YWx1ZTtcbiAgICAgICAgICAgIGF3YWl0IHRoaXMucGx1Z2luLnNhdmVTZXR0aW5ncygpO1xuICAgICAgICAgICAgLy8gRHJvcCBhIG5vdy1pbmVsaWdpYmxlIC5tZCB0YXJnZXQgc28gdGhlIG91dGxpbmUgZG9lcyBub3Qga2VlcFxuICAgICAgICAgICAgLy8gZGVzY3JpYmluZyBhIGZpbGUgaXQgaXMgbm8gbG9uZ2VyIGFsbG93ZWQgdG8uXG4gICAgICAgICAgICBpZiAoXG4gICAgICAgICAgICAgICF2YWx1ZSAmJlxuICAgICAgICAgICAgICB0aGlzLnBsdWdpbi5sYXN0QWN0aXZlUXVhcnRvRmlsZSAmJlxuICAgICAgICAgICAgICB0aGlzLnBsdWdpbi5pc01hcmtkb3duRmlsZSh0aGlzLnBsdWdpbi5sYXN0QWN0aXZlUXVhcnRvRmlsZSlcbiAgICAgICAgICAgICkge1xuICAgICAgICAgICAgICB0aGlzLnBsdWdpbi5sYXN0QWN0aXZlUXVhcnRvRmlsZSA9IG51bGw7XG4gICAgICAgICAgICB9XG4gICAgICAgICAgICB0aGlzLnBsdWdpbi50cmFja0FjdGl2ZVF1YXJ0b0ZpbGUoKTtcbiAgICAgICAgICAgIHRoaXMucGx1Z2luLnJlZnJlc2hPdXRsaW5lVmlld3MoKTtcbiAgICAgICAgICB9KVxuICAgICAgKTtcbiAgfVxufVxuIl0sIm5hbWVzIjpbIkl0ZW1WaWV3IiwiTWFya2Rvd25WaWV3Iiwic2V0SWNvbiIsIlN0YXRlRmllbGQiLCJFZGl0b3JWaWV3IiwiUmFuZ2VTZXRCdWlsZGVyIiwiRGVjb3JhdGlvbiIsIlRleHRGaWxlVmlldyIsIkVkaXRvclN0YXRlIiwia2V5bWFwIiwiaW5kZW50TW9yZSIsImluZGVudExlc3MiLCJTdWdnZXN0TW9kYWwiLCJNb2RhbCIsIlNldHRpbmciLCJOb3RpY2UiLCJub3JtYWxpemVQYXRoIiwiVEZvbGRlciIsIlRGaWxlIiwiUGx1Z2luIiwiZGVib3VuY2UiLCJGaWxlU3lzdGVtQWRhcHRlciIsIkZpbGVWaWV3Iiwic2hlbGwiLCJwYXRoIiwic3Bhd24iLCJQbHVnaW5TZXR0aW5nVGFiIl0sIm1hcHBpbmdzIjoiOzs7Ozs7Ozs7Ozs7Ozs7Ozs7Ozs7Ozs7Ozs7OztBQUdBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFFTyxNQUFNLGdCQUFnQixHQUFHLGtCQUFrQixDQUFDO0FBWW5EO0FBQ0E7QUFDQTtBQUNBO0FBQ0EsU0FBUyxnQkFBZ0IsQ0FBQyxRQUFzQixFQUFBO0lBQzlDLE1BQU0sS0FBSyxHQUFxQixFQUFFLENBQUM7SUFDbkMsTUFBTSxLQUFLLEdBQXFCLEVBQUUsQ0FBQztBQUNuQyxJQUFBLEtBQUssTUFBTSxDQUFDLElBQUksUUFBUSxFQUFFO1FBQ3hCLE1BQU0sSUFBSSxHQUFtQixFQUFFLEdBQUcsQ0FBQyxFQUFFLFFBQVEsRUFBRSxFQUFFLEVBQUUsQ0FBQztBQUNwRCxRQUFBLE9BQU8sS0FBSyxDQUFDLE1BQU0sSUFBSSxLQUFLLENBQUMsS0FBSyxDQUFDLE1BQU0sR0FBRyxDQUFDLENBQUMsQ0FBQyxLQUFLLElBQUksQ0FBQyxDQUFDLEtBQUs7WUFBRSxLQUFLLENBQUMsR0FBRyxF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
