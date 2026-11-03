# vault

Reads an Obsidian vault as data. Five CLI verbs and one HTTP surface, all
read-only.

```
vault links  [--root PATH] [--json] [--dead] [--inbound FOLDER]
vault lint   [--root PATH] [--json] [--carrying KEY]
vault class  [--root PATH] [--json] [--only c2] [--list]
vault classify --id VAULT/RELATIVE.md [--declared c1] [--json]
vault bases  [--root PATH] [--json] [--strict]
vault compare --root PATH --against PATH [--json] [--list]
```

The root is a personal fact and never lives in this repo. It comes from the
overlay's `config/knowledge.toml` (`vault_root = "..."`), or from `--root`. The
server takes the same path from the same file and has no `--root`: a service
resolving its own root from an argument would be a second declaration of where
the vault is.

Link resolution compares names in Unicode NFC form. Canonically equivalent names
that identify multiple files remain ambiguous.

`compare` counts newly unresolved link occurrences in unchanged notes shared by two vaults.
It reports counts only, excludes changed or ambiguous note paths, and refuses unreadable notes.
The excluded destination-note count makes the comparison's coverage explicit.
`--list` includes the source path and target for each occurrence; these may be private.

## `class` — which notes hold whose facts

PRD **Q9a** (2026-08-23): the folder sets the default, a note's frontmatter
`class:` key overrides it in either direction. `Atlas/People/`,
`Atlas/Documents/` and `Atlas/Finance/` are **C2 Others**; a health folder is
C2 wherever it sits; everything else is **C1 Mine**.

The rule is not here. `content_item::DataClass::classify_vault_note` holds it,
beside the mail rules and the C0–C3 vocabulary they share, because PRD §6.1
forbids a second definition of what `c2` means by name. This verb is the walk
and the report. `vault classify` calls that same function for one vault-relative
path without loading the vault. Governed clients use this narrow command instead
of maintaining their own copy of the folder rules.

**Measured against this vault, 2026-09-07** — the acceptance figures, in the
same spirit as the link counts below:

| Class | Notes | From |
|---|---|---|
| C0 Public | 0 | Unreachable from a location. Publishing is an act (§15), not a folder |
| C1 Mine | 2,587 | Everything Q9a's table does not name |
| C2 Others | 170 | `Atlas/People` 89 · `Atlas/Documents` 74 · `Atlas/Finance` 7 |
| C3 Secret | 0 | Only reachable by declaring it |

Two things the measurement says that the ruling could not. **No note in this
vault carries a `class:` key**, so every one of the 2,757 rows above is a folder
default and the override path has no production evidence yet — it is tested, not
exercised. And **the health rule fires on nothing**: the only health folder here
is `Atlas/Documents/Gesundheit/`, which the `Atlas/Documents` rule already claims
one line earlier. It stays because Q9a names health as a rule rather than a
folder, and the day that folder moves out of `Atlas/Documents/` is the day it
starts earning its place.

Three lists, not one total: `--list` prints every note with its class,
`--only c2` prints one class with the reason each note landed there, and the
default report names the **refused declarations** — notes whose frontmatter set
a class outside the vocabulary. A refusal is the interesting row. The folder
default answers for it, so nothing fails; what it means is that somebody
believes that note is classified and it is not.

## `bases` — the Bases, checked against the vault they query

PRD **D5** says the Bases are unverified in Obsidian and that a CLI cannot
confirm Base *rendering*. Both are still true. What a CLI can confirm is what a
Base states about the vault before rendering starts: a Base is a query, it names
folders and it names the frontmatter keys it will draw as columns, and each of
those is checkable against the notes on disk.

**Measured 2026-09-08** — 28 Bases, 37 folder references:

| | |
|---|---|
| Folder references naming a folder that holds no note | **11**, across 11 Bases and 10 distinct folders |
| Declared columns no note in scope carries | **106** |

Those are two different failures and both look identical in Obsidian. A Base
whose folder moved renders an empty table; a Base whose folder is fine and whose
`maturity:` became `status:` renders a table of blank columns. Neither throws.

A missing folder gets **candidates, never a rewrite**. `.base` files live in the
vault and §5.5 is one-way, so this verb names where the folder probably went and
stops. The rule is narrow — a folder elsewhere in the vault with the same final
segment, holding at least one note — and each candidate is weighed by how many of
that Base's own declared columns its notes carry, because a matching name is not
a destination:

| Base | Names | Candidate | Columns it fills |
|---|---|---|---|
| `Focus.base` | `TELOS/Focus` | `Atlas/Focus` | 3 of 3 |
| `Reflections.base` | `TELOS/Reflections` | `Atlas/Reflections` | 4 of 5 |
| `Soma.base` | `Projects/Soma/Domains` | `Projects/Sjel/Knowledge-Base/Domains` | 4 of 4 |
| `Investments.base` | `Atlas/Finance/Investments` | `Projects/Archive/Ledger/Notability/Investments` | **0 of 8** |
| `Tasks.base`, `Calendar.base` | `Projects/Tasks` | nine of them | 7 of 10 at best |

Four references have exactly one candidate and only three of them survive the
column check. The fourth is an archived Notability import that shares a word.
`Projects/Tasks` is the opposite shape: every `Tasks/` folder now sits under a
project, so nine candidates is the correct answer and none of them is a
proposal.

### Every unresolved folder carries a verdict

"MISSING" is one word for six different repairs, so each unresolved reference is
diagnosed and the verdict is printed above its candidates:

| Verdict | Means |
|---|---|
| `moved` | one namesake carries this Base's columns |
| `spread` | every namesake sits somewhere under the folder's own parent |
| `never created` | not on disk, and the parent holds notes |
| `empty` | on disk and holds no note |
| `moved, destination unclear` | several namesakes carry the columns |
| `gone with its parent` | no ancestor holds a note either |

`spread` is `Projects/Tasks`, in `Tasks.base` and `Calendar.base`. **Q104
(2026-09-09) rules that `Projects/Tasks/` is never created and a task lives under
the project that owns it**, so the repair is a filter matching the shape rather
than a path naming the folder. The verdict is structural, not a special case:
any missing folder whose namesakes all sit under its own parent reads the same
way.

`never created` is a claim about the disk and about the note index, never about
history — this verb does not read the vault's git repository. The one it needs a
directory listing for is `empty`, which is why the walk happens: a folder that is
there and holds no note is a different job from a folder that is not there, and
the note index alone cannot tell them apart.

`--strict` exits non-zero when a positive folder reference resolves to nothing.
Without it the verb answers `0` for a vault where every Base is broken and `0`
for one where none is, which is an instrument that cannot be wrong.

## The server

`vault-server` on `8094`, loopback. Three routes:

| Route | Answers |
|---|---|
| `GET /health` | Liveness. A literal — it cannot see the vault. |
| `GET /ready` | Readiness: the vault root resolves. |
| `GET /routes` | This manifest, as data. |

`GET /api/tasks` served band 620 from 2026-08-27 (PRD Q48) to 2026-10-18. It
retired when `capabilities/projects` became the owner of tasks and the vault's
`Projects/` moved into Sjel (capabilities/projects/README.md).

`GET /api/people` and `GET /api/journal-health` served what the Journal knew about
each person and which Journal health keys had a producer. They retired on
2026-10-20 with the CLI's `people`, `journal`, `names` and `fields` verbs, when
`Atlas/People` and `Journal/` left the vault. `capabilities/entities` computes
`last_contact` from the Journal in `projects` now (`src/journal.rs`). Git history
holds the rules those modules recorded (Q106, D2).

**Nothing in this crate writes, by design.** A write route would make the server a
second writer of files a human is editing in Obsidian.

## Why a binary

Every vault operation worth doing starts by asking the same two questions: what
is in here, and what links to what. A skill that describes how to answer them
answers differently each run. A binary with tests answers the same way twice,
which is the only reason a migration can be gated on it.

## The acceptance check is a second instrument, not a saved number

There used to be a table here of seven counts measured by `find` and `rg` before
this crate existed, called "the fixture", with the tool's answer beside each. It
was retired on **2026-09-08** because by then it disagreed with the vault on
every line — 1,138 notes under `Knowledge/` against 1,141, 14 ambiguous basenames
against 89, 18,332 wikilinks against 22,344 — and one of its rows had stopped
describing the vault entirely: **0 notes carry a `knowledge:` key today, where
the fixture recorded 996.** The key is gone; the notes carry `type:` and
`status:` now.

None of that was the tool drifting. The vault gained 509 notes and lost a
frontmatter key, and a saved count over a hand-edited vault cannot survive that.
**A fixture that can only ever be wrong is a test that cannot fail** — every
mismatch reads as "the vault moved again", so nothing in it is falsifiable.

What replaces it is the property the old table actually had, kept live: **two
implementations that share no code, run against the same vault at the same
moment, and are made to agree or made to explain.** The second one is
`acceptance/link-counts.py` — its own walk, its own `[[([^]\n]+)]]` pattern, its
own three rungs, deliberately not the crate's. Run both and compare:

```
python3 capabilities/vault/acceptance/link-counts.py <vault-root>
vault links --root <vault-root>
```

Run 2026-09-08 against 2,757 notes:

| Measure | Independent probe | `vault` | |
|---|---|---|---|
| Notes | 2,757 | 2,757 | exact |
| Notes under `Knowledge/` | 1,141 | 1,141 | exact |
| Path-form wikilinks | 5,173 | 5,173 | exact |
| Path-form wikilinks that are dead | 406 | 406 | exact |
| Ambiguous basenames | 89 | 89 | exact |
| Notes in `Knowledge/` linked from outside | 134 | 134 | exact |
| Wikilinks total | 22,354 | 22,344 | probe is wrong, −10 |
| Dead wikilinks | 5,194 | 5,184 | the same 10 |

Three rows moved because writing the probe found real defects, and each is worth
more than the row it fixed:

- **17 links that were never links.** The probe could not see
  `np.array([[1, 2], [3, 4]])` in a fenced block or
  `<% tp.date.now('yyyy-[W]ww') %>` in a Templater expression; the crate counted
  both, and then counted them dead. Obsidian forbids `[` and `]` in a note name,
  so `targets_in` now refuses a span that holds one. The crate lost 17 links and
  17 dead links.
- **`inbound` matched on names, not on links.** It answered 139 for
  `Knowledge/`; five of those were links that share a name with a note in
  `Knowledge/` and open a different one. It resolves through the same ladder
  `report` uses now, and both instruments say 134.
- **`inbound` could not see a nested folder at all.** It compared the first path
  segment, so `--inbound Atlas/People` answered `0 distinct notes` — as a fact,
  not as an error. It answers 71.

The remaining 10-link gap is the probe's, and it is left standing rather than
tuned away: all ten are `[[[…]]]`, six of them the empty `- "[[[]]]"` that a
broken template wrote into frontmatter and four inside Mermaid nodes
(`M[[[Net Present Value Method]]]`), where nothing renders as a link anyway. The
regex can re-anchor inside the triple bracket and the crate's left-to-right scan
cannot. **Where the two disagree the reason gets written down and the loser gets
named. A number quietly adjusted to match is not a check.**

The falsifiable half of this crate is its 70 tests, not this table — 60 in the
library and 10 over the server's handlers. Each one plants an input the code must
reject and watches it get rejected.

## What the counts found that a note count could not

**10,995 of the vault's 22,344 wikilinks live in frontmatter**, not in prose — in
`categories:`, `related:` and `sources:`. That is the membership graph every MOC
is fed by and every provenance edge the knowledge model rests on. This crate was
written body-only first and reported a vault 60% smaller than it is. The two
counts stay separate because they break differently: a folder move rewrites a
prose link, an editor rewrites a `categories:` entry, and one number cannot tell
you which repair you owe.

## Dialect drift

`lint` scans the raw frontmatter rather than the parsed map, because the drift
this vault has is invisible after parsing: `knowledge: reference` and
`knowledge: "reference"` are the same value and two different conventions, and
`maturity: evergreen` versus `maturity: 🌲` is why every Base in the vault
carries a hand-written compatibility shim.

## Related

- `libs/markdown-root` — containment-checked vault access, recursive walk, the
  byte-addressable frontmatter this crate reads, and the wikilink parser the link
  checker shares with `capabilities/entities`.
