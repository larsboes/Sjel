# vault

Reads an Obsidian vault as data. Seven CLI verbs and one HTTP surface, all
read-only.

```
vault links  [--root PATH] [--json] [--dead] [--inbound FOLDER]
vault lint   [--root PATH] [--json] [--carrying KEY]
vault names  [--root PATH] [--json] [--folder Atlas/People]
vault class  [--root PATH] [--json] [--only c2] [--list]
vault people [--root PATH] [--json]
vault journal [--root PATH] [--json]
vault bases  [--root PATH] [--json] [--strict]
```

The root is a personal fact and never lives in this repo. It comes from the
overlay's `config/knowledge.toml` (`vault_root = "..."`), or from `--root`. The
server takes the same path from the same file and has no `--root`: a service
resolving its own root from an argument would be a second declaration of where
the vault is.

## `class` — which notes hold whose facts

PRD **Q9a** (2026-08-23): the folder sets the default, a note's frontmatter
`class:` key overrides it in either direction. `Atlas/People/`,
`Atlas/Documents/` and `Atlas/Finance/` are **C2 Others**; a health folder is
C2 wherever it sits; everything else is **C1 Mine**.

The rule is not here. `content_item::DataClass::classify_vault_note` holds it,
beside the mail rules and the C0–C3 vocabulary they share, because PRD §6.1
forbids a second definition of what `c2` means by name. This verb is the walk
and the report.

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
| `Soma.base` | `Projects/Soma/Domains` | `Projects/Axon/Knowledge-Base/Domains` | 4 of 4 |
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

`vault-server` on `8094`, loopback. Six routes:

| Route | Answers |
|---|---|
| `GET /health` | Liveness. A literal — it cannot see the vault. |
| `GET /ready` | Readiness: the vault root and its `Projects/` folder resolve. |
| `GET /routes` | This manifest, as data. |
| `GET /api/tasks?status=open\|done` | Every action note under `Projects/`, read live. |
| `GET /api/people` | `last_contact`, `met_at` and `mention_count` per person, computed from `Journal/` backlinks. |
| `GET /api/journal-health` | The six Journal health keys, counted; which of the six can be produced; `social` computed from `Journal/` person links. |

One task is `{id, title, done, due, priority, summary, projects, uri}`. `id` is
the vault-relative path; `uri` is the `obsidian://open` address of the note.

It exists because PRD **Q48** (2026-08-27) retired the `tasks` capability and
returned the Action kind to `Projects/**/Tasks/`, where the vault contract
§5.1b had assigned it all along. The dashboard's decision ladder needed an HTTP
source for band 620 and the data had moved here, so the reader that already
existed grew a second front end.

**Q104** (2026-09-09) closed the other half of that shape: `Projects/Tasks/` is
never created, so the folder rule reads a `Tasks/` folder only when a project
sits above it. A note filed directly under `Projects/Tasks/` is still served if
it declares `type: task` — the ruling is about what a folder name means, and it
does not silently drop an action. For example, `Projects/Garden/Tasks/Order
seeds.md` is served on its folder alone, and `Projects/Tasks/Order seeds.md` only
if it says `type: task`.

**There is no write route, and that is the ruling rather than an omission.** A
task is created, edited and marked done in Obsidian, in a note a human owns.
Adding a `PATCH` would make Axon a second writer of files a human is editing —
the conflict §5.5 states as "Axon reads the vault and does not write to it". The
ladder links to the note; it does not close it.

**Which frontmatter keys are served, and why not all of them.** A task note
carries eleven keys; five are served, because the ladder reads them: `summary`
renders the row (beside `title`, which is the filename, not a key), `due` and
`priority` rank it, `projects` labels it, `done` decides whether it is a
decision at all. The other six — `scheduled`, `context`, `energy`, `focus`,
`events` and `blocked_by` — have no reader, and a served field with no reader
is a contract nothing checks.

### `/api/people` — D2, served instead of written

`last_contact`, `met_at` and `mention_count` sit on 70 of the 89 `Atlas/People`
notes and have no producer. All three come out of `Journal/` backlinks and
`vault people` has computed them since 2026-09-07 — and refused to write them,
because **D3** is unresolved: machine-owned frontmatter has no protection
mechanism, so a producer could not tell its own value from a human's correction
and would overwrite the correction on its next run.

**A computed read does not need D3 ruled first.** Nothing is stored, so nothing
can be overwritten; the answer is recomputed off the notes on every request and
is stale for exactly as long as the request takes. The three fields reach a
reader without Axon becoming a second writer of files a human edits.

The drift is served beside the value — `stored` and `disagrees` per person, the
4 notes whose written value contradicts the Journal — because a computed number
served alone would read as authoritative, and the stored one is what a human
typed.

The route reads `Atlas/People/` and `Journal/` and not the vault: 442 notes in
20 ms against 2,757 in 155 ms, measured 2026-09-08. A vault with People and no
`Journal/` answers **503 and names the folder**, rather than 89 people at zero
mentions — that is the reading `people.rs` refuses, because zero is a
measurement and a missing producer is not.

`contact_frequency` is not served, and that is **D1**: how often you want to see
someone is a judgement, not a backlink count.

## `journal` — Q106, and the five keys that answer "no producer"

PRD **Q106** (2026-09-09) asked for a producer for the six keys every daily note
carries. **One of the six has a source. Five do not, and saying which is the
deliverable.**

| Key | Producer | Why |
|---|---|---|
| `social` | `Journal/` person links | a day that links an `Atlas/People` note names who was in it |
| `learning_hours` | none | nothing on the host measures hours |
| `exercise` | none | Apple Health is unreadable on a Mac |
| `sleep_quality` | none | same |
| `energy` | none | a self-report |
| `mood` | none | a self-report |

The consequence is visible in Obsidian without any of this. `Habits.base` is a
Base over the same folder that charts these keys, and a chart over keys that are
almost never filled in plots a handful of points. That is what a producer was
asked for, and five sixths of it cannot be supplied.

**Apple Health is not a source on a Mac, and that is measured.**
`HealthKit.framework` ships in `/System/Library/Frameworks` and its headers say
`API_AVAILABLE(… macos(13.0))`, so the API compiles and stopping there would be
reasonable. A Swift probe on a current macOS host answers
`HKHealthStore.isHealthDataAvailable() == false`, and `~/Library/Health` does not
exist. The framework is present and the store is not. The template describes
`sleep_quality` as "0-100 (SleepCycle / Apple Health native scale)", and neither
half of that is reachable without a phone-side export, which is a human act
rather than a producer.

**`energy` and `mood` are self-reports and this capability will not invent a
proxy for them.** Sleep would predict mood; a Mac's keystroke rate would predict
energy. Both would be a number Sjel made up, rendered in a chart the operator
reads as their own. The honest options are the human or the delete key. The
recommendation is to **keep both and cut the other four from the template**: the
two that only a human can fill in are the two worth prompting for, and a template
that asks six questions and gets one answer trains the person to skip the block.
Deleting `energy` and `mood` instead would cost `Resources/Bases/Habits.base` its
`wellbeing` formula and the charts built on it, which is the reason that Base
exists.

**`social` is produced, and it is served as evidence rather than as a verdict.**
The rule is the one **Q102** already ratified for `last_contact`: a `Journal/`
note that links a person is the record of contact with them. `people.rs` reads
those links per person; `journal.rs` reads the same links per day. The people are
served beside the boolean, because a derived `true` with no evidence is an
assertion.

The calendar was the other candidate and it does not work. `calendar_entries`
covers a far shorter span than the Journal and has no attendee column, so the
only person-shaped data in it is a name inside an event title. Joining that to
`Atlas/People` would be a string match on a few overlapping days. It is not an
honest producer and it is not built.

### Several numbers instead of one accuracy figure

```
social, produced from Journal person links
  days naming a person <n>
  distinct people      <n>
  stored social: true  <n>
    links agree        <n>
    links differ       <n>
  template false, links name somebody <n>
```

`social: false` on an untouched note and `social: false` typed by somebody who
spent the day alone are the same bytes. An instrument that called both a
disagreement would report every untouched day that names somebody as a
contradiction, and bury the few days where a human's answer and the links really
disagree. The split is the finding.

**The only labelled sample is the days a human set `social: true`.** The link
rule's agreement with those days is the producer's measured recall, and it is not
perfect: a day can describe meeting several people in prose and link none of
them. So "`social` has a producer" means a rule with partial recall against the
only ground truth that exists. That is why it is served as evidence with the
people beside it, and why nothing writes it.

### A trailing comment is not a value

`mood:                   # 1-5` is the template's own scaffolding, and
`markdown_root::parse_fields` reads it as the value `# 1-5`. That is correct for
a parser that also feeds a mail adapter, where `#general` is a value, and wrong
here. The census reads `Note::raw_frontmatter` with YAML's comment rule instead,
the same move `lint` makes and for the same reason.

The gap is not cosmetic. The template puts a trailing comment after
`sleep_quality`, `energy`, `mood` and `learning_hours`, so every unfilled note
stamped from it carries four keys that a comment-blind reader counts as filled.
No key in `lint::TRACKED` carries a trailing comment today, so `lint`'s published
coverage numbers are unaffected; this is a trap set for the next reader of these
six.

### A template that never ran

A daily note created outside Templater keeps the raw expression:

```
1 frontmatter values are still template expressions:
  Journal/01. Daily Notes/2031-03-15.md  week: "[[<% tp.date.now('yyyy-[W]ww', 0, tp.file.title, 'YYYY-MM-DD') %>]]"
```

It is reported and not repaired, because §5.5 is one-way and this capability has
no write path. The correct value is the ISO week of the note's date, the form the
neighbouring notes carry (`week: "[[2031-W11]]"` here).

The detector is for the class: any frontmatter value holding `<%` is a template
that never ran. The files in `Resources/Templates/` that legitimately hold one
are outside the daily folder and outside the scan.

It is also narrower than the defect. A `week:` link can be wrong without holding
a template expression — the wrong ISO week, or an empty `[[]]` — and a `<%` scan
sees neither. Neither is repaired here.

### Checked by a second implementation, as this crate's rule requires

`acceptance/journal-health.py` is to `vault journal` what `acceptance/link-counts.py`
is to `vault links`: its own walk, its own `[[...]]` regex, its own
comment-stripping loop, sharing no code with the crate.

```
python3 capabilities/vault/acceptance/journal-health.py <vault-root>
vault journal --root <vault-root>
```

Diff the two outputs. Every measured number must be identical: the note count,
the people register, the six `present`/`asserted`/`comment-only` triples, the
`social` split, and the unrendered notes. The only lines that differ are the
crate's extra `producer` column and the raw value it prints beside each
unrendered key. The probe deliberately carries neither: which key can be produced
is a decision, and a second implementation of a decision is not a check.

One blind spot they share, named so nobody reads their agreement as wider than
it is: both open a quoted scalar on `'` as well as `"`, so a plain value holding
an apostrophe — `mood: it's 5 # felt good` — keeps its comment in both.

### More notes than dates

`days` counts notes whose filename starts with a date, and one date can have two
of them: iCloud writes a conflict copy such as `2031-03-14 2.md` beside
`2031-03-14.md`. Both resolve to the same date, so `not_a_day` stays 0 and
`entries` carries two rows with the same `date`. The copy adds 1 to every
`present` count. **`date` is therefore not a key.** A writer that maps date to
entry drops one of the two silently, which is a thing to fix before, not after,
anything writes back.

### What it would take to write `social`

Nothing here writes. Q102 opened the vault for two `Atlas/People` keys under
their existing names and did not open the Journal, and the write path it
authorised belongs to another stream. Writing `social` later needs four things,
in this order:

1. **A ruling that extends Q102 to `Journal/01. Daily Notes/`.** Q102's own test
   is satisfied already — `Resources/Bases/Habits.base` and
   `Resources/Bases/Journal.base` both read all six keys, so `social` is not a
   key invented for a producer. `mention_count` failed that test and `social`
   passes it.
2. **A decision about the template defaults.** A writer that only fills blanks
   would write nothing, because no daily note has a blank `social`. A writer that
   overwrites `false` would overwrite every untouched day and could not tell them
   from a deliberate "no".
3. **D3, machine-owned frontmatter.** Same unresolved ruling that keeps
   `/api/people` a read: with no way to mark a value as Sjel's, the next run
   overwrites the correction a human made to the last one.
4. **The contradictions resolved by hand first.** Those are the days where a
   human wrote `true` and the Journal names nobody, and a producer would silently
   erase them.

**What counts as a task** is `capabilities/vault/src/tasks.rs`'s module doc: the
vault's own `Resources/Bases/Tasks.base` filter, scoped to `Projects/`, minus
archived folders, minus notes with no `done` key. Each divergence is measured
against the live vault and named there. Tracking the operator's own Base rather
than inventing a second definition is the point — two surfaces disagreeing about
one folder is exactly what §5.1b's no-doubling law forbids.

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

- `libs/markdown-root` — containment-checked vault access, recursive walk, and
  the byte-addressable frontmatter this crate reads. The offsets exist so a
  future writer can re-serialise frontmatter and concatenate the original body
  bytes rather than round-tripping prose through a parser that would reformat
  Bases embeds and Mermaid fences.
