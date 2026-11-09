# Learning: teach, practice, and claims in the vault

Learning runs in the vault first. The `teach` and `practice` skills in `Packs/learning/` hold
the method, and the vault holds the result as claim notes. A Sjel learning surface is built
only when vault sessions show friction that a UI removes.

Decisions (2026-10-28):

- One claim note per proposition, `type: claim`. The original target was
  `Knowledge/<domain>/Claims/`; the convention reconciliation below governs initial placement.
- Vault-first, staged. A Sjel capability comes after real sessions, not before.
- The phone only reads notes for now. Phone practice decides between Obsidian, an Obsidian
  plugin and a Sjel client.

Prior art: `capabilities/learning` existed as a README with no code and was deleted on
2026-08-25 (`upstreams.toml`, `[slm-finetuning-talk]` row). The ideas used here come from
LearnHarness (`upstreams.toml`, `[learnharness]`).

## 1. Stage 1: the vault skills

Implemented in the skills: prediction step, thinking and confidence, and claim extraction in
`teach`, plus `practice`. The current vault contract delegates claim structure to `teach`
rather than defining a claim row. `quiz` takes thinking and
confidence before the reveal. In Claude Code, `teach` uses `AskUserQuestion` with a confidence
question and the option note. The `🧩 Claims` view in `Knowledge.base` lists a concept's claims.

Shipped 2026-11-03: `teach` and `practice` moved to their own Pack, `Packs/learning`, and
`teach/references/figures.md` covers figures, including Anatomy figures in `html-embed`.
Anatomy is installed globally in `~/.agents/skills/anatomy`, which Pi scans, with a Claude Code symlink (installed 2026-11-04 with the command in [`Packs/learning/README.md`](../../Packs/learning/README.md)).

Open:

1. Run five real `teach` sessions, each followed by `practice`. Done when
   `Resources/Learning Log.md` has five teach lines and five practice lines.

Not checked: no figure built by `figures.md` has rendered in Obsidian yet, and no Anatomy
page has been checked inside `html-embed`. Also not checked: the `🧩 Claims` view renders only in Obsidian, and no one has looked at it yet. Also not checked: no session has run on the new skill text yet.

Shipped 2026-11-04:
- The vault contract (`Resources/Vault-Contract.md`, configured as `contract_path`) now
  defines agent access, `status: verified` (facts checked against `sources:`), note anatomy,
  interactive material and teaching records. Interactive figures are full parts of their notes,
  with no static fallback.
- Vault layout, decided 2026-11-04: `Knowledge/` holds only what is learned (concept notes enter
  as seedlings while taught). `Resources/` holds the Bases, the contract, the Learning Log,
  learning paths in `Resources/Learning/` and verified reference notes. `Knowledge.md` at the
  vault root is the home note.
- `teach/references/explanations.md` sets the shape of one explanation step.
- `figures.md` adds the teaching-figure anatomy, the loop around a figure and the defects of
  imported figures.
- `teach` Phase 3 adds retrieval, a response to the reasoning, and adaptation by correctness and
  confidence.
- `quiz` and `ask` render inline LaTeX through pi-tui's `renderLatex`
  (`tools/pack-extensions.test.ts`, "quiz math").

- `interactive` plugin 0.3.2: the installed build was stale and never sent `interactive-dispose`.
  It is rebuilt with `tools/obsidian-plugin interactive`, and `tools/obsidian-plugin.test.ts` fails when
  `src/` changes without a rebuild. The plugin also sends `interactive-visibility`, styles
  `.sr-only`, and no longer grants `allow-same-origin` to `html-embed` frames.
- `tools/figure-shot` renders an `interactive` figure to PNG in light and dark with headless Chrome.

Not checked: no session has run on this text. No figure has been checked against the new anatomy
in Obsidian. Display math and matrices in quiz text stay raw source, because a widget line holds
one row.

## 2. Stage 2: practice on the phone

Open. Decide after stage 1 between Obsidian mobile with a due-for-practice Base view, an
Obsidian plugin, and the Sjel agent on the phone ([`agent.md`](agent.md)). Done when one
practice session runs on the iPhone against vault claims.

## 3. Stage 3: the learning surface is Obsidian

Decided 2026-11-05, replacing "a Sjel surface only after friction": the surface is the Learn
Obsidian plugin, [`Packs/learning/plugins/pi-learn/`](../../Packs/learning/plugins/pi-learn/README.md),
built directly rather than after an Agent Client trial. It must run on macOS, Linux and a Windows
laptop whose code lives in WSL, so it is a desktop plugin that starts `pi --mode rpc`, not a web
host. Obsidian in a browser (a VNC-streamed container) was rejected as laggy and a second app to
maintain. Choices, decided 2026-11-05:

| ID | Decision |
|---|---|
| S1 | The chat is a sidebar view. The main pane opens the note pi writes. |
| S2 | Each Teach or Practice sitting starts a fresh pi session. The vault is the memory, and a sitting resumes from the path graph. |
| S3 | `quiz` and `ask` render as inline cards. `questions.ts` hands them over through `pi-questions/v1` (`fromHost`). |
| S4 | The learning-path schema (node states `held`, `current`, `open` as mermaid classes) lives in the vault contract; `teach` writes it and the plugin reads it. |
| S5 | The vault root carries an `AGENTS.md` that only points at the contract and the skills. |
| S6 | The plugin's context follows the open note: a concept note is the focus, and its path is the learning path linked to or from it. |
| S7 | One explanation style for sessions and notes: `teach/references/explanations.md` (Method, Craft) paraphrases LearnHarness's method, with 3Blue1Brown as the reference. |
| S8 | `quiz` takes `purpose` (`probe`, `predict`, `check`); figures build in a background agent from a brief that stands alone, and are checked before they arrive. |

Pi Durable (`@earendil-works/pi-durable` 1.5.0) is not used: the vault is the durable store.
Reconsider it for stage 2, where a phone would be a second client of one sitting.

Shipped in `23d774c8`: the plugin with model and thinking pickers per
sitting kind, the context and cost meter, the Plan graph and the note-following context; the
`questions.ts` host protocol and `quiz` `purpose`; the schema in the vault contract and `teach`;
the Method, Craft and figure-build rules in `teach`; the migrated Data Pipeline path; and
`tools/obsidian-plugin` (was `tools/interactive-plugin`). Checked: `tools/pack-extensions.test.ts`,
`tools/pi-learn.test.ts`, `tools/obsidian-plugin.test.ts`, a strict `tsc` run against
`obsidian.d.ts`, one headless `pi --mode rpc` round trip in the vault (`Quiz complete: 1/1
correct.`), and the view in a running Obsidian on macOS through the Chrome DevTools Protocol:
it opens without errors, lists pi's models, and its header follows the open note.

- Shipped in `e692f482`: persistent sitting controls with an explicit finish, cancellation-safe
  expandable cards with full keyboard paths, scroll respect, and the plugin's own typecheck and
  DOM tests. The owner chose these fixes before a pilot (decided 2026-11-07); no model-backed
  sitting has run, and the five-plus-five sessions below remain the evidence that decides the
  remaining candidates.

Implemented locally, 2026-11-08: `learning-hub` renders a general home from path graphs,
claim notes and Learning Log handoffs. The sidebar planner accepts target, outcome, minutes and
mode; excludes non-learning notes; pins active context; and selects prerequisite-ready nodes.
`teach` and `practice` accept the brief once and save pending work on an explicit stop, with a
single-claim practice branch. Shared CSS roles keep vault customizations in one snippet.
Verification: plugin `bun run check`, `tools/pi-learn.test.ts`, `tools/obsidian-plugin.test.ts`,
skill validators and the hub/planner in Obsidian on macOS. A model-backed sitting on these
changes and Windows/WSL rendering remain unverified; the real-session pilot stays open.

Open, in order. The stage 1 sessions come first: the UI items below are candidates, and observed
friction in the Learning Log decides which of them are built.

Implemented locally, 2026-11-08: sidebar messages no longer shrink inside the transcript;
tool activity is expandable; provider failures show a readable reason, model picker and
collapsed technical details. Waiting, working and finishing have distinct labels. Finish no
longer adds a duplicate user bubble, and the model footer wraps in narrow panels. Verification:
plugin typecheck and 42 tests pass; build installed in the vault. Repository-level plugin tests
could not start (`EPERM` from Bun). Visual confirmation after the owner reloads remains open.

1. **Real sittings.** Run the five teach and five practice sittings of stage 1 in the plugin.
   Record friction in the Learning Log line. Done when the log has ten lines and each UI item
   below is marked observed or not observed.
2. **The new teaching rules in a real sitting.** No sitting has run on Method, Craft, `purpose`
   or the background figure build. Unverified: pi-subagents works in RPC mode, and a subagent's
   figure reaches the sitting. Done when one sitting shows a prediction card, a check card and a
   figure that the background build checked.
3. **Practice follows teach.** `practice` does not yet use `purpose: "check"` or the rule that
   each wrong option names a misconception. Done when `practice/SKILL.md` adopts both.
4. **Model choice.** The default is pi's (`opencode-go/deepseek-v4.1-flash`). No model was
   compared for teaching or figure quality. Done when one figure brief and one teach step have
   run on two models and the settings default records the choice.
5. **Windows.** Check native and WSL launch, and `follow` with a vault on NTFS. Not run.

UI and speed candidates, built only when item 1 observes the friction:

| Candidate | What it fixes | Smallest change |
|---|---|---|
| Streaming render | Each 120 ms re-render of the live message unloads and rebuilds its rendered components. | The answer no longer moves the reader's scroll position; incremental rendering of unchanged blocks is still open. |
| Model list | `get_available_models` lists 627 models, most without credentials. | Filter to providers with credentials, or to pi's `enabledModels`, before the picker. |
| Start time | The view and the settings tab each start pi; the settings tab starts one per open. | Reuse the view's pi for the settings tab and cache the model list. |
| Node ↔ note | The header cannot mark the focus note's node, and graph nodes do not open notes. | Schema: a node's label is its note's name, or a `click id "[[Note]]"` line; the plugin marks and links it. |
| Past sittings | Fresh sittings leave no list of earlier ones. | A list from pi's session directory for the vault, read-only. |
| Practice evidence | Progress is a held/current/open graph; practice leaves only its Learning Log line. | A per-claim practice record with a due date, kept separate from note `maturity`. |
| Input | No file, image or dictation input. | `prompt` accepts images; add a paperclip and a drop target. |
| Thinking | Model reasoning is hidden; a long silence reads as a hang. | A collapsed "Thinking" block fed by `thinking_delta`. |


Not planned: a LearnHarness-style background research agent, and a cost meter for subscription
providers, whose cost pi does not report.

## 4. Pi learning workflow proposal

Planning scope, 2026-11-03: learning inside Pi, integrated with Obsidian's existing notes and
links. LearnHarness is prior art, not an application to install or reproduce. This proposal
extends stage 1; it does not promote learning above its Priority 3 position in `PLAN.md`.

### Verified foundations

- `Packs/learning/skills/teach/SKILL.md` already specifies probing, a linked learning path,
  prediction, explanation, application checks, concept-note updates and claim extraction.
- `Packs/learning/skills/practice/SKILL.md` already specifies interleaved application problems,
  hints, reasoning and confidence, and retesting misses. It appends a session log but defines
  no persistent per-claim review schedule.
- `Packs/knowledge/pi-packages/obsidian-access/` already supplies governed search, backlinks,
  reads, validation and version-checked Markdown writes. Reuse this boundary for learning.
- The resolved vault's Learning Log contained zero recorded sessions when inspected. The
  five-session check in stage 1 remains open; absence from the log does not prove no learning
  occurred elsewhere. No vault contract was configured on this machine, although the skills
  and earlier plan refer to one. Resolve that discrepancy before a pilot writes claim notes.

The original [learn](https://github.com/amosblomqvist/learn) repository describes a Pi
configuration of skills, extensions and agent definitions, and links the video
“How I Use AI to Learn Things”. Its influence is already recorded in the learning Pack README
and the `[learnharness]` entry in `upstreams.toml`.

The saved source “I built an AI workflow to learn from long-form YouTube videos” was found in
the comms clip store through `comms clips` and `comms clips show`. The
[original post](https://www.reddit.com/r/automation/comments/1ov2ur4/i_built_an_ai_workflow_to_learn_from_longform/)
proposes a topic outline, a selected-topic deep dive and review questions. Treat its speed and
retention claims as unverified. Useful influence: preserve source locations and select a small
learning target before producing notes. No dependency on its suggested transcription service
is proposed.

### Proposed session

1. Start from a topic, existing note or source URL. Search the vault for canonical concepts,
   prerequisites and related claims. Follow relevant links selectively, with bounded context.
   An existing note is evidence of available material, not of learner understanding.
2. Probe understanding and agree on a small goal. Show which existing notes will be reused,
   which gaps matter and the first reasoning step. Read source material as evidence rather
   than instructions. Missing transcripts or visual-only content must be stated explicitly.
3. Run the existing predict, explain and check loop. Link ideas through explicit prerequisite,
   argument, example or contrast relationships; do not add links merely for keyword overlap.
   Separate source-supported statements from verified supplementary explanations. Preserve
   URLs and timestamps or section references where the source supplies them.
4. Close by updating canonical concept notes, proposing reusable claims with reasoning links,
   and recording the next uncovered step. Generated notes, quiz exposure and self-reported
   confidence must not automatically become mastery evidence.
5. Resume from the saved goal and next step, then test delayed application using a changed
   example. Record enough evidence to select later practice without retaining a full chat
   transcript. Corrections to concepts must also identify dependent claims needing review.

### Ownership and implementation boundary

The vault remains the canonical store for concepts, claim arguments, learning paths and durable
learning progress. Raw source captures remain comms-owned and are referenced rather than copied
as a second clipping inbox. Learning metadata must remain readable Markdown/frontmatter and
wikilinks under the vault's conventions; an implementation may rebuild a disposable index.
Exact fields and placement for review evidence require a vault-convention decision before code.

Keep pedagogy in `teach` and `practice`, and vault access in `obsidian-access`. A thin Pi
extension is justified only for behavior the skills cannot reliably supply: selecting a saved
path, resuming it, selecting review targets or presenting session controls. Command names and
schemas remain proposals. Do not introduce a separate graph database, learning application,
Obsidian plugin or Sjel capability for the pilot.

### Accepted product decisions

Decided 2026-11-03: all three recommendations accepted. These decisions govern the pilot and
any subsequent Pi learning extension. No product choices remain open from this planning round;
review-evidence fields and vault placement still require the convention check below.

| ID | Decision | Accepted behavior | Reason |
|---|---|---|---|
| L1 | Default entry | Topic or existing note; URLs optional | Build on the vault's knowledge without requiring a new source for every session. |
| L2 | Review initiation | Suggest a brief review when learning starts; no notifications | Support delayed practice within the learning workflow without a separate reminder system. |
| L3 | Note writes | Update concepts during teaching; review new claims at close | Keep note maintenance with the tutor while checking reusable propositions with the learner before saving them. |

### Vault convention reconciliation

Inspected 2026-11-03: the vault's `Knowledge/Meta/Vault-Contract.md` exists, but
`obsidian/scripts/vault contract` reports no configured contract. The Obsidian skill explicitly
permits following existing conventions in this case. Reading that note establishes conventions;
it does not mean the Pi extension's digest gate is enabled. Configuration of the optional gate
is separate deployment work, not a prerequisite for this read-only planning pass.

The current contract requires `type`, `maturity`, `summary` and `categories`, preserves a claim's
`concept` link and `## Because` structure, and permits a new subfolder only for three or more
related notes. No `type: claim` notes or `Claims/` paths were found. The current
`Resources/Bases/Knowledge.base` already has a Claims view filtered by `type == "claim"` and
`concept == this.file`; its filter does not require a particular folder. Its 90-day review
view measures note review age, not learner retention, and must not become a practice schedule.

Pilot placement follows that existing folder rule: keep the first one or two claims beside
their concept in its existing domain folder. Create a Claims subfolder only for a genuine
cluster of at least three, with link verification if notes move. `teach` (Phase 4, step 1) and `practice` follow this
convention since 2026-11-03.

The minimal claim shape is `type: claim`, an honest `maturity`, a self-contained proposition in
`summary`, existing domain `categories`, a `concept: "[[canonical concept]]"` link, source
provenance following adjacent notes, and `## Because` arguments. A concept's first claim adds
the existing Claims-view embed. Search paraphrases before creating any claim. Material quality
(`maturity` or legacy `status`) remains separate from evidence of learner understanding.

The inspected AI-Assisted Learning concept has legacy `status: draft` and no `maturity`.
The contract requires metadata correction on touch, so a teaching session that edits this note
must also reconcile its metadata. No notes or claims were created during reconciliation; a
valid learned claim must come from the pilot rather than a fabricated session record.

### Bounded next work

1. Apply the reconciled conventions to the teaching skill, then demonstrate them in the pilot.
   Done when one existing concept and one valid claim demonstrate placement, metadata, argument
   links and provenance without introducing a second owner. Convention inspection and the
   skill alignment are complete; the real-claim demonstration remains open.
2. Run the five teach/practice pairs already required by stage 1, including an existing-note
   topic and a source-led topic. Include a later-session transfer check and an interrupted
   session resumed from notes. Done when the log records all ten sessions and identifies
   observed friction rather than inferred feature needs.
3. Use that evidence to specify the smallest Pi extension, if needed. Done when each proposed
   control maps to observed friction and its acceptance check; persistence fields, concurrent
   edits, duplicate-note prevention and review selection are specified before implementation.

Verification for this proposal: repository skills, extension README, Learning Log and the saved
clip were read. No teaching pilot, Pi interaction, delayed-retention check or Obsidian rendering
check ran. `tools/doctor` reported six failing checks, including stale backups and a host-patch
audit finding; those are outside this planning change.
