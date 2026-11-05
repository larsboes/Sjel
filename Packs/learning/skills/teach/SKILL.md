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
- **The vault contract, when one is configured, governs every write**, and the `obsidian` skill carries the procedure. In Pi, call `obsidian_contract` before the first mutation; it loads the contract configured for this machine, or reports that none is configured, in which case follow the vault's existing conventions. For a teaching session: concept notes are Title Case singular under the right `Knowledge/<domain>/`, typed at birth according to the contract or those conventions, one concept = one note, connected with `[[wikilinks]]` and never duplicated.
- **The student's thinking is the primary data.** Ask them to think out loud with every answer — typed or dictated, unfiltered — and to give a confidence (0–100 %). Read all of it. One answer is one bit; the reasoning shows the model behind it. A right answer on a confused model is a warning, a wrong answer on sound reasoning is progress. Keep your own turns short so their thinking arrives often.
- **Link before you write.** Before explaining any concept, `grep`/`find` `$VAULT` for an existing note. Exists and the probe showed the student knows it → just `[[link]]` it. Exists but thin → extend it in place. Missing → create it. Search `$VAULT/Knowledge/` only. `$VAULT/.archive/` holds unverified notes from before the restart: read a matching archive note as source material to check, never link it and never count it as known.
- **Math renders in both places.** Obsidian renders LaTeX (`$...$`, `$$...$$`) and mermaid natively. Pi's TUI renders a LaTeX subset as Unicode in messages and in `quiz` text. Follow "Math in the terminal" in `references/explanations.md`, and give every formula a plain-words reading.
- **Explanations follow `references/explanations.md`.** It sets the shape of one step and what moves from the terminal into the note. The vault contract sets the note anatomy.
- **Figures follow `references/figures.md`.** It picks the form (SVG, `interactive` block or an Anatomy figure in `html-embed`), the rules every figure follows, and the check before embedding.

## Phase 1 — Probe

Goal: a detailed map of the student's current understanding on every strand the topic depends on.

1. Ask what they want to learn and to what depth, if not already clear. Note any context they volunteer about what they already know. Depth is a decision, so the `ask` tool fits it: 2-4 levels with a one-line consequence each, your recommendation marked, and the student's note coming back with the choice. Without the tool, ask it as plain text.
2. Build the dependency tree of the topic in your head, then **binary-search the edge on each strand** with the `quiz` tool (`purpose: "probe"`): start broad (does the foundation hold?), then split toward the frontier. 2-4 questions per call, graded, always with explanations. Continue until each strand's edge is located — don't stop at the first wrong answer; find *where* it stops holding. **If a result comes back with a contested answer, settle it before asking another question.** A contest is a claim that the grading was wrong, and the learner's reason is the most informative thing you will get all session: weigh it against your explanation, then call `quiz` with `mode: "review"` and your revised verdict — accepting the reason and restoring full marks when they were right. If they push back on that, call it again. The exchange is the teaching, not an interruption of it. Never let a contest drop because the next question is already queued.
3. Pick the question tool by harness:
   - **pi:** `quiz`. The student types their thinking (`t`) and sets a confidence (`←→`) before answering. The result reports both per answer.
   - **Claude Code:** `AskUserQuestion`, which does not grade. Per quiz item, send two questions in one round: the item (at most 3 options plus "I don't know") and "Confidence?" (`<25 %`, `~50 %`, `~75 %`, `>90 %`). Tell the student once per session to type their thinking into the note of the option they pick. After the round, grade each item in your reply with its explanation. A contest arrives as prose; settle it in prose before the next question.
   - **Neither (non-interactive):** ask numbered questions as text and ask for the thinking and a confidence with each answer.
4. While probing, in parallel where possible, **fact-check anything you'll teach that you're not certain of** (web search if available, or flag as to-verify).

## Phase 2 — Plan

Goal: the teaching path, fully reasoned out — from the measured edge to the goal understanding.

1. Reason out the full path: which concepts, in which order, each step sized to one reasoning step. Minimize teaching what they already hold and what they can't yet reach.
2. Open or create the **learning path note** `$VAULT/Resources/Learning/<Topic> — Learning Path.md` (the vault contract places plans outside `Knowledge/`). Write the plan into it as a `## Learning Path` section that follows the vault contract's learning-path schema: one mermaid `graph TD` of concept dependencies, each node's state as a class (`held`, `current`, `open`), plus a one-line statement of the goal understanding. On resume, read the graph first: teaching continues at the `current` node, else at the first `open` node whose prerequisites are `held`. This graph is a real concept-dependency map — it stays valuable after the session.
3. Show the student the path in the terminal (compact text form) and confirm before teaching.

## Phase 3 — Teach

Walk the path **one reasoning step at a time**. Never rush ahead; each step must be digestible alone. Per step:

0. **Retrieve** — from the second step on, open with one question that makes the student produce an earlier step in their own words or apply it to a new case. Use the question tool. A miss here changes the plan before new material arrives.
1. **Predict** — before explaining, point at the problem the current state cannot handle and ask the student what has to come next, with `quiz` and `purpose: "predict"`. It need not be answerable; the attempt is the point. Skip it on wrap-up or pure repair turns, never by habit.
2. **Explain** the step in the terminal — one idea, built from what's already established. Open by resolving the prediction: what they guessed, what is true, and why. Slow beats complete.
3. **Write the durable core into the vault**: create/extend the concept's note (contract above) with the clean explanation, `[[links]]` to prerequisites and the learning path note, and `categories:` for its domain. The note gets the polished knowledge; the terminal gets the pedagogy. Never paste the conversational explanation verbatim into the note.
4. **Visualize when a picture or simulation carries the idea.** Read `references/figures.md` first. Write the asset into the concept note's folder under `Assets/`, as the vault contract places it, render it and read the PNG, then embed it in the concept note. Never embed a figure nobody has looked at.
5. **Check understanding with the question tool from Phase 1** (`purpose: "check"`) — 1-2 questions applying (not recalling) the step, answerable by reasoning from the session alone. Write each wrong option as the answer a specific wrong model gives, so a miss names the misconception. Prefer questions that ask which observation separates two explanations.
6. **Respond to the reasoning.** Give the verdict, quote the part of the student's thinking that was right or wrong, and name the one misconception in a sentence. Then adapt by correctness and confidence:

   | Result | Next move |
   |---|---|
   | Right, high confidence | Move on. Consider skipping a planned step. |
   | Right, low confidence | One more variant on a new surface. |
   | Wrong or IDK, low confidence | Re-explain from a different angle, then check again. |
   | Wrong, high confidence | A misconception: show a contrast case or the step's figure, ask the prediction again, and revise the remaining path. |

   Contested → reconcile it with `mode: "review"` first: a learner who disputes a verdict is engaging more deeply with the material than one who accepts it, and moving on while it stands teaches them that arguing is pointless.
7. Questions from the student always take priority over the path.

## Phase 4 — Close

1. **Extract claims.** Distil what was taught into claim notes: one true, self-contained proposition per note. Place the first one or two claims of a concept beside that concept note, in its domain folder. Create `Knowledge/<domain>/Claims/` only when a cluster reaches three related claims, and check every link after a move. Frontmatter: `type: claim`, an honest `maturity`, the proposition in `summary`, the domain's existing `categories`, `concept: "[[<concept note>]]"`, and source provenance in the form adjacent notes use. Each `## Because` bullet is one argument whose premises jointly imply the claim; independent arguments are separate bullets. Before creating, search every vault basename and the domain's claims: a paraphrase of an existing claim links to it instead. Premises may be claims from earlier sessions. Show the student the list before writing. A concept note that gets its first claim also gets a `## Claims` section that embeds `![[Knowledge.base#🧩 Claims]]`. `maturity` rates the note, not the student: a claim the student answered correctly once is still not evidence of mastery.
2. Update each touched note's `maturity:` honestly (taught + verified ≥ `growing`).
3. Update the `## Learning Path` graph: `held` only for nodes that passed the gate this session, `current` for the node the next session resumes at. A node that was taught but did not pass stays `current`.
4. Append ONE line to `$VAULT/Resources/Learning Log.md` (create from the existing shape if missing): `- YYYY-MM-DD · <topic> · edge found: <one clause> · notes: [[A]], [[B]] · claims: <count> · next: <first uncovered node>`.
5. Tell the student in the terminal: what they now hold, which notes grew, and the natural next session — a `practice` session on the new claims, or the next teaching node.
