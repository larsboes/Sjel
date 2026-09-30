# Gotchas

Traps that cost a render cycle each. All of them are handled inside `deckkit`, so
the reason to read this is to know *why* the library has the shape it has, and to
recognise the failure if you bypass it.

The theme of the list: **python-pptx's defaults are PowerPoint's defaults, and
PowerPoint's defaults are not a design.** Every item below is a default that has to
be actively undone.

## Rendering

**There is no PowerPoint or Keynote on the build machine.** LibreOffice is the only
renderer, invoked as `soffice --headless --convert-to pdf`, and it is the reference
for what "looks right" means here. If LibreOffice is missing, `render` fails with the
install hint rather than a traceback, but it cannot fall back, because a deck that
is never rendered is never verified.

**Two renderers disagree about relative-path figures and about newlines.** See the
newline item below. Anything the two treat differently is a defect waiting for the
day the deck is opened on the presenting machine.

**Rasterise the PDF, do not screenshot the app.** `pymupdf` at 110 dpi gives a
3840×2160-ish PNG per slide, which is enough to read 9.5pt text and to see a
half-point misalignment.

## text and runs

**A newline inside `run.text` writes a literal newline inside `<a:t>`.** LibreOffice
happens to render it as a line break; PowerPoint treats it as whitespace and collapses
it. The portable form is `paragraph.add_line_break()`, which `markup.write_runs` emits.
Symptom in the wild: a two-line title that looks fine locally and runs off the edge
on the presentation laptop.

**`run.text` does not inherit the paragraph's font.** A run with no explicit
`font.name` falls back to the renderer's default, which is Calibri in PowerPoint, so
one word in a heading silently changes typeface. Every run this library creates sets
name, size and colour.

**Unpaired `*` or `~` prints literally.** The markup is two tokens, and neither is
escaped. `check` catches it in the source, which is cheaper than catching it on
slide 24.

## Shapes

**Every autoshape carries a `<p:style>` with `effectRef idx="2"`, a theme drop
shadow.** `shape.shadow.inherit = False` inserts an empty `<a:effectLst/>` in `spPr`,
and LibreOffice draws the shadow anyway. The fix is to remove the entire `<p:style>`
element, which is what `layout._flat()` does. Symptom: a deck that looks like a 2010
corporate template no matter how good the palette is.

**Autoshape text defaults to centred.** The shape XML carries `<a:pPr algn="ctr"/>`
and `<a:bodyPr anchor="ctr"/>`, so any paragraph without an explicit alignment is
silently centred. Invisible in source, obvious on screen, and it makes a panel with
one long bullet look like a poster. `layout.paragraph()` always sets alignment.

**Vertical padding must not scale with horizontal padding.** A statement bar is
0.88in tall. `pad=0.30` applied to all four margins leaves 0.20in of usable height and
a second line is lost. `layout.panel()` caps vertical inset at 0.12in for this reason.

**The rounded-rectangle badge must not wrap and must size to its text.** A fixed-width
badge wraps a long name to two lines, which spills out of the rounded box and reads as
a rendering fault. `layout.badge()` computes the width and disables wrapping.

**Table cells are rectangles, not a table object.** python-pptx's table carries
PowerPoint's theme styling, which fights the deck palette and cannot be re-coloured
without XML surgery. Composing rectangles costs twenty lines and gets the deck's own
type and rules.

## Geometry

**The body floor and the footer are different numbers.** 6.68 and 6.83. The 0.15in
between them is not slack, it is what stops a block from touching the badge.
Lowering the floor to fit something is always the wrong fix.

**Never hard-code `y = 1.7` for content start.** A two-line title pushes the rule down
by 0.50in and everything after it. Read `s.top`.

**Reserve space for a bottom statement before sizing what sits above it.** Columns
sized to the body floor plus a statement bar at the bottom is an overlap by
construction. `s.statement_reserve` exists so the arithmetic is stated once.

## Build and deploy

**The skill is copied into a harness, never linked, and still resolves its own root.** The
launcher resolves with `cd -P` and walks up from its own location rather than naming a
checkout path, because Sjel sits at `~/Developer/Sjel` on one machine and `~/Sjel` on
another. Hard-coding either breaks the other. `cd -P` is no longer load-bearing — the
materializing deployer that replaced symlinks on 2026-08-09 is the only one left, and it
refuses a symlinked source outright — but it stays, because a user who symlinks the skill
into their own checkout would otherwise get a root one hop off.

**Dependencies are pinned in the launcher, per verb.** `build` and `check` resolve
`python-pptx` and `pillow`; only `render` adds `pymupdf`. Resolving the rasteriser on
every build costs seconds for nothing.

**A theme is loaded relative to the deck file, not the process cwd.** `Theme.load()` is
called with a `Path(__file__).parent`-relative path in the deck module, so the deck
builds the same from anywhere.
