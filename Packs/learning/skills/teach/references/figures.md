# Teaching figures

Read this at Phase 3, step 3, when a figure carries the idea better than text does.

## When to draw

- Draw when the concept has state, flow, a threshold or a mechanism that the student can act on.
  Do not draw a figure that only repeats the text.
- A figure tests a prediction. Run this loop:
  1. Ask the prediction in the terminal ("what happens to the queue when arrivals double?").
  2. The student moves the control in Obsidian.
  3. Ask what they saw, in their words.
  4. Explain the gap between the prediction and what they saw.
- Build a simulation when the learner asks to see or to try something. When the concept is
  dynamic (a parameter to sweep, a process over time, a distribution to sample), offer one.
- A figure is a full part of its note. The note's text does not repeat what the figure shows.

## Build in the background

A figure build must not stop the teaching. This follows the demo builder of LearnHarness
(`upstreams.toml`, `[learnharness]`).

1. Write a brief that stands alone. The builder does not see the conversation. The brief gives
   the concept, the prediction that the figure answers, each control and what it changes, the
   formulas and constants, the readout and the assumptions for the caption.
2. Start the build in a background agent: pi-subagents in Pi, the `Agent` tool in Claude Code.
   Tell the learner in one line that a figure is in progress, then continue to teach.
3. The builder writes the asset, renders it with `tools/figure-shot`, reads both PNGs, and fixes
   the figure until every number and state on it is true. It reports the path only after that
   check.
4. When the figure arrives, embed it and teach with it. Name the control and the value to try:
   "set consumers to 4 and watch partition 3". A figure that no turn refers to is decoration.
5. If the build fails, say so and continue in prose. Make the brief more exact and try again
   once.

## Pick the form

| Concept | Form | Embed |
|---|---|---|
| Static structure or geometry | SVG | `![[name.svg]]` |
| State machine, queue, curve, one parameter | Hand-written interactive HTML | `interactive` block |
| Physical mechanism: a lock, a pump, a gauge | Anatomy figure | `html-embed` block |

Most teaching figures are the second row. Use Anatomy only when the student understands the
concept by recognizing a machine. Anatomy's own description matches any diagram request. In a
teaching session this table decides, and Anatomy loads only for the third row.

## Anatomy of a teaching figure

A small simulation of one mechanism, usually 2–8 KB of HTML. From top to bottom:

1. A one-line readout row: the control's current value against the fixed input
   ("processing: 6 / arrivals: 10").
2. The control: one slider or one toggle between two modes.
3. A status banner that changes colour and text when the system changes state ("poll blocked:
   offsets stop committing, lag grows"). The state change is the lesson.
4. The drawing: bars, queues, lanes or a timeline, driven by the model.
5. Pause and Reset buttons when the figure animates.
6. A caption with the model's assumptions ("each queue holds 20 units, each stage passes at most
   10 per tick").

A visually hidden heading (`class="sr-only"`, styled by the plugin) describes the whole
behaviour in one sentence, for screen readers and
for search.

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

## Defects to remove from imported figures

Figures exported from chat tools often run, but they break in a vault. Fix each one before
embedding:

| Defect | Fix |
|---|---|
| Host CSS variables (`--text-secondary`, `--border`, `--text-danger`) | Map to the Obsidian variables below |
| `document.getElementById` | Scope lookups to the root: `root.querySelector` |
| `setInterval` with no cleanup | Clear it in the `interactive-dispose` listener and pause it offscreen |
| Hard-coded hex colours | Derive colours from theme variables |
| Icon fonts or scripts from a CDN, full exported pages | Rewrite as a self-contained fragment |

## Native `interactive` block

````markdown
```interactive
file: Knowledge/<note folder>/Assets/<kebab-name>.html
height: 380
```
````

- Write self-contained HTML: inline CSS, inline classic scripts, no network requests.
- Scripts run in Obsidian's global context, and several embeds can share one note. Get the
  root with `document.currentScript.parentElement` and scope every DOM lookup to it.
- Register one `interactive-dispose` listener on the root. It clears timers, animation frames
  and global listeners.
- Listen for `interactive-visibility` on the root. Pause animation when `event.detail.visible`
  is false and resume it when true. Assume the figure is visible when its script starts.
- Native scripts run with Obsidian's own privileges. Never call `require`, `fetch`, or any API
  outside the figure's root.
- Use the vault theme variables `--text-normal`, `--text-muted`, `--background-secondary`,
  `--interactive-accent` and `--font-monospace`. Listen for `obsidian-theme-change`.

## Anatomy figures

- Anatomy is an upstream skill (`upstreams.toml`, `[anatomy]`), installed globally in
  `~/.agents/skills/anatomy`. Load it by name. If it is not installed, write a hand-written
  figure instead.
- Build only Anatomy's Figure tier (one mechanism, 40–80 parts) during a session. A Figure can
  take up to an hour, so do not make the student wait: start the build in a background `Agent`
  (pi-subagents in Pi, the Agent tool in Claude Code) and keep teaching. Use a hand-written
  figure or no figure for the current step, and embed the Anatomy figure when the agent reports.
- A Hero or an Epic takes hours. Propose it as follow-up work, not as a teaching step.
- Anatomy's `build.mjs` writes a standalone page. Copy that page to
  `<note folder>/Assets/<kebab-name>.html`, or to the place the vault contract sets.
- Embed it with `html-embed`, not `interactive`. The native renderer keeps only the children of
  `<body>`, so it drops the `class="iso"` and `data-theme` that Anatomy puts on `<body>`. The
  iframe renderer keeps the document, sets `data-theme` to the vault theme, and maps the
  `--anatomy-*` variables to vault variables.
- Never run Anatomy's gallery publish step from a teaching session.

## Check before embedding

1. Render the figure to a PNG and read the PNG. For an `interactive` figure, run
   `tools/figure-shot <asset.html>` in the Sjel checkout; it renders light and dark. For SVG, use `qlmanage -t -s 1000 -o /tmp
   <file>.svg`. For an Anatomy page, use
   `node ~/.agents/skills/anatomy/scripts/capture.mjs` (Anatomy's `references/verify.md`).
2. Open the note in Obsidian and check both themes, the note width, the controls and a reopen of
   the note.
3. If step 2 did not run, say so in the session report.
