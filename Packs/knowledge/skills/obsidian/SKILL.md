---
name: obsidian
description: Operates the user's Obsidian vault as the single durable knowledge surface — resolves the vault, gates every write on the vault contract, creates and retypes notes, runs bulk property/link sweeps, maintains the wiki layer, and recalls what earlier sessions said. Use for any work inside the vault (notes, frontmatter, MOCs, bases, canvases, clippings, session and planning notes), for "what do we know about X", and for "what did we do last week / where did we leave off". Do not use for generic markdown outside the vault, for prose style (human-writing), or for authoring skills (skill-creator).
allowed-tools: Read, Write, Edit, Bash, Glob, Grep
---

# Obsidian

The vault is an Obsidian **Life OS, not a code repo**, and it is the one place knowledge is
allowed to be durable. Anything worth keeping ends up here as a typed note — not in a session
transcript, not in a scratch file, not in a second system beside the vault.

## 1 · Resolve and gate (always first, never hardcode)

```bash
VAULT=$(scripts/vault root)          # from the overlay's config/knowledge.toml
scripts/vault contract               # fails loudly if the contract is missing
```

Read `$VAULT/Projects/Sjel/Knowledge-Base/Vault-Contract.md` **before creating, moving, or retyping
any note.** It owns placement, the no-doubling law, naming, and frontmatter typing. If it is
missing, stop and say so — do not improvise structure. There is no vault-root `AGENTS.md` pointer
today; the contract path above is the pointer (verified absent 2026-09-09).

Three rules from it that gate every single write, restated because they are what goes wrong:

- **Type at birth.** No note without `type:` + `maturity: seedling` + `summary:` + `categories:`.
- **One owner per kind of statement.** A concept is Knowledge, a proper-noun instance is Atlas, a
  consumed work is `Atlas/Media` (ideas graduate to a Knowledge concept), what-happened is
  `Atlas/Events` + Journal, a pattern is `Atlas/Reflections`, a decision is the Project.
  Assert once; thread everywhere with `[[links]]`.
- **Update the owner in place.** A new note must earn its folder; a new folder needs 3+ notes.

## 2 · Read before you act

Almost everything already has a home. Grep the vault and open the owning note before answering a
planning or "update my X" request — the job is to find where it lives, not to invent a new place.
A note with no inbound or outbound link is a leak.

## 3 · Pick the reference

| Task | Read |
|---|---|
| Wikilinks, embeds, callouts, properties | `references/markdown.md` |
| `.base` views, filters, formulas | `references/bases.md` |
| `.canvas` files | `references/canvas.md` |
| Obsidian CLI (notes, search, properties, plugin dev) | `references/cli.md`, then `obsidian help` — authoritative |
| Ingest a source · synthesize across notes · lint the wiki | `references/wiki.md` |

**28 bases already exist** in `$VAULT/Resources/Bases/` (Tasks · Projects · Sessions · Knowledge ·
People · Media · Journal · Focus · …). Query or embed one before building a
new view — most "show me X" needs are already a view away. Never write base or canvas syntax from
memory.

## 4 · Operations

**Session and planning notes.** Ratified in `$VAULT/Projects/Sjel/Knowledge-Base/Vault-Planning-Convention.md`:
a session note goes into the project it serves, `Projects/<Name>/Sessions/YYYY-MM-DD <Title>.md`;
no fitting project → `Resources/Inbox/`. Frontmatter `type: session-isa | session-plan |
project-isa`, `phase:`, `progress: M/N`, `started:`, `principal_stated_goal:` verbatim. Body: H1 ·
`## Claims` (`- [ ] C1 — claim. Falsifier: …`) · `## Anti-claims` · `## Log`. Project-lifetime ISAs
are all-vault at `Projects/<Name>/ISA.md`; repos keep a pointer file naming the vault path.

> The convention's guardrail: **notes are created only on explicit decision — no automatic work
> capture.** Do not add a hook that stages sessions into the vault; the transcripts already hold
> that, and the Inbox is contracted to drain to zero weekly.

```bash
scripts/vault sessions --phase climbing    # what is still open, across all projects
```

**Recall — what did we do / what do we know.** The transcripts carry `cwd`, `gitBranch`,
`timestamp`, `aiTitle` and every prompt, so this is a search over what exists, not a second copy:

```bash
scripts/vault recall "sparpreis" --limit 10       # ranked by how much it was actually discussed
scripts/vault recall --cwd Projects/VBB --since 2026-08-01
```

Vault-side recall is a plain grep over `$VAULT` plus the `Knowledge.base` view — the notes are the
answer to "what do we know", the transcripts to "what did we do".

**Harvest.** When a session produced something durable, promote it *now*, by hand, into the owner
note per §1. Capture surfaces and their contracted "empty" (§5 of the contract): `Resources/Inbox/`
drains to zero weekly · root `Scratchpad.md` is never empty and is triaged weekly · daily notes are
an immutable log. Brainstorming belongs in Scratchpad and graduates from there.

## 5 · Gotchas (verified on this setup)

- **Moves are `git mv`**, then rewrite path-style `[[links]]` and verify each resolves. Bare-name
  links survive renames only while the filename stays unique vault-wide.
- **Base and canvas rendering cannot be verified from the CLI** — flag such output as
  verify-in-Obsidian, never assert that it renders.
- **The vault is iCloud-synced with its git dir outside the tree** (`.git` is a pointer file).
  Normal git commands work from inside it; never re-init or nest a repo.
- **`maturity` on legacy notes is not trustworthy** — a 2025 bulk import mislabeled ~895 notes
  `evergreen`. Fix on touch, forward-only, never bulk-rewrite.
- **Secrets never land in a note.** The vault is git-synced; `Atlas/Documents/` and
  `Atlas/Media/Assets/` are gitignored under named rulings, not as a convenience.
- **`python3` here is 3.9** — no `tomllib`. Use `scripts/vault root`, not an inline TOML parse.

## 6 · Known stale spots in the vault

Report these when touched; do not silently work around them.

- `Knowledge/llm-wiki/` is a two-file stub (`index.md`, `log.md`) from an abandoned parallel wiki
  layout, sitting as a sibling of the nine real domains. It has no owner and violates §2.
- `Projects/Archive/LifeOS/` holds 13 session notes under a project name that outlived the system;
  the planning convention it once owned moved to `Projects/Sjel/Knowledge-Base/` with the 2026-09-07
  ruling (contract §2 conv. 6).
- The Vault-Planning Convention's "Registry pointer" section points at `MEMORY/STATE/work.json`,
  which retired with LifeOS.
