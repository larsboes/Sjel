# Document shape

How a crystallized document is built so it stays true after you leave.

The test for every convention here: **can the next reader tell what was decided, when, why, and
what is still open — without asking anyone?** If not, the convention is decoration.

## The decision record

Every answered question becomes a record *where it applies*, not in a changelog at the end. A
changelog separates a decision from the thing it decided, and the reader who needs it is reading
the thing.

```markdown
> [!done] Q9b — answered 2026-08-23: **rungs 0 and 1 in v1, and every reduced call leaves a
> visible receipt.**
>
> v1 ships the known-entity registry and the pattern rules. NER and local-model reduction are
> named, deferred, and not pretended to exist.
>
> The receipt appears on **every** reduced call, in the answer itself. It costs almost nothing and
> it turns the guarantee from a promise into something visible while you work. A silent gate is
> indistinguishable from a gate that is switched off.
```

Four elements, each load-bearing:

- **The id** — so the index can reference it and integrity can be checked.
- **The date** — a decision without one cannot be re-opened intelligently. "We decided this before
  we knew X" is only answerable with dates.
- **The ruling in bold** — skimmable. Most readers want only this.
- **The reasoning** — the next reader is usually the same person, having forgotten. The
  *because* is what stops the decision being undone by accident.

Add the consequence when there is one: what this closes, breaks, or obliges elsewhere.

## Superseding, not deleting

When new evidence overturns a decision, keep the old one visible:

```markdown
> [!done] Q26 — answered 2026-08-23: **the rejection is withdrawn. It answered a different
> question.**
> The earlier ruling assumed a guessing redactor. The shipped one is deterministic. The objection
> still stands on its own merits, but it now argues against a working mechanism rather than a
> hypothetical one.
```

A reader who finds only the new decision will re-derive the old objection and wonder why nobody
considered it. Showing the reversal *and its cause* is what stops the loop.

## The open-question index

One place, numbered, with what each blocks:

```markdown
| # | Question | Blocks | Why it is still open |
|---|---|---|---|
| Q31 | Where do projected files live? | §5.5 | Deferred with the pattern itself |
```

**Derive the counts, never type them.** A hand-typed "22 of 26 settled" is wrong within two edits,
and a document that miscounts its own questions has stopped being trustworthy in a way readers
notice. `scripts/integrity.py` reads the ids out of the body.

## Marking what you could not verify

Unverified claims get an explicit table, not a hedge buried in prose:

```markdown
**Unsourced, and marked as such in the text**

| Claim | Where | Status |
|---|---|---|
| Competitor X protects data only with local models | §1 · §6 · §11 | **My impression.** No entry in the manifest. Tracked as Q25 |
| "roughly two weeks of work" | §6.3 | My estimate. Not measured |
```

In a document that is otherwise rigorous, an unmarked guess borrows credibility from everything
around it. The table is how you stop that.

## The supersession table

When a document is meant to replace others, say exactly which and what happens to each:

```markdown
| File | Fate | Why |
|---|---|---|
| `Projects/Soma/Soma.md` | Fold into §5, then archive | The boundary is one section here, not a parallel project |
| `README.md` | Stays, becomes a projection | Public-safe subset. May not state a rule this document does not |
```

Written early, this prevents the most common failure of a consolidating document: becoming the
*n+1*th source of truth instead of the first.

## Sources

A document making claims about a system needs a section naming where each claim can be re-checked,
with line numbers where they exist:

```markdown
| Source | Cited for |
|---|---|
| `libs/content-item/src/lib.rs` L318, L405, L499 | The shipped class vocabulary, escalation rule, processing policy |
| `crates/comms/src/store/migrations.rs` L279–290 | The egress ledger |
```

This is what makes the document maintainable by someone else — and by you, in six months, when the
code has moved.

## Watch the balance

A document that is 700 lines of *how it is built* and 48 lines of *what it does* is describing
plumbing, not a product. Measure the sections; the imbalance is invisible while writing and
obvious in a table:

```bash
python3 scripts/integrity.py <doc> --sections
```

The fix is usually not trimming the long parts. It is that the thin sections were never
interrogated, because plumbing questions are easier to ask than purpose questions.

## The handover

When the questions run out, separate two things the next reader will otherwise conflate:

```markdown
### Next session — the open points
Every question in this document is answered. What follows is **work**, not undecided design, and
it is ordered so the first item is the one that should not be done tired.

| # | Item | Why it is sized this way |
|---|---|---|
| B1 | The class migration | Touches two libs, a live CHECK constraint, and a ratified value. **Its own session** |
```

Then, separately, carried defects — each with its known cause and the decision it needs. A defect
with a cause is a task; a defect without one is a mystery, and mysteries do not get picked up.

## Formatting that survives

- **Callouts for decisions** (`> [!done]`, `> [!question]`, `> [!warning]`) — greppable, and they
  render as distinct blocks in most markdown tools.
- **Tables for anything with more than two dimensions.** Prose comparing four options across three
  axes is unreadable; the table is scannable.
- **Code blocks for anything with shape** — a folder tree, a config, a data flow. Same argument as
  previews in a question round.
- **One idea per sentence**, active voice, no semicolons. If a rewriting standard is available in
  the environment, apply it to the finished document — a decision document is read under time
  pressure by people looking for one specific thing.
