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
Anatomy is installed globally (`~/.agents/skills/anatomy`), so pi loads it from the vault.

Open:

1. Run five real `teach` sessions, each followed by `practice`. Done when
   `Knowledge/Meta/Learning Log.md` has five teach lines and five practice lines.

Not checked: no figure built by `figures.md` has rendered in Obsidian yet, and no Anatomy
page has been checked inside `html-embed`. Also not checked: the `🧩 Claims` view renders only in Obsidian, and no one has looked at it yet. Also not checked: no session has run on the new skill text yet.

## 2. Stage 2: practice on the phone

Open. Decide after stage 1 between Obsidian mobile with a due-for-practice Base view, an
Obsidian plugin, and the Sjel agent on the phone ([`agent.md`](agent.md)). Done when one
practice session runs on the iPhone against vault claims.

## 3. Stage 3: a Sjel learning surface

Not started. Only if stages 1 and 2 show friction that a UI removes. It reads and writes the
same claim notes, so no data migrates.

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

Inspected 2026-11-03: the vault's `Knowledge/Meta/Vault/Vault-Contract.md` exists, but
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
cluster of at least three, with link verification if notes move. Before using the pilot in Pi,
align `teach`'s unconditional Claims-folder instruction with this convention. This is an open
skill change; the plan alone does not change deployed behavior.

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
   links and provenance without introducing a second owner. Convention inspection is complete;
   the skill alignment and real-claim demonstration remain open.
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
