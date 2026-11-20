# Learn: pi in the Obsidian sidebar

A desktop Obsidian plugin that runs teach, practice and curate sittings with pi, beside the notes they
write. It runs on macOS, Linux and Windows.

## What it does

- Starts `pi --mode rpc` in the vault (`docs/rpc.md` in the pi package). Pi's own settings
  supply the skills and extensions, so `teach`, `practice`, `curate` and the `questions` extension load as
  they do in the terminal.
- Renders replies with Obsidian's Markdown renderer. `interactive` figures, mermaid, LaTeX and
  `[[links]]` render inside the chat through the plugins that already handle them in notes.
- Renders `quiz` and `ask` as inline cards. It sets `PI_QUESTIONS_CLIENT=pi-questions/v1`.
  `Packs/knowledge/extensions/questions.ts` sends JSON through an `editor` dialog for quizzes
  and an abortable `input` dialog for decisions. Decision cards wait for an answer or explicit
  cancellation, without an idle timeout, pending queue or resume command. Card results retain
  answer, thinking, confidence, note and contest. Cancellation grants nothing.
- Holds Teach, Practice and Curate as persistent modes. **Continue** restores the latest saved
  conversation for the selected track and mode, or starts one when none exists. **New session**
  starts fresh with one click; **History** searches saved learning conversations. Enter in an
  empty view uses Continue for the open note, path or typed topic.
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

Permission and other plain dialogs collapse into a result receipt after an answer. Requests
with a timeout expire locally as well as in Pi, disable their controls and never accept a late
click as approval. Routine Markdown writes use machine-configured vault permissions; Curate
uses direct note tools rather than introducing compound shell backup commands.

Curate starts from the open concept, source, exercise or learning path. Its scope contains the
anchor and directly linked active notes, including backlinks; it excludes vault rules, logs and
hidden archives. The opening prompt includes the scoped structure check. The curate skill
slims repetition, adds explanatory links and proposes consolidation without changing mastery,
source qualifications or learning-path states. Merges, deletions, moves, splits and substantial
rewrites receive a concrete preview before approval. Routine wording and metadata fixes apply
directly within scope. Curate uses plain chat questions for necessary approvals. A visible fixed-target
notice explains when the currently browsed note differs from the sitting's anchor.
These semantic rules are agent instructions;
the plugin does not enforce them as a filesystem policy. Finish verifies saved edits and
summarizes pending proposals without creating a teaching/practice log entry. Each mode remembers
its own model and thinking level. Open a note and choose Curate, or select Curate in the planner.

Pi saves conversations in its native session directory. `Packs/learning/extensions/sessions.ts`
uses Pi's session discovery to return metadata for the current vault, including older Learn
sittings. Continue switches to a saved session and restores its messages without a model call;
answered cards appear as read-only receipts. Live, unsubmitted card inputs do not survive a
process restart. No pending-decision queue is kept. The vault remains the record of learning
progress; New session starts at the path graph's `current` node rather than assuming mastery.
The session planner accepts a path or new topic, optional outcome and minutes, and mode.
Blank minutes impose no time budget and no setup question. Planning starts no model conversation. Starting a sitting uses the single current graph node, otherwise the first open
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

`Resources/Vault Layout.json` supplies the vault-relative `learning`, `knowledge`, `resources`
and `log` paths for the plugin and skills. Without it, the defaults are `Learning`, `Knowledge`,
`Resources` and `Learning/Learning Log.md`. Roots must be distinct and non-overlapping; the log
must be inside the learning folder. The plugin reads the layout at startup and before review.

**Learn (Sjel): Check vault structure** previews misplaced typed notes, missing required metadata and
unresolved wikilinks. Each Apply move action uses Obsidian's file manager, respects its link-update
setting and refuses existing destinations. Ambiguous notes require inspection; the command
does not infer semantic duplicates, merge notes, move figures or change learning evidence.
New concepts may use `type: concept`; existing `type: note` concepts remain eligible in Knowledge.
Exercise, log, reference and system types make placement explicit. Learning paths remain
discoverable by their heading. The hub lists reusable knowledge and supporting resources separately.

| Setting | Meaning |
|---|---|
| pi command | The pi executable. macOS and Linux start it through the login shell, so a name on the shell's `PATH` works. |
| Run pi in | Windows only: Windows, or WSL with the vault path translated by `wslpath`. |
| Teach model, Practice model, Curate model | Picked with a fuzzy search over the models pi lists (`get_available_models`), with a thinking level each. Pi's default applies when unset. The footer's model chip and thinking menu change the running sitting and remember the choice for its kind. |
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
5. Reload Learn, then choose Continue on the same track. The saved conversation opens without
   generating a new reply. History opens an older sitting; New session starts fresh without
   changing or deleting its predecessor.
