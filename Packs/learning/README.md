# learning

Teaching, practice and curation that grow the Obsidian vault. The skills hold the method. Pi stores conversation history, exposed by `extensions/sessions.ts`.
The vault holds
the result as concept notes and claim notes. Open work and decisions live in
[`docs/plans/learning.md`](../../docs/plans/learning.md).

The Pack depends on [`knowledge`](../knowledge/README.md) by name. Every vault write goes
through the `obsidian` skill, and graded quizzes come from that Pack's `quiz` pi extension.
Without `quiz` (other harnesses, non-interactive runs), `teach` asks numbered text questions.
The Learn plugin renders `quiz` and `ask` as cards in Obsidian. Decision cards preserve drafts
on idle deferral and resume within the same sitting.

## Skills

- **`teach`**: one-to-one teaching at the measured edge of the student's understanding: probe
  with graded quizzes, plan a dependency path, teach one reasoning step at a time. Each answer
  carries the learner's thinking and a confidence, each step opens with a prediction, and a
  session closes by extracting claim notes (`type: claim`) with their `## Because` arguments.
  In Claude Code, `teach` uses `AskUserQuestion`: the item plus a confidence question per
  round, the thinking in the option note.
- **`practice`**: practice on claims already taught: interleaved problems that never name
  their target, one hint at a time, and a retest of every miss or an explicit pending retest when the sitting stops. Reads claim notes, writes only
  the Learning Log line.
- **`curate`**: maintain one note and its directly related notes by slimming repetition,
  adding explanatory links, extracting reusable concepts and consolidating overlap. Preserve
  sources, qualifications and learning progress. Preview merges, deletions and substantial
  restructuring before applying them; curation creates no teaching/practice log entry.

## Figures

`teach/references/figures.md` decides when a step gets a figure and which form it takes: an
SVG, a hand-written `interactive` block, or an Anatomy figure embedded with `html-embed`. The
`interactive` Obsidian plugin that renders both blocks lives in
[`Packs/knowledge/plugins/interactive/`](../knowledge/plugins/interactive/README.md).

Anatomy is not part of this Pack (`upstreams.toml`, `[anatomy]`). Install it once per machine
so that pi finds it from any working directory, the vault included:

```sh
npx skills add wheresryan22/anatomy -g -a pi claude-code
```

## Obsidian plugin

[`plugins/pi-learn/`](plugins/pi-learn/README.md) runs teach, practice and curate sittings with pi in
Obsidian's sidebar, beside the notes they write. It holds the sitting controls, keeps answered
cards readable, and runs the skill's closing procedure on request. Its `learning-hub` Markdown block projects
paths, prerequisite-aware next steps, claims and recent handoffs from the vault. Continue restores the latest saved conversation for the selected track and mode; History
opens older conversations, and New session starts fresh. The session planner accepts optional
outcome and time settings; browsing notes leaves an active sitting's context fixed. The shared vault layout separates Learning, Knowledge and Resources;
the plugin checks mechanical issues through **Check vault structure**. Curate uses scoped
structural findings as context for an agent-led improvement pass. Build and install it with
`tools/obsidian-plugin pi-learn install`; `bun run check` in the plugin directory typechecks it
and runs its DOM tests.

## Activate

```sh
tools/harnesses sync learning
```

## Attribution

`teach` started from [amosblomqvist/learn](https://github.com/amosblomqvist/learn). Its
prediction step, claim extraction and the `practice` skill paraphrase ideas from
[amosblomqvist/learnharness](https://github.com/amosblomqvist/learnharness) (AGPL-3.0). No text
is copied, and the verdict is in `upstreams.toml` (`[learnharness]`).
