# presentations pack

Turns an argument into a deck, and an academic argument into a deck that survives a
defence. Two skills. **`slide-deck`** owns the deck mechanism (a role-based theme, layout
archetypes, a deterministic build, a render loop, and an argument extractor) and the
judgment about what a slide owes the one after it and what an academic talk is judged on.
**`diagrams`** renders Mermaid sources to figures with the colours of the deck that will
embed them, so a diagram and the slides around it cannot drift apart.

## Why it exists

"Make me a deck" produces two different failures, and they need different tools.

The first is a deck that reads correctly in source and is visibly wrong on screen: drop
shadows on every box, bullets silently centred, a statement bar drawn on top of the
columns above it, a dead bottom third. None of it is visible in code; all of it is
obvious in a render.

The second is a deck that is mechanically clean and **answers the wrong question**. For a
project readout this costs polish. For an academic talk it costs the talk: a research
question that names the case as its subject, a contribution that is bounded in every
sentence and never claimed, a related-work matrix that lists what studies did and
positions nothing. A real first assessment of a DSR thesis put **ten of fifteen lost
points** into four of those five failures, while praising the methodology as
"extensive, well-derived and argued". One discipline produced both the praise and the
deduction.

The mechanism catches the first failure without eyes. The genre profiles and the
readiness pass catch the second.

## The mechanism

```
skills/slide-deck/
  SKILL.md              router: genre → brief → spine → content → build/check/render → readiness
  scripts/deck          bash + uv launcher, delegates to scripts/deckkit
  scripts/deckkit/      theme, layout, markup, slide, deck, check, render, readiness, cli
  references/
    genre-dsr-defence.md          DSR / qualitative thesis defence, the spine, the Q&A, the traps
    genre-empirical-cs-talk.md    empirical-CS conference talk, the spine, the novelty question
    genre-academic-job-talk.md    academic job talk, the research-programme spine
    academic-narrative.md         the objects: question, contribution, bounding register,
                                  construct column, the four validities
    evidence-on-slides.md         n, uncertainty, the bound; reproduce figures; access and colour
    citing-on-slides.md           inline attribution, the one references slide, reproduced material
    academic-register.md          person, tense and register of a spoken sentence
    defence-readiness.md          the five questions a reviewer answers against the argument
    narrative.md                  genre-neutral craft: assertion titles, claim/evidence/bound, budget
    design-system.md              colour roles, type scale, grid, emphasis, density, access
    slide-recipes.md              archetypes, primitives, and the academic compositions
    theming.md  verification.md  gotchas.md
  assets/deck.template.py         what `deck init` scaffolds
  assets/themes/                  warm-scientific-teal, slate
  evals/evals.json                behaviour evals with and without the skill

skills/diagrams/
  SKILL.md              router: source → derived theme → render → measure the aspect → place it
  scripts/diagrams      bash + uv launcher, delegates to scripts/diagramkit
  scripts/diagramkit/   theme (palette → Mermaid config), render (mermaid-cli, pinned), cli
  references/mermaid.md the variable mapping, the layout defaults, how to write a source
```

A deck is a Python module that builds one `Deck`. A module rather than a data format
because a deck's structure is a sequence of decisions with numbers in it, a time
budget, a run count, a matrix row, and Python reads those better than YAML does.

```python
s = deck.open("Die Forschungsfrage fragt nach einer Klasse —\n"
              "der Fall ist das Untersuchungssetting", "2 · Forschungsfrage")
s.headline("Welche Rolle spielt Context Engineering bei Sprach- und "
           "Laufzeitmigrationen, deren ~Verhaltensanforderungen außerhalb des "
           "Quellcodes~ liegen?", height=1.85, size=20)
s.panel(s.x, s.top + 2.07, s.w, 0.72,
        "~Untersuchungssetting:~ eine hochgradig kundenspezifische Pipeline …",
        voice="caution", size=13.5, pad=0.24, anchor="middle")
```

That is the abstraction a written assessment will mark as "only partly achieved" if the
case is the subject of the question rather than the setting for it.

## How it works with the other skills

Five skills share this ground. Each one owns a different thing.

| skill | owns |
|---|---|
| `academic-writing` | The argument in the paper or thesis, and the constructs a talk renders: contribution types, the validity taxonomy, the citation discipline. |
| `human-writing` | The prose a person reads. Speaker notes, README text, and any explanatory writing in a deck. |
| `asd-ste100` | Text that a machine parses without a person to resolve ambiguity. Error messages, tool descriptions, and the instruction files of this Pack. |
| `unslop` | Source code and web interfaces. |
| **this Pack** | The deck module, the theme, the layout, the speaker notes and the rehearsal passes. |

Two consequences follow.

The instruction files in this Pack follow the `asd-ste100` structural rules, because an
agent parses them. The sentences are short and active, each sentence carries one
instruction, and the files use no semicolons. Run `ste-lint.py` on a reference file to
check that.

The talk's own words stay with the user. When a deck reads as machine-written, hand the
notes and the slide text to `human-writing`. When the argument is unclear, hand the source
document to `academic-writing`. This Pack does not rewrite either one.

## The passes, and the different questions they answer

```
build → check → render → LOOK → timing → readiness → answer the five → handout → rehearse
```

| verb | answers | needs eyes | exit code |
|---|---|---|:--:|
| `build` | does it compile into a .pptx | no | 0, or 2 on a broken module |
| `check` | is anything mechanically broken | no | the error count |
| `render` | does it look right | **yes** | 0 |
| `timing` | does the script fit the time budget | no | 0 |
| `readiness` | does it argue the right thing | **yes**, by a reviewer | 0 |
| `handout` | can it be rehearsed from | no | 0 |

`check` cannot see a shadow; `render` cannot see whether the argument answers the
question; `readiness` cannot see that the script takes twelve minutes. Each is cheap and
none substitutes for another.

`check` covers twelve findings, including shape-on-shape overlap, what catches a statement
bar landing on the columns above it, and `figure`, an undescribed picture. Its exit code is
the error count, so `deck check && deck render` is a usable gate.

`render` emits a PDF, one PNG per slide, and a **contact sheet**, a 3×3 grid of the whole
deck in one image. A deck is a sequence, and a sequence cannot be judged one slide at a
time. Every entry in `gotchas.md` was found by looking at a contact sheet and nothing else.

`readiness` prints the argument skeleton, the ordered assertions, the sentences slides
exist to deliver, the closing verdict, the question each backup slide answers, so a
reviewer can read it in one screen and answer the five questions in
`references/defence-readiness.md`. It is deliberately not a checker: whether an argument is
any good is not a rule, and a script that guessed would be trusted.

`timing` reads the `ZEIT h:mm–h:mm (N s)` line and the quoted `Sprechtext` back out of the
notes, sums the budget against `Deck(minutes=…)`, and reports each script against its slot at
125 wpm. On the worked deck it reports the finding that matters, 1,548 spoken words, 12:23
at a deliberate pace, against a noted 10:00.

`handout` composes a printable PDF, one page per slide: the slide image, its notes and its
clock. It is what the speaker rehearses from, and what an examiner occasionally asks for.

## Genre profiles

Two talk genres ship, mirroring the **`academic-writing`** skill's paper genres so the
two cannot drift: `genre-dsr-defence.md` (a design-science or qualitative thesis
defence) and `genre-empirical-cs-talk.md` (a conference talk of a paper with baselines
and ablations). They are not interchangeable, a defence is judged on abstraction,
positioning and contribution; a paper talk on beating a baseline.

The underlying constructs (contribution types, the validity taxonomy, the rigor and
relevance cycles, evaluation-strategy fit) live in `academic-writing`'s genre
references; `academic-narrative.md` renders them for the talk and points there rather
than restating them.

## The first deck

The skill was extracted from a 32-slide DSR bachelor colloquium deck (21 presentation
slides, 11 backup), the worked example that keeps the references honest, and the source
of the readiness questions. It built clean and passed `check` with 0 errors and 0
warnings after the port; the port itself found four defects the library now prevents. It
is also where the awkward parts of the API come from: `reserve=s.statement_reserve`
exists because a statement bar was drawn on top of three columns, and `deco=True` exists
because the overlap check needs to know a hairline is not a block.

The same deck is the worked example for the academic layer: its research question states
a class with the case as the setting, its contribution is claimed in one sentence before
it is bounded, and its backup set includes the map from each point in the written
assessment to the answer and the evidence.

## Activate

```bash
"$SJEL_ROOT/tools/harnesses" status presentations
"$SJEL_ROOT/tools/packs-claude" deploy presentations    # -> ~/.claude/skills/slide-deck
"$SJEL_ROOT/tools/packs-pi"     deploy presentations    # -> registered in pi's settings
```

Requires `uv` on PATH and **LibreOffice** for `render` (`brew install --cask
libreoffice`). Both are `toolchain.toml` entries; `tools/doctor` warns when either is
missing.

## Attribution

Original work, no upstream. The design system is transcribed from a reference deck the
user supplied and re-coloured from that project's own figure palette
(`assets/themes/warm-scientific-teal.json`, provenance recorded in the file). The
academic layer is grounded in the source thesis's first assessment and its validity
taxonomy (Wohlin et al.), with the constructs sourced from the `academic-writing` Pack.
The `deckkit` code is written against `python-pptx` and `pymupdf`, both pinned in
`scripts/deck`.
