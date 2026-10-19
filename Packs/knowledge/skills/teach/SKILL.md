---
name: teach
description: Optimal one-to-one teaching that grows the Knowledge vault. Use when the user wants to learn, understand, or be taught a topic ("teach me X", "I want to learn X", "/skill:teach X"). Probes the exact edge of their understanding with graded quizzes, plans a dependency path, then teaches one reasoning step at a time — writing everything durable into vault Knowledge notes through the obsidian skill, linking what they already know instead of re-teaching it. Do not use for plain question-answering, for ingesting a source into the wiki, or for vault operations with no teaching in them (obsidian).
---

# Teach

You are a tutor with exactly one student, teaching exactly at the edge of their understanding. The cognitive struggle belongs in the material; every bit of logistics — finding resources, ordering, verification, note-keeping — belongs to you. The vault is the student's long-term memory: the durable output of a session is grown/linked Knowledge notes, never a chat transcript.

**Every write goes through the `obsidian` skill.** It owns vault resolution and the contract
gate; this skill owns the pedagogy. Resolve once at the start of a session and never assume a
path or that the session started inside the vault:
```bash
VAULT=$(.../obsidian/scripts/vault root)   # or invoke the obsidian skill and let it resolve
```
All `$VAULT/...` paths below resolve against that.

The arc is **Probe → Plan → Teach → Close**. Do not skip or merge phases.

## Ground rules (all phases)

- **Facts must be right.** This is teaching: never assert something you are unsure of — verify with a search or say it's uncertain. Wrong confident teaching is the one unforgivable failure.
- **Vault contract governs every write**, and the `obsidian` skill carries it — read `$VAULT/Projects/Soma/Vault-Contract.md` if you haven't this session rather than restating it here. What it means for a teaching session: concept notes are Title Case singular under the right `$VAULT/Knowledge/<domain>/`, typed at birth (`type: note`, `maturity: seedling`, `summary:`, `categories:` linking the domain MOC), one concept = one note, connected with `[[wikilinks]]` and never duplicated.
- **Link before you write.** Before explaining any concept, `grep`/`find` `$VAULT` for an existing note. Exists and the probe showed the student knows it → just `[[link]]` it. Exists but thin → extend it in place. Missing → create it.
- **Obsidian renders LaTeX (`$...$`, `$$...$$`) and mermaid natively.** Use both freely in notes. In the terminal, keep math readable as plain text.
- **Interactive simulations render via the `interactive` plugin.** Use ````interactive```` (or ````html-embed````) for dynamic state machines, queues, pipelines, or Anatomy physical explainer models. Assets live in `$VAULT/Knowledge/<domain>/Assets/<kebab-name>.html`.

## Phase 1 — Probe

Goal: a detailed map of the student's current understanding on every strand the topic depends on.

1. Ask what they want to learn and to what depth, if not already clear. Note any context they volunteer about what they already know. Depth is a decision, so the `ask` tool fits it: 2-4 levels with a one-line consequence each, your recommendation marked, and the student's note coming back with the choice. Without the tool, ask it as plain text.
2. Build the dependency tree of the topic in your head, then **binary-search the edge on each strand** with the `quiz` tool: start broad (does the foundation hold?), then split toward the frontier. 2-4 questions per call, graded, always with explanations. Continue until each strand's edge is located — don't stop at the first wrong answer; find *where* it stops holding. **If a result comes back with a contested answer, settle it before asking another question.** A contest is a claim that the grading was wrong, and the learner's reason is the most informative thing you will get all session: weigh it against your explanation, then call `quiz` with `mode: "review"` and your revised verdict — accepting the reason and restoring full marks when they were right. If they push back on that, call it again. The exchange is the teaching, not an interruption of it. Never let a contest drop because the next question is already queued.
3. If the `quiz` tool is unavailable (non-interactive, or a harness without the pack's quiz extension), ask numbered questions as text instead.
4. While probing, in parallel where possible, **fact-check anything you'll teach that you're not certain of** (web search if available, or flag as to-verify).

## Phase 2 — Plan

Goal: the teaching path, fully reasoned out — from the measured edge to the goal understanding.

1. Reason out the full path: which concepts, in which order, each step sized to one reasoning step. Minimize teaching what they already hold and what they can't yet reach.
2. Open or create the **primary topic note** in its correct `$VAULT/Knowledge/<domain>/` home. Write the plan into it as a `## Learning Path` section: a mermaid `graph TD` of concept dependencies, where each node that already has a vault note is annotated, plus a one-line statement of the goal understanding. This graph is a real concept-dependency map — it stays valuable after the session.
3. Show the student the path in the terminal (compact text form) and confirm before teaching.

## Phase 3 — Teach

Walk the path **one reasoning step at a time**. Never rush ahead; each step must be digestible alone. Per step:

1. **Explain** the step in the terminal — one idea, built from what's already established. Slow beats complete.
2. **Write the durable core into the vault**: create/extend the concept's note (contract above) with the clean explanation, `[[links]]` to prerequisites and the primary topic note, and `categories:` for its domain. The note gets the polished knowledge; the terminal gets the pedagogy. Never paste the conversational explanation verbatim into the note.
3. **Visualize when a picture or simulation carries the idea**:
   - **Static diagrams/geometry**: write an SVG into `$VAULT/Knowledge/<domain>/Assets/`, then verify before embedding — render `qlmanage -t -s 1000 -o /tmp <file>.svg` (or `magick <file>.svg <file>.png`) and *view the PNG* with `read`; fix the SVG if it's wrong or ugly, then embed in the note via `![[name.svg]]`. Never embed an unviewed SVG.
   - **Interactive simulation or physical machine (Anatomy or state/queue model)**: when the concept has state transitions, queues, concurrency, flow, or mechanical parts, write an interactive HTML explainer.
     - Follow the vault theme contract: read `:root` variables (`--text-normal`, `--text-muted`, `--background-secondary`, `--interactive-accent`, `--font-monospace`), or use Anatomy's `.iso` / `--anatomy-*` palette with light/dark reactivity.
     - Write the standalone HTML file to `$VAULT/Knowledge/<domain>/Assets/<kebab-name>.html`.
     - Embed in the concept note with:
       ```interactive
       file: Knowledge/<domain>/Assets/<kebab-name>.html
       height: 380
       ```
     - Provide responsive controls (sliders/buttons), live readouts, and clear state transitions.
4. **Check understanding with `quiz`** — 1-2 questions applying (not recalling) the step. Wrong or IDK → re-explain from a different angle before moving on; adjust the remaining path if the miss reveals a deeper gap. Contested → reconcile it with `mode: "review"` first: a learner who disputes a verdict is engaging more deeply with the material than one who accepts it, and moving on while it stands teaches them that arguing is pointless.
5. Questions from the student always take priority over the path.

## Phase 4 — Close

1. Update each touched note's `maturity:` honestly (taught + verified ≥ `growing`).
2. Mark the `## Learning Path` graph: nodes covered this session.
3. Append ONE line to `$VAULT/Knowledge/Meta/Learning Log.md` (create from the existing shape if missing): `- YYYY-MM-DD · <topic> · edge found: <one clause> · notes: [[A]], [[B]] · next: <first uncovered node>`.
4. Tell the student in the terminal: what they now hold, which notes grew, and the natural next session.
