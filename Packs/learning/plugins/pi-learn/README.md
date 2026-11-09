# Learn: pi in the Obsidian sidebar

A desktop Obsidian plugin that runs teach and practice sittings with pi, beside the notes they
write. It runs on macOS, Linux and Windows.

## What it does

- Starts `pi --mode rpc` in the vault (`docs/rpc.md` in the pi package). Pi's own settings
  supply the skills and extensions, so `teach`, `practice` and the `questions` extension load as
  they do in the terminal.
- Renders replies with Obsidian's Markdown renderer. `interactive` figures, mermaid, LaTeX and
  `[[links]]` render inside the chat through the plugins that already handle them in notes.
- Renders `quiz` and `ask` as inline cards. It sets `PI_QUESTIONS_CLIENT=pi-questions/v1`, and
  `Packs/knowledge/extensions/questions.ts` (`fromHost`) hands each widget to the plugin as JSON
  instead of falling back to plain text. A card returns the same result object as the terminal
  widget: answer, thinking, confidence, note and contest.
- Holds Teach and Practice as persistent controls. Enter in an empty view starts the chosen kind
  from the open note, path or typed topic. Starting over an unfinished sitting asks first.
- Closes a sitting on request. **Finish sitting** stops any outstanding question and asks the
  active skill to run its closing procedure; the sitting closes only after a complete reply with
  no failed tool call, so a claim, path or Learning Log write cannot be assumed.
- Keeps every answered card expandable with its question, choice, thinking, confidence and
  feedback. A prediction is reported as explored, not scored.
- Leaves the reader's scroll position alone while an answer streams, unless the reader is at the
  bottom. The composer's button shows Stop while that is what it does.
- Follows eligible concept and claim notes under `Knowledge/`, or learning path notes. Dashboard, MOC, reference and configuration notes are excluded. An active sitting keeps its context when notes are browsed. Before a sitting, an explicitly selected planner target takes priority over the open note. A path note is the path. An eligible concept or claim note is the focus, and its path is a
  learning path that links to it or that it links to. Teach and Practice start from that context:
  the path at the focus note, the path alone, or the note alone when no path holds it.
- Reads the path with the vault contract's learning-path schema: node states as dots in the
  header, and the graph itself under Plan.
- Labels each quiz card by its `purpose`: probe, prediction or check.
- Shows context used and the estimated API cost of the sitting (`get_session_stats`) in the footer.

## Sittings

Teach and Practice each start a fresh pi session: the vault is the memory, and a sitting resumes
from the path graph's `current` node. Decided 2026-11-05 (`docs/plans/learning.md`, stage 3).
The session planner accepts a path or new topic, desired outcome, available minutes and mode.
Leaving minutes blank asks the tutor to establish the budget once. Planning starts no model
conversation. Starting a sitting uses the single current graph node, otherwise the first open
node whose prerequisites are held. Cycles, multiple current nodes and unsupported dependency
syntax require inspection rather than selecting an arbitrary node. Directed dependency chains
with `-->`, `-.->` and `==>` are supported. With no learning path open, type a topic first. A message sent while pi works becomes a steer.

## Dynamic home

Embed the following in any home note, with optional `cssclasses: [learning-hub]` for the
reading layout:

````markdown
```learning-hub
```
````

The block lists graph-bearing path notes, their progress and next ready step, existing claim
notes and the five latest Learning Log lines. Vault and metadata events refresh it after
create, edit, rename and delete. Empty, blocked and unreadable paths have explicit states.
Plan buttons open the sidebar planner without starting a sitting. Data remains in Markdown;
the hub holds no saved counts, topic links or duplicate progress fields.

Both surfaces use the optional CSS variables `--learn-gap`, `--learn-width`, `--learn-accent`,
`--learn-rule` and `--learn-radius`. A single vault snippet can override these; defaults come
from the active Obsidian theme. Structural styles belong to the plugin.

Finish sitting saves completed work plus an explicit pending handoff for unfinished gates,
claim reviews, problems, retests and contests. It does not require new questions to end a
sitting. Required record-write failures still prevent the closed acknowledgement.

## Settings

| Setting | Meaning |
|---|---|
| pi command | The pi executable. macOS and Linux start it through the login shell, so a name on the shell's `PATH` works. |
| Run pi in | Windows only: Windows, or WSL with the vault path translated by `wslpath`. |
| Teach model, Practice model | Picked with a fuzzy search over the models pi lists (`get_available_models`), with a thinking level each. Pi's default applies when unset. The footer's model chip and thinking menu change the running sitting and remember the choice for its kind. |
| Follow edits | Open the note pi writes. |

## Build and install

```sh
tools/obsidian-plugin pi-learn install
```

Then reload the plugin in Obsidian. `tools/obsidian-plugin.test.ts` fails when the built or the
installed copy is older than `src/`. `tools/pi-learn.test.ts` covers the path reader and the
launch command per platform.

`bun run check` in this directory typechecks `src/` and runs `tests/` against linkedom: the cards'
keyboard and cancellation paths, sitting start, reset and finish, pending dialogs, and scroll
position. Those tests use a stub for Obsidian, so they do not prove the view renders.

## Check

The view needs Obsidian, so these checks are manual:

1. Open a learning path note, open Learn from the ribbon, type a topic and press Enter. Teach
   starts, and the Teach, Practice and Finish controls stay visible after the first reply.
2. A `quiz` card appears, takes thinking and confidence, and reveals the verdict. Its answered
   question stays expandable after Next, and Stop works during the reveal.
3. The concept note pi writes opens in the main pane, and an `interactive` figure in a reply
   renders.
4. Press **Finish sitting**. The closing questions or summary arrive, and the controls report a
   closed sitting only when the skill completed its writes.
