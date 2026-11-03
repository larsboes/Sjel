# Teaching figures

Read this at Phase 3, step 3, when a figure carries the idea better than text does.

## When to draw

- Draw when the concept has state, flow, a threshold or a mechanism that the student can act on.
  Do not draw a figure that only repeats the text.
- A figure tests a prediction. Ask the prediction in the terminal first ("what happens to the
  queue when arrivals double?"). Then the figure shows the answer.

## Pick the form

| Concept | Form | Embed |
|---|---|---|
| Static structure or geometry | SVG | `![[name.svg]]` |
| State machine, queue, curve, one parameter | Hand-written interactive HTML | `interactive` block |
| Physical mechanism: a lock, a pump, a gauge | Anatomy figure | `html-embed` block |

Most teaching figures are the second row. Use Anatomy only when the student understands the
concept by recognizing a machine.

## Rules for every interactive figure

These rules paraphrase the Anatomy skill ("Truth first", "Frame it", "Make it live") for
teaching.

1. **One model.** Write the behavior as a small pure function of the inputs. The motion, the
   readout and the caption read from that function. Every number on the figure is true.
2. **One control per idea.** Give the student the variable that the step teaches, as a slider
   or a button. Do not add controls for other steps.
3. **Live readout.** Show the value that the prediction was about, with its unit.
4. **Frame.** Show a title, a one-line hint that names the control, and the readout. Keep text
   out of the drawing where possible.
5. **Calm motion.** Use eased, time-based motion. Stop animation when the figure is offscreen
   and when `prefers-reduced-motion` is set.
6. **Keyboard.** Every control works from the keyboard.

## Native `interactive` block

````markdown
```interactive
file: Knowledge/<domain>/Assets/<kebab-name>.html
height: 380
```
````

- Write self-contained HTML: inline CSS, inline classic scripts, no network requests.
- Scripts run in Obsidian's global context, and several embeds can share one note. Get the
  root with `document.currentScript.parentElement` and scope every DOM lookup to it.
- Register one `interactive-dispose` listener on the root. It clears timers, animation frames
  and global listeners.
- Use the vault theme variables `--text-normal`, `--text-muted`, `--background-secondary`,
  `--interactive-accent` and `--font-monospace`. Listen for `obsidian-theme-change`.

## Anatomy figures

- Anatomy is an upstream skill (`upstreams.toml`, `[anatomy]`), installed globally in
  `~/.agents/skills/anatomy`. Load it by name. If it is not installed, write a hand-written
  figure instead.
- Build only Anatomy's Figure tier (one mechanism, 40–80 parts) during a session. A Hero or an
  Epic takes hours. Propose it as follow-up work, not as a teaching step.
- Anatomy's `build.mjs` writes a standalone page. Copy that page to
  `$VAULT/Knowledge/<domain>/Assets/<kebab-name>.html`, or to the place the vault contract sets.
- Embed it with `html-embed`, not `interactive`. The native renderer keeps only the children of
  `<body>`, so it drops the `class="iso"` and `data-theme` that Anatomy puts on `<body>`. The
  iframe renderer keeps the document, sets `data-theme` to the vault theme, and maps the
  `--anatomy-*` variables to vault variables.
- Never run Anatomy's gallery publish step from a teaching session.

## Check before embedding

1. Render the figure to a PNG and read the PNG. For SVG, use `qlmanage -t -s 1000 -o /tmp
   <file>.svg`. For an Anatomy page, use its `scripts/capture.mjs`.
2. Open the note in Obsidian and check both themes, the note width, the controls and a reopen of
   the note.
3. If step 2 did not run, say so in the session report.
