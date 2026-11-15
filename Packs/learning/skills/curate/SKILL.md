---
name: curate
description: Improve existing Obsidian knowledge notes by slimming repetition, connecting ideas, extracting reusable explanations and consolidating overlap. Use for a curation sitting on a note, topic cluster or learning path. Do not use to teach or assess understanding, run practice, bulk-rewrite the vault or automatically restore archived notes.
---

# Curate

Curate improves the existing knowledge surface. The agent reads the notes, identifies their
distinct contributions and edits the smallest useful scope. A cleaner note does not establish
learner understanding or stronger evidence for a claim.

## Resolve scope

Use the `obsidian` skill to resolve the vault and load its contract before any mutation.
Read `Resources/Vault Layout.json` when present; its paths own placement. In local vault mode,
use file tools. Otherwise use the governed Obsidian tools and their contract gate.
Use direct `read`, `edit` and `write` operations for routine note work. Machine policy can
automatically grant Markdown edits inside the configured vault. This is not a grant for compound
shell commands, arbitrary scripts or changes outside that root. Keep a read of the original
content for comparison; do not introduce temporary shell backups or mandatory prose-linter
commands for ordinary curation. Use an existing diff when available. If a genuine operation
needs permission, respect the gate and explain it; never retry a denied or deferred operation
through another tool. Technical permission prompts and content decisions are separate.

Start from the selected note and its directly linked active notes, including backlinks. A
topic request first resolves to one existing anchor note. If no unambiguous anchor exists,
ask for the note to curate. Do not create a topic just to obtain an anchor. Read related notes
only as needed to understand ownership and overlap. Do not traverse their links recursively
or turn the request into whole-vault cleanup. A source summary or exercise is a valid anchor.
Vault rules, session logs and `.archive/` are outside the editable scope.

Use the supplied desired outcome and time budget. Without a budget, perform one bounded pass
on the anchor and the directly related notes needed for it; ask only when scope is ambiguous.
Structural findings supplied by Learn are leads to re-check against current files, not authority
to move or rewrite notes.

## Inspect, propose, apply

1. Read the full anchor, relevant neighbors and sources already available in scope. Search
   titles, aliases and paraphrases across the active folders before creating an extracted note
   or selecting a canonical owner. Distinguish repetition from a different premise, use case,
   disagreement, example or uncertainty.
2. State the concrete improvement and affected notes. Apply clear wording reductions, metadata
   repairs and explanatory links within the requested scope directly. Starting Curate authorizes
   these routine edits; do not turn the sitting into an inspection-only proposal. Correcting stale
   framing, separating note quality from mastery, clarifying archived-source labels and linking
   an existing pending request are routine when the underlying facts and curriculum stay unchanged.
   A substantial rewrite changes the note's argument, coverage or ownership, not merely several
   sentences. For a merge, deletion,
   move, split or substantial rewrite, show the proposed owner, destination, preserved material
   and affected links before asking for approval. Existing approval applies only to its named
   changes; do not repeatedly ask for it. Prepare the full proposal before asking.
   In the Learn sidebar, ask necessary approval questions as plain text in this chat and wait
   for the reply. Do not call `ask`, queue a decision or send the owner to `/decisions`: RPC
   `ask` currently defers to the terminal queue. Show the exact proposal followed by one direct
   approval question. Continue independently authorized edits while approval is pending. A pending
   proposal must not block routine corrections elsewhere in the requested scope.
3. Edit only the approved scope. Re-read each file immediately before changing it and preserve
   concurrent edits. Remove repetition and stale scaffolding, while retaining every distinct
   idea, caveat, counterexample, attribution and source. A shorter note is useful only if the
   reasoning remains reconstructable. Leave useful material alone when there is no improvement.
4. Link where the relationship helps reasoning: prerequisite, explanation, implication,
   application or disagreement. Prefer a sentence explaining the relationship over an unrelated
   links list. Do not pursue a link count. Several learning paths share one canonical concept.
5. Extract source-independent explanations into Knowledge and keep source-specific summaries
   in Resources. Keep provenance attached to the statements it supports. Split only when the
   ideas can stand on their own; preserve discoverability from the original note. Use existing
   topic maps before making another and follow the contract's folder threshold.

Do not silently resolve factual conflicts. Preserve the competing statements and their sources,
or verify the disputed point before changing it. Do not upgrade `status: verified` or `maturity`
just because prose improved. Do not create taught claims, rewrite `## Because` arguments in a
way that changes their meaning, or change learning-path node states. Preserve existing progress
when repairing links or restructuring a path. Do not manufacture a learning session log entry.

For approved moves, use Obsidian's link-aware file manager when available. If only local file
tools are available, map incoming links, relative links and figure paths first, update them
together, and verify the result. Never overwrite an existing destination. Merges preserve all
distinct content in the canonical owner and retain an alias or redirect when needed. Use a
recoverable removal mechanism; otherwise leave the superseded note and report pending removal.

## Close

An explicit stop ends editing without another approval round. Save only already-authorized work.

- Re-read the edited files. Compare before and after for lost qualifications, source mappings,
  changed arguments and accidental progress changes. Check required metadata, valid frontmatter
  at the beginning of each note, placement, resolved links, headings and figure paths.
- Re-run the scoped structural checks against current files. The Learn command **Check vault
  structure** checks placement, metadata and unresolved wikilinks; it does not assess meaning.
  Without the command, perform those checks with the available file or Obsidian tools.
- Report the changed files and why, unresolved conflicts or proposals, and checks that actually
  ran. Mark UI rendering as unverified when it was not inspected. Store a durable pending issue
  beside its relevant content only when it is useful; do not create an administration note or
  duplicate session transcript. Never report an unapproved proposal as applied.
- A failed required write or integrity check prevents a successful close. Report the exact
  remaining problem. A deliberate no-op can close successfully when nothing needed changing.
