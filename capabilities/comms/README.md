# comms

General observed-information intake for Sjel, plus reviewed mail triage. Its `feed_items`
store is intentionally source-agnostic. Security advisories and updates to systems or
packages belong here. So do watched-repository changes, news, useful articles and opportunity
signals such as scholarships, hackathons, calls for papers or events. The extractors
implemented today are the current ingestion set, not the boundary of that contract.

The capability currently has two ingestion paths:

1. **Gmail-as-router (read-only sweep, explicit reviewed actions).** Sweeps your inbox threads, classifies
   each into a stream (`aktiv`, `issue`, `feed`, `werbung`, `belege`, `steuern`,
   `sonstiges`) with a one-sentence rationale, and proposes triage. A sweep never
   changes Gmail. Authenticated dashboard actions may archive one stored thread
   or move it to Gmail Trash after a separate confirmation; permanent deletion,
   sending and arbitrary label changes do not exist.
2. **Share-link media ingest.** Turns a pasted URL into a feed item — metadata,
   an optional transcript, and an optional local-LLM summary — stored for a
   dashboard to read. Per-source extractors for YouTube/Instagram/podcast
   (yt-dlp), GitHub repositories, arXiv papers and Reddit posts; anything else
   falls through to a generic fetch-and-strip.
3. **Bounded awareness sources.** A source scan imports public GitHub Trending
   repositories or the newest results for configured arXiv queries. The public,
   non-personal defaults are daily GitHub Trending plus recent `cs.AI`, `cs.LG`
   and `cs.CL`; an explicit `feed_sources: []` disables them. Personal arXiv
   queries belong in the private overlay.

Mirrors `capabilities/scouting`'s shape: a `comms` library plus two binaries
(`comms` CLI, `comms-server` HTTP API), a blocking SQLite client, config resolved
from the private overlay at runtime.

### Mail classification today

Mail triage has two rungs, and everything it does stays on this machine. No
cloud model, no embedding service, no message body, no attachment.

**Rung 1 is deterministic.** It reads the sender header, the subject header and
whether a `List-Unsubscribe` header exists, and nothing else. First match wins:
personal rules from the private overlay, then generic public heuristics, then
the conservative `aktiv` fallback. Every proposal stores its rationale, its
method, its classifier revision — and, since the model rung exists, which of
those three rungs actually fired, because that fact cannot be re-derived later.

**Rung 2 is a local model, and it looks only at what rung 1 did not decide**
(PRD Q85, 2026-09-05). It runs on the threads that reached the `aktiv` fallback,
never on a thread a rule matched. Measured on a copy of the live store 2026-09-05:
102 of 237 proposals sat at that fallback, all 102 carrying the fallback rationale
verbatim, and the `issue` column the dashboard labels *Action* held zero rows —
*does this mail ask something of me* is not a substring, which is the justification
for climbing at all. It reads the sender's **domain** (never the address), the stored
subject and the stored preview — both already carrying
`deterministic-entity-redaction-v3` output, because intake redacts before it
writes. It answers with a category, a self-reported confidence and an urgency
score, each with a one-line rationale. Four things bound it:

- a `c3` **Secret** mail is refused before a prompt is built and before any
  model is woken, and the refusal is stored rather than left as an absence;
- only the light local model may answer. A thread too long for its window is a
  stored verdict, not a handoff to a bigger model;
- every call is loopback-only, and a non-loopback endpoint is refused outright;
- it is **shadow by default**. It writes verdicts to its own table and moves no
  category until the overlay declares `mail_model.apply` *and* a
  `mail_model.min_confidence_bp` above zero — a floor of zero would write every
  disagreement at any self-reported confidence, so the operator names the floor
  in the same edit that turns writing on. Even then it refuses any proposal that
  would raise the mail's data class — those are held for a person, because the
  class change and the redaction that follows it cannot be undone.

The rung is explicit: two routes and a CLI verb, no timer. Run it with
`comms mail classify --shadow`, read `comms mail classify --report`, then set
the two overlay keys and run `comms mail classify --apply`. Apply does not ask
the model again: a stored shadow verdict already carries the answer, so the
apply pass reads it and writes the category. Undo a bad batch with
`--revert-all` (`--revert` needs a thread id and refuses to stand in for it),
which restores the deterministic verdict on the category axis and says in its
own output what it cannot restore.

A category changed in the dashboard becomes a `human` override and every later
sweep preserves it, model pass included. A model row survives a deterministic
resweep, but a rule that actually **fires** takes the row back — so a new
overlay rule can still correct the model, which a bare method rank would have
made impossible. Category ordering in the dashboard is an attention aid, not a
hidden score, and **the urgency score ranks nothing yet**: it is stored, published
and displayed, and the evaluator's reserved urgency factor reads it through
`mail_evaluation::urgency_from_verdict` behind one gate,
`mail_evaluation::URGENCY_VALIDATED`. That constant stays `false` until the frozen
corpus in `eval/README.md` carries a measured urgency-band error, so turning it on
is a source edit a reviewer sees rather than an overlay value that reorders the
ladder silently. A stored `urgency_bp` with no rationale beside it counts as absent:
a weighted bar the reader cannot check is what this evaluator exists not to render.

Every shared content item also carries one inspectable trust class (Q27).
`c0` (shown as **Public**) may use local processing and is eligible for
configured cloud processing. `c1` (**Mine**) stays local unless an explicitly
reviewed, pseudonymized derivative is created. `c2` (**Others**) holds another
person's facts and never reaches a cloud provider at all. `c3` (**Secret**)
holds credentials and is refused local model processing too — mechanically, not
by declaration: `content_item::local_prompt_allowed` is the gate, and both
prompt-builders here ask it before they build a prompt (`digest.rs` for the
digest, diagram and chart rungs, `media.rs` for the feed-summary drain). A
refused item stores a `local_refused` row saying so. The cloud side is the same
shape: `content_item::cloud_admission` is the policy and `tier_allows` here is a
thin wrapper that adds the transformation-version pin (see the root
[README](../../CONTRIBUTING.md#data-classes)). Public Feed
sources default to `c0`; mail defaults to `c1`, while deterministic metadata
rules raise likely tax, receipt, financial or health mail to `c2` and
authentication or account-recovery mail to `c3`. A mail that names a person the
vault knows is raised to `c2` as well. The rules use sender, subject, snippet
and category only—never body or attachment content—and a dashboard override is
stored as `human` and preserved by refreshes.

For `c2` and `c3` mail the metadata *is* the payload: a one-time code arrives in the
subject line, so storing that subject verbatim would publish it to a log, an API
response and a dashboard at once. The sweep therefore redacts subject and
snippet before the row is written, using the same local deterministic detector
the cloud preview uses (`deterministic-entity-redaction-v3` — links, addresses,
IBANs, phone numbers, long numbers, token-like secrets, and people named by a
salutation, a self-introduction, an organisation apposition or a login handle).
A redacted subject
reads `Your verification code is [number]`. The sender is kept: it is what makes
a proposal reviewable when its subject cannot be read. Both sweep entry points —
the CLI and the HTTP API — go through one intake path, because a gate only one
of them uses is a gate over half the traffic. `POST /triage/redact` applies the
same pass to rows stored before this existed; it is idempotent and reports what
kind of thing it removed, never the value. It reaches the model rung's stored
sentences as well: a verdict is redacted against the class its thread held when
it was written, so a thread that rises to `c2` later has its verdict narrowed by
whichever path raised it, this route included.

**A reduced call says so in words** (PRD Q9b, 2026-08-23). Every cloud
derivative that removed anything carries `redaction_receipt`, one sentence
built from the findings already on the object: *"Reduced 4 details before this
call: 2 mentions of people, 1 email address and 1 link."* `null` when nothing
was removed — a receipt on an untouched call trains the reader to stop reading
the ones that matter. It is composed in `cloud_derivative.rs` rather than by
each surface, so the CLI, the dashboard and anything later say the same thing
about the same call, and the dashboard renders it above the per-kind ledger it
already had.

Two things about the wording, because both are constraints and not choices.
The sentence counts **occurrences, not people**: findings aggregate by
`entity_type`, so `person: 5` means five mentions were replaced and says nothing
about how many distinct people they were — the detector never learns that,
because knowing it would mean keeping the names. Q9b's own example reads *"3
facts about 2 people"*, and the second number is not in the data. And a kind the
phrase table does not know is named by its raw literal instead of being dropped,
because the leading total sums every finding: a receipt whose breakdown does not
add up to its own number is worse than no receipt.

A later sweep cannot undo that. Classification runs the named-person rule
against the people registry, so a pass with the overlay unmounted answers `c1`
for a thread an earlier pass raised to `c2`; the class column keeps the strict
value, and the upsert redacts the *incoming* subject and snippet against it
before writing them. The row therefore keeps following its thread — a newer
message's date, sender and text — while the redaction follows the class. Keeping
the old text instead would leave one message's date beside an older message's
subject for the life of the thread.

### Collecting on a schedule

`inbox_sweep_minutes` lets comms-server pull proposals unattended. It ships as
`0`, disabled — a background job that reads a mailbox is opt-in per machine, not
something a fresh clone starts doing. The manual paths are unaffected either
way: `comms sweep --dry-run` previews without storing, and the board's own sweep
button still pages on demand.

Each scheduled pass takes the newest `inbox_sweep_max_threads` threads and stops
— a bound, not a cursor. A cursor advancing every run would walk backwards
through the whole mailbox over days, which is the unbounded rescan the schedule
is meant to avoid; re-reading the newest page instead costs nothing, because
proposals upsert on Gmail thread id and preserve human decisions. Paging deeper
stays a manual, cursor-carrying call. Both the schedule and the manual route run
the same sweep function, for the same reason both go through the intake gate.

`inbox_sweep_quiet_hours` (`"22-7"`, wrapping midnight) suppresses only the
unattended pass. Failures are counted, not narrated: the run records an error
*class* — `auth`, `quota`, `network`, `unknown` — and each consecutive failure
doubles the number of ticks skipped, up to 32, so an expired token backs off
instead of retrying every fifteen minutes all night. Any success clears the
streak. `GET /triage/sweep/status` exposes last run, last *success* (kept
separate, because a failing run still ran), last failure, the error class and
the counts from the last pass; the mail board renders it, and reads visibly
different when the schedule is failing.

### Mail events into Calendar

A stored inbox sweep also checks new mail for event/ticket terms in its subject and preview. Only a
candidate's latest body is fetched, capped, and sent to the loopback `summarization_light` role;
body text and model prompts are never stored. `c3` is refused before the body fetch. `c2` may be
analyzed locally, but the title, location and evidence are redacted before they cross into
Calendar. If no light local role is configured, or the message does not fit its window, no bigger
or remote model is substituted.

Dates become Calendar `event` proposals only when the model supplies a verbatim evidence quote
that grounds the date in the fetched body; the quote is discarded after validation. Each proposal
is an all-day, one-day `possible` entry with an idempotent source key. It does not mark a ticket as committed, write to Gmail, or export
to Google Calendar. Calendar's existing trip-draft clustering can then suggest a trip; creating
the Trips plan remains an explicit action. The existing `inbox_sweep_minutes` cadence controls
both collection and event analysis: `0` is manual-only (the default), and a configured interval
processes only newly stored threads. A manual sweep can re-check its page after a reported
analysis failure.

Category and TELOS relevance are deliberately separate axes. An explicit
relevance refresh compares the stored sender, subject and Gmail snippet with
the configured TELOS lenses through the existing embedding/reranking pipeline.
Only loopback inference endpoints are accepted for mail; if the local model is
unavailable, the result is truthfully labelled `lexical`. Mail bodies and
attachments are not fetched or scored. Raw relevance values are ranking
signals, not calibrated probabilities, and neither scoring nor a human mail
correction changes a TELOS source note.

The dashboard presents proposed mail as a horizontally scrolling category
board. Each column has a real stored-item count, its own select-all control and
cards ranked by their strongest TELOS match when one exists. Inbox collection
is cursor-paged in batches of at most 100 instead of being capped at the CLI's
25-item default. The cursor is intentionally browser-session state: rescanning
from the newest page is idempotent because proposal upserts preserve human
decisions.

The board is an index, not a second reader. Opening either a normal Feed entry
or a mail proposal resolves the versioned
[`content-item-v2`](../../schemas/content-item.schema.json) contract and uses
the same dashboard reader. The contract owns canonical title, author, optional
summary, source content, relevance, evaluation and provenance. Its nullable
`mail` extension adds category and classification evidence; Gmail mutations
remain separate authenticated actions. This adapter boundary lets collection
tables evolve independently without duplicating content presentation.

## Digests: what the local model wrote, sized to the source

A **digest** is a different noun from a summary. `summary` on `content-item-v2` is what the
*source* said it is — calendar reads it from the entry's own description — so a generated
paragraph written over it would destroy the only verbatim text an entry has. The digest is
stored beside it in `content_digests`, one table for every source, because a digest has none
of the per-item invariants that keep the item tables apart.

The shape follows the length, and nothing else chooses it: under 600 characters no digest is
produced at all, then brief, standard and sectioned, each raising both the structure asked for
and the token ceiling. The full ladder and its reasoning live in
[`libs/summarize`](../../libs/summarize/README.md).

**Improving one is a press, not a re-run of the same thing.** `POST /content/:source/:id/digest`
takes `{"depth":"detailed","focus":["…"]}`. `detailed` moves the shape exactly one rung up that
same ladder — asking a model to "be more detailed" only gets a longer version of the same
guess — and the focus terms are named individually in the prompt, with an instruction to say so
rather than invent when the source is silent on one. Both are stored with the result, so a
differently-shaped digest is explained rather than mysterious, and the automatic pass skips any
row whose `depth` is `detailed`: a model upgrade must not quietly throw away a decision an
operator made. A digest replaces its predecessor; there is no revision history.

That ladder also gives the short floor its escape hatch. The automatic pass skips a 400-character
item as `skipped_short`; an explicit press produces one anyway, because the operator looking at it
can see something the character count cannot. An *empty* source is not that case and stays
skipped either way — a model asked to summarize nothing invents the answer confidently.

### Mail reads a body, and does not keep it

The sweep is unchanged: `format=metadata`, read-only, no body. A **digest** fetches
`format=full` for that one thread, walks the MIME tree to `text/plain` (or strips `text/html`
through the same extractor the article path uses), hands it to the model and drops it. Nothing
writes it. The digest row is the only thing that survives the call, which is the whole point:
mail is distilled into an outcome, never retained as a local copy of the message.

Two gates follow the data class rather than the caller. Whether a digest may use a non-loopback
target is decided by the same function the reviewed-derivative queue uses — `tier_allows` in
`cloud_derivative.rs`, asked about the passthrough representation, because a digest sends the
source text as it stands. Nothing but `c0` survives that question, mail is never `c0` by
construction, and an endpoint carrying no reviewed cloud policy admits nothing at all. A cloud
endpoint is refused outright rather than downgraded. For `c2` and `c3` content the produced digest passes the same
deterministic detector the sweep runs on subject and snippet **before** it is stored, and the
count is recorded: a model asked to summarize a one-time-code mail will quote the code, and the
digest must not be where it gets published.

`POST /content/digests/refresh {"source":"mail","limit":25}` is the bounded automatic pass. It
is explicit rather than timer-driven for the same reason the inbox sweep ships disabled: a
background job that quietly pulls every body out of a mailbox is not something a fresh clone
should start doing.

### Diagrams

`POST /content/:source/:id/diagram` draws the item as one Mermaid diagram, from the digest when
there is one — a diagram of a long paper drawn from its first 15,000 characters is a diagram of
its introduction. The answer is validated before it is stored: a model asked for a diagram will
cheerfully answer with prose, and prose in a diagram column fails at the reader, which is the
hardest place to work out what went wrong. Same remote refusal as the digest, because a diagram
of a private mail is still that mail.

### Charts, and why a number has to be findable

`POST /content/:source/:id/chart` pulls one set of comparable numbers out of the item's source
and stores it as a table. Most content has none, and that comes back as `skipped_short` rather
than an error: a reader that showed a failure for every ordinary essay would train you to stop
reading the state.

**The blocker was never rendering.** A feed item is prose, so "chart this" is an extraction
problem, and extraction has one failure mode that matters: a model asked for numbers produces
plausible ones. Every value therefore has to appear **verbatim in the source** before it is
allowed into a figure. That gate is deterministic, not a second model grading the first, and it
runs against the same capped text the model was shown — claiming a number is present because it
sits in text the model never saw would make the check a formality. Rows that fail are dropped;
if too few survive, the whole extraction is refused with a count of what could not be found.

**One measure, one series.** The figure palette is the low-chroma print palette the operator's
papers use. Run it through a categorical-separation check and even two of its hues fail: `warm_dark`
against `teal_dark` is ΔE 9.7 for *normal* vision, under the 15 floor, before colour-vision
deficiency is considered. It cannot carry identity, so a chart drawn from it must not need to.
That is also what prose actually yields.

**The form is derived, not chosen.** No second model call picks a chart type: an ordered run of
three or more categories gets a line, everything else bars. One less model output to validate,
and the same answer every time.

What is stored is a **table, not a chart specification**. The dashboard compiles it into
Vega-Lite, so transforms, data URLs and the palette stay out of a model's reach; the model
chooses the numbers and, indirectly, the mark, and nothing else. The figure ships with its
caption and a disclosure holding the extracted values, because an extracted number is a claim
about a source and the table is where you check it.

Calendar entries are digested through the same routes. Comms reads one entry over Calendar's own
`content-item-v2` contract — the bounded cross-capability read it already does against Trips —
rather than opening a second capability's database schema.

## Relationship to scouting

Feed and Scouting are connected but not interchangeable. Feed answers “what changed or may be
worth noticing?” Scouting answers “which opportunities fit the operator's current profile,
and why?”

A feed entry may later be promoted to Scouting when it represents a scholarship, hackathon,
event, call for papers or another opportunity that needs scoring. Most kinds should not take
that path. Security updates, release notes, repository changes and general articles remain
useful observations without becoming opportunities. Conversely, a Scouting run may publish a typed
observation into the feed so the result appears in the daily stream. The cross-capability
link must preserve the source item ID rather than copy its content into an unrelated record.

TELOS relevance is an annotation on this general feed, not a filter that turns it into
Scouting. Explicitly configured `TELOS/Focus/*.md` notes become independent lenses. The
optional single-line `relevance_query` frontmatter owns the semantic search intent; when it is
present, only that value is embedded, keeping file names, UI labels, wiki links and note
scaffolding out of the vector. Notes without it retain the original bounded full-note
representation. The item and every lens are embedded in one batch through the same backend; if
that backend is unavailable, both sides use the same deterministic lexical vector space and the
stored match is labelled `lexical`. Scores are raw ranking signals, not calibrated probabilities.

The Feed's displayed rank is a separate deterministic evaluation, not an LLM judgment.
`feed-evaluator-v6-feedback` combines the strongest TELOS match (45%), an explicit upcoming-trip
match (25%), age since first ingest (20%), the stored content basis — title, author, summary and
source text (10%) — and, when it has earned the right to count, a fifth factor learned from the
operator's own decisions (15%, with the four base weights scaling to 0.3825/0.2125/0.17/0.085 so
the sum stays 1.0). Travel matching compares item text with destination names and the plan's
declared interests; the winning factor carries the Trip ID, label, dates and matched terms so
the UI never has to reverse-engineer a prose explanation.

One rule governs every factor whose producer has not run: **it carries weight 0 and the others
scale so the sum stays 1.0.** That is how an inert learned factor, or an urgency the model rung
has not published, leaves the arithmetic whole instead of dumping the item to the bottom of its
band by a zero it never earned.

**A class refusal is not that case, and is never rescaled.** The refused share is withheld, not
handed to the survivors, so a refused item's weights sum to less than 1.0 on purpose. Rescaling
it promoted exactly what the class ladder forbids: measured on a copy of the live database, a
refused mail kept only `category` (a constant 1.0 on the `aktiv` band) and `age`, so all eleven
c3 threads scored 0.863-0.990 while the best mail anybody had actually read reached 0.702. A
refusal is missing evidence, not a free pass, and a refused row can never outrank a scored row
whose other factors are identical.

**Nothing the class ladder refuses is scored, by either path.** `score_items` asks
`content_item::local_prompt_allowed` before it builds the embedding batch, and a refused item
is excluded from the lexical fallback too — one rule instead of two, because a lexical score is
still a content-derived number rendered in a rationale on a surface. A refused item is stored
as a refusal: an evaluation at `mode = 'unscored'` with a zero-weight interest factor reading
"Not scored: c3 is never read by a model", and its stored matches deleted. Both writes go past
the tier gate, because a refusal is a withdrawal rather than a weaker producer: the refusal's
`unscored` mode ranks `deterministic`, so escalating an already-scored item to c3 otherwise left
its model-derived score, its rationale and its matches exactly where they were while the pass
reported the refusal. This is not
hypothetical for mail: the triage scoring path built a synthetic `FeedItem` and never copied
the triage row's class, so `FeedItem::new` stamped the undeclared default — literally `c1` —
and every c3 mail was embedded, sender address included.

**What must be re-embedded and what must merely be re-ranked are two different questions.**
The *relevance revision* (lens fingerprints plus the embedding and reranking producers) decides
what has to go back through a model; the *context revision* (those plus the travel snapshot)
decides only what has to be re-evaluated, from matches already stored. Before the split every
term in the context revision was an embedding trigger, so a trip starting re-embedded the whole
window for a factor weighted 0.10. A stored `lexical` row is also stale while an embedding role
is reachable, which drains rows written during an outage over ordinary passes rather than
needing a force flag.

**A pass records a receipt, and the receipt is not a delivery** (PRD Q90, 2026-09-05).
`POST /feed/relevance/refresh` writes a `relevance-pass` row into `comms_source_state` with
the mode that actually answered,
the counters and an error class — and never `last_success_at`, because that column is the whole
body of `GET /__axon/freshness`, which answers "is data still reaching this capability".
`local-inference` is excluded from that query for the same reason: a machine whose collectors
have all stopped must not be held green by a local model answering a drain.

**A request that names no `offset` resumes the sweep; one that names an offset gets that page.**
The receipt's cursor carries how far the chain in progress has come, and until 2026-09-08 nothing
read it back: `tools/feed-sweep` posts `{days: 3650, limit: 100}` nightly with no offset, so
every night re-scored the same newest hundred rows. The deployment's cursor stood at
`@3650:100` with 175 of its 374 scored items still `lexical`, every one of them at offset 100 or
beyond. A chain is resumed only at its own window, a narrower window may not take a wider chain
over — the Feed panel's button asks for 90 days and would otherwise reset the nightly sweep on
every press — and reaching the end of a list resets the progress so the next pass starts over.
New items do not wait for the sweep to come round: `enrich_many_in_background` scores an item
when it is ingested.

`comms relevance backfill [--days N=3650] [--batch N=100] [--max N] [--force]` pages that route
until every item in the window has been seen, printing the mode each page answered in. It names
every offset it sends, so it walks the corpus itself. Only a
chain that ran the full 3650-day window marks the corpus complete at the current relevance
revision: a narrower pass reaches the end of its own window after a handful of rows, and stamping
the corpus done from there left every row it never saw reading as current forever. It is an HTTP
client against the running server, never a second opener of the database: `Store::open` runs the
whole migration on every call and two openers deadlock.

**The fifth factor learns from the ledger, and is inert until that is worth doing**
(PRD Q88, 2026-09-05). L2-regularised logistic regression over an explicit ~50-slot feature
vector — kind, source, top TELOS lens, lens scores, content_status, freshness bucket, hashed
author, hour bucket. **No slot
carries text**: the author is one of sixteen hash buckets and everything else is a categorical id
or a number, so a feature vector cannot reconstruct a title. Training obeys the same ladder as a
prompt: an item that fails `content_item::local_prompt_allowed` contributes no label and no
feature vector. A full deterministic refit, never an incremental update — identical inputs give
identical weights, which is what lets the revision be a cache key — stored as a second
`context_kind = 'feedback-model'` row in the existing `comms_feed_context_snapshots`, with its
`feature_names` beside its weights: a new lens or a new source changes the vocabulary, and a
model whose stored names differ from the ones computed now is stale by definition.

The factor renders at **weight 0 with the rationale "not yet learned"** until three measured
conditions hold — ≥50 labels, ≥10 of the smaller class, held-out AUC ≥0.65 on a time-ordered 30%
split — and while it is inert its revision is the literal `none`, so accumulating labels does not
restale cached evaluations for a factor that counts for nothing. What obliges the gate: of the
19 live labels the keepers are github, youtube and article, and **zero of the 185 stored arXiv
items has ever been kept**. A model fitted on that learns "arXiv is never kept" and buries the
largest source in the feed. `POST /feed/model/train` refits and reports the gate; `dry_run`
reports it without writing. It is gated against a frozen synthetic corpus whose judgement rule
was written before the trainer ran (`eval/feedback-corpus.json`). Active, the factor names its
two strongest signed contributions — it may re-rank and must explain itself, and it may never
write a status.

**Mail has its own evaluator, and it publishes one number with one writer**
(PRD Q89, 2026-09-05). `comms_triage_evaluations` and `comms_triage_evaluation_factors`
mirror the feed's pair column for column — same currency check, same tier gate, same
normalized factor table — with mail-shaped factors: TELOS interest 0.55 from the rows
`POST /triage/relevance/refresh` already stores, category 0.30 from the `rules` stream,
age 0.15 reusing the Feed's own freshness curve,
and a reserved **urgency 0.25** the LLM model rung fills through
`mail_evaluation::urgency_from_verdict`. It stays at weight 0 while
`URGENCY_VALIDATED` is false, and the other three scale to 1.0 — except on a class refusal,
which is never rescaled, so a refused mail caps at 0.45 and cannot outrank a mail that was
read. A refusal passes no urgency whatever the rung once stored: a c3 mail reached no model,
and a refusal exists to withdraw a model-derived number rather than to carry one forward.

There is **no correspondent factor**: `is_known_person` compares one whitespace-free token,
so it is false for every address, and Q72 rule 2 already settled that the registry is asked
per token of subject and snippet, never over the sender. Said plainly: on the `aktiv` band
the category factor is a constant, so ordering there is carried by interest, age and — when
it exists — urgency.

`TriageOut` gains `score_bp`, the evaluator's `overall_score` in basis points (0..=10000, the
unit the companion register uses per Q73), and `evaluated_at`. **One writer**: this evaluator.
The model rung's urgency is not a competing number on the same field — it arrives as the
`urgency` factor's input, so it moves the score *through* the evaluator and keeps its rationale
beside the other three. `null` means no stored evaluation, which is not the same as 0. The full
breakdown rides the existing reader contract at `GET /content/mail/:id`, whose `evaluation` was
hardcoded `None` until now.

**A rationale never quotes a stored mail field.** `intake` redacts subject and snippet for c2
and c3 and deliberately keeps the sender unredacted, so no address, subject or snippet appears
in a rationale, a log or a receipt — only the lens label, the category and the age.

**Every decision is now recorded, with its time.** `comms_feed_interactions` is append-only:
`(feed_id, event, surface, occurred_at)`, where `event` is one of `opened`, `kept`, `dismissed`,
`reopened`, `unkept` or `shared`. An item's label is its most recent decisive event, retracted
by a later `unkept`. **One writer per verb**: `set_feed_status` writes `kept`, `dismissed` and
`unkept` inside the same transaction as the UPDATE, and `POST /feed/:id/interactions` refuses
those three with a 400 naming the status route, accepting only `opened` and `reopened`. Two
paths writing one decision would double every count in a table whose whole justification is
that it can be read by hand — and so would one path writing a decision that never happened, so
one row means one status *change*: `set_feed_status` reads the stored status inside the same
transaction and writes nothing when the press does not move it. `GET /feed/evaluation/status`
reports the ledger's own counters as `interactions`, which is where `opened` and `reopened` —
verbs no training label reads — reach a surface. `shared` ships with no writer on purpose: SQLite has no alterable
constraint, so widening that CHECK later costs a table rebuild, and declaring the value now is
free. A row carries ids and verbs — no content, no text.

Trips remains the owner. Comms reads `GET /api/plans`, retains a bounded snapshot containing
only upcoming plan identity, title, destinations, date window, interests and revision, and
uses the last successful snapshot while Trips is temporarily unavailable. It does not copy
traveler names or itinerary items. A plan change alters the evaluation context revision;
normal refreshes therefore evaluate changed context once and skip it thereafter. Changing a
Feed status alone still does not invalidate the evaluation.

Obsidian has three bounded roles:

- The HTTP scanner reads only exact Markdown files declared in private configuration. A
  source may require an exact heading; when that heading is missing it returns zero candidates
  and never falls back to the full note. Frontmatter, fenced code, inline-code URLs, image
  targets, private-network hosts and credential-looking URLs are excluded. Scanning does not
  fetch anything. Each candidate still needs an explicit import.
- **The feed library projects into `Resources/Sources/`.** Saving an item writes a note;
  unsaving removes it. The contract is below.
- The older CLI keeper export can write a distilled reading note for an explicitly kept
  item when `keeper_export_dir` is configured. It is the ad-hoc exporter PRD Q49 replaces:
  unset in this host's overlay, superseded by the projection above, and left in place rather
  than removed in a commit about something else.
- `comms keep <id>` resolves a MAIL id as well. Until it did, a mail had no way out of the
  Inbox except staying in it, so the Information lane was declared rather than real. The note
  carries subject, sender, date, a Gmail permalink, the stream and the data class. It never
  carries the snippet or the body, which are the raw mail this lane exists to avoid copying.
  Every field comes from the stored row, so a c2 or c3 mail exports the redacted form the intake
  gate produced. No Gmail write happens: archiving is a mutation the doctrine permits only on
  explicit approval, and folding it in here would archive as a side effect of filing.

No path here is the Trips importer, edits TELOS, or scans Scouting's opportunity notes.

### The feed library in the vault

PRD Q49 (2026-08-27) ruled one shared machine→vault bridge rather than an exporter per
capability, and named the feed library as its first consumer. The mechanism is
`libs/markdown-root/src/projection.rs`; `src/projection.rs` here declares only the shape.

**What is projected.** Every feed item with `status = 'keeper'` — the whole library, with no
time window, because `/feed/library` is the durable collection and a windowed query would
delete a note the moment its item aged out. One note per item, at
`Resources/Sources/<title>.md`.

**When.** Two triggers and no schedule:

| Trigger | Covers |
|---|---|
| A layer on `comms-server`'s write routes, after a `2xx` | every save and unsave through the dashboard |
| `comms export-sources [--dry-run]` | the repair, and the first run on a host; works with the server down |

The layer runs after the response, in a spawned task, behind a lock. A vault failure is
logged and the request still succeeds: a durable row must never be traded for a reachable
vault. What the layer does *not* catch is enrichment — a summary that arrives from the
background drain comes from no request, so the note carries it after the next mutation or the
next `comms export-sources`.

**Why the folder is treated more carefully than `Resources/Sjel/`.** Q49 sends this bridge
into the folder the Sources consolidation is building: humans write there, `Clippings/` merges
in, and `Atlas/Media`'s V3 survivors move across. Three guards follow, and each has a test:

1. A file whose projection header does not name `comms` as its owner is never written and
   never deleted. That covers a human's note and another capability's projection alike.
2. A refused name is not worked around. When a human's note holds the name, the item gets no
   note rather than a near-miss file beside it — a second note about one source is what the
   consolidation removes. This is Q31's promotion, and the run reports every refusal.
3. The sweep considers `Resources/Sources/*.md` and nothing else, and removes only files
   carrying comms' header. Unsaving deletes the machine's note, never the human's.

Two saved items whose titles reduce to one file name both take an id suffix, so neither name
depends on iteration order.

**What the frontmatter carries**, under vault rule V2 — *a key must name its reader before it
may be written*:

| Key | Value | Reader |
|---|---|---|
| `type` | `source` | the Source template's own first key |
| `format` | the item's kind as a word `Atlas/Media` already uses | `Media.base`'s Format column and `format_emoji` |
| `status` | `backlog` | all four of `Media.base`'s table views filter on it |
| `author` | the row's author, when present | `Media.base`'s Author / Creator |
| `url` | the saved link | the Source template declares it; it is the way back to the thing |
| `created` | the item's ingest day | the key the ten `Clippings/` notes already use |
| `axon_feed_id` | the row id | the sweep, and the argument `comms dismiss <id>` takes |
| `axon_projection_version` | `1` | a later generator, recognising output it cannot produce |

The title is the file name, not a key — `Media.base` renders `file.name` as Title. `format`
maps each feed kind to a word measured in `Atlas/Media`: article (11 notes), thread (7),
paper (6), repo (2), video (1). A `mail` item gets no `format`, because the vault has no word
for one and inventing a value nothing groups by is worse than omitting the key. `summary` is
the one reader-having key left unwritten: the body carries the summary verbatim, live
summaries run 484–1,293 characters, and a paragraph that size in a YAML scalar buries the
note. `data_class`, `stream`, `content_status` and the provenance columns have no vault
reader and stay in the store.

**The saved date exists now, and this note still does not render it.** `set_feed_status`
writes a `kept` row into `comms_feed_interactions` in the same transaction as the status
change, so the moment a link was saved is kept — it was not before. What the frontmatter
should carry is a separate decision about the note: `created` is still the day the item was
ingested, which is earlier than the save and sometimes by weeks. The fact is now available to
whoever takes that decision; it is no longer a column and a migration.

**No `Resources/Sources/` Base exists yet.** `Media.base` still filters
`file.inFolder("Atlas/Media")`, so these notes are queryable only by folder until the
consolidation repoints it. That is a vault-side chore, and this projection does not do it.

## Why this shape: one local inference server, then native Apple where it earns the seam

Comms does not need both oMLX and Ollama as permanent runtimes. The split was historical:
summaries already used oMLX's OpenAI-compatible chat endpoint, while relevance was first
implemented directly against Ollama's `/api/embed`. Current oMLX releases expose
`/v1/chat/completions`, `/v1/embeddings`, `/v1/rerank` and `/v1/models`, support embedding and
sequence-classification models alongside
LLMs, and manage them through LRU eviction, per-model idle TTLs and a process memory guard.
The intended local-AI rung is therefore one authenticated oMLX process with one generation
model, one much smaller multilingual embedding model and one bounded reranker. See the
[oMLX API and model-management documentation](https://github.com/jundot/omlx#api-compatibility).
The selected model is
[`mlx-community/multilingual-e5-base-mlx` at the audited commit](https://huggingface.co/mlx-community/multilingual-e5-base-mlx/tree/576fdf3eab52a419f6d126a0c4d7c59b3882ffde):
573 MB, 94 languages, XLM-RoBERTa, MIT-licensed, and represented by tokenizer/config data plus
one safetensors weight shard. It replaced the 253 MB E5-small baseline only after the base
model passed the same frozen DE/EN corpus that small failed; both decisions and exact pins are
recorded in `upstreams.toml`. The reranking stage uses the Apache-2.0
[`soichisumi/bge-reranker-v2-m3-mlx` pinned tree](https://huggingface.co/soichisumi/bge-reranker-v2-m3-mlx/tree/b4577f49e18adb53ed9e557192094f69f3dc2c1c),
an XLM-RoBERTa sequence-classification conversion whose inspected tree contains only model,
tokenizer and config artifacts.

The shared `inference.json` makes that protocol choice explicit. Comms asks for
the `embedding`, `reranking` and `summarization` roles; backend URLs, model IDs, role prefixes
and key-file references are declared once there rather than repeated in
`comms.json`:

- `openai` is the preferred path. It posts one revision-cached batch to
  `{base_url}/embeddings`, restores the response by its declared indexes and may use the same
  backend key reference as the summarizer. The role's `query_prefix` and `document_prefix` express
  retrieval-model roles without coupling Comms to E5; the documented candidate uses `query: `
  for TELOS lenses and `passage: ` for feed items.
- `ollama` remains a migration and compatibility path for installations that already have an
  embedding model there. Its request sets `keep_alive: 0`, because a cached background
  enrichment pass benefits more from returning unified memory immediately than from keeping a
  model warm for Ollama's default idle period. See Ollama's
  [keep-alive documentation](https://docs.ollama.com/faq#how-can-i-preload-a-model-into-ollama-to-get-faster-response-times).
  It has no native reranking endpoint, so Comms truthfully keeps semantic scores on that backend.
- An unknown, unavailable or incomplete provider fails to the labelled lexical path. It never
  silently presents a heuristic score as semantic.

Feed relevance is two-stage. One mixed embedding batch selects at most three candidate lenses
per item. The reranker then evaluates those `(lens, item)` pairs jointly in batches of 32. If a
reranking request fails, the whole refresh keeps the embedding scores rather than mixing score
spaces; if embedding fails, the deterministic lexical control remains labelled `lexical`.

The resolved embedding and reranking roles' cache keys are part of the evaluation context
revision. Changing either producing backend or model therefore invalidates the right ledger rows
on the next normal refresh. Relevance input is capped at 1,800 characters, and a stored summary
replaces rather than duplicates the raw transcript; this fits the selected model's 512-token
window without spending local compute on text it would truncate.

On this memory-constrained interactive Mac, oMLX now uses its aggressive memory guard, at most
eight concurrent requests, an embedding batch size of 32, a 60-second idle TTL for both E5
models and 180 seconds for Gemma. Those are machine settings in oMLX, not tracked Sjel defaults.
They should be tuned from measured peak memory and latency rather than copied as a universal
configuration.

Apple's system model is the next summarizer experiment, not the evaluator and not an embedding
replacement. The
[Foundation Models framework](https://developer.apple.com/documentation/foundationmodels/)
and Apple's official
[Foundation Models SDK for Python](https://apple.github.io/python-apple-fm-sdk/)
provide local, OS-managed summarization on Apple Intelligence-capable Macs. The experiment
must account for the system model's
[4,096-token session limit](https://developer.apple.com/documentation/foundationmodels/managing-the-context-window):
Comms currently permits 15,000 input characters, so a native provider needs bounded
chunk-and-reduce summarization rather than simple truncation. It must also record the system
model/OS revision because Apple updates the model with the OS.

For relevance, Apple's
[`NLEmbedding`](https://developer.apple.com/documentation/naturallanguage/nlembedding) was the
first native candidate because it is the purpose-built semantic-similarity API. The current
Mac exposes revision-1 German (640-dimensional) and English (512-dimensional) sentence models.
Using each query's language model to rank both German and English candidates failed at 3/6
useful top-1, `0.500` pairwise accuracy and `0.699` mean nDCG. Apple's
[`NLContextualEmbedding`](https://developer.apple.com/documentation/naturallanguage/nlcontextualembedding)
does expose one shared Latin model for both languages. Mean-pooling its documented subword
vectors produced a fast, low-process-memory comparison, but failed the unchanged corpus at
4/6 useful top-1, `0.618` pairwise accuracy and `0.776` mean nDCG. Five of six top results
matched the query language, including four weaker same-language candidates ahead of the
intended cross-language match. It is therefore not a Comms replacement. The deterministic
evaluator and lexical fallback remain the control in every run.

### First embedding smoke test

On 2026-07-29 the exact registered E5 snapshot was loaded through oMLX 0.5.0 and called through
the authenticated `/v1/embeddings` endpoint. It returned four 384-dimensional vectors for
synthetic German/English inputs. One German local-AI query scored `0.876` against the matching
English passage and `0.845` against the matching German passage, versus `0.753` for an
unrelated ceramics passage. This proves protocol/model compatibility and gives a first
cross-language separation signal; it is deliberately not presented as a quality benchmark.
It therefore did not decide adoption; that required the fixed, manually judged item–TELOS set
described above. With this initial E5-small setup, the first normal 30-day Comms refresh
evaluated 35 stored items semantically against five configured lenses with zero lexical
fallbacks. Repeating the same non-forced refresh evaluated zero items and skipped all 35 as
revision-current, confirming that the normal cache path does not spend a second model pass.

The repeatable public baseline lives in [`eval/`](eval/README.md): six manually judged,
synthetic DE/EN rankings, explicit 0–3 rationales, fixed acceptance thresholds and a runner
that calls only the loopback oMLX endpoint. It complements rather than exports the private
TELOS/Feed evaluation. E5-small failed at 4/6 useful top-1, `0.765` pairwise accuracy and
`0.865` mean nDCG. E5-base passed without changing the corpus at 6/6, `0.882` and `0.986`,
respectively. Its observed 30-vector run took 1.14 seconds; the local model uses 547 MiB on
disk and raised oMLX RSS to about 1.16 GB immediately after the batch, bounded by a 60-second
per-model idle TTL. The compiled Apple contextual runner took 0.75 seconds and peaked near
85 MiB RSS for the same 30 vectors; `NLEmbedding` took 0.67 seconds and peaked near 73 MiB.
Both resource wins are outweighed by their large quality losses.
After E5-base became the configured model, the first normal refresh semantically evaluated all
35 stored items; the immediate second non-forced refresh evaluated zero and skipped all 35 as
revision-current.

## Gmail mutation boundary

Every sweep is **strictly read-only**. The Google endpoints used by the sweep are:

- OAuth token refresh — `POST https://oauth2.googleapis.com/token`
- list inbox threads — `GET .../gmail/v1/users/me/threads?q=in:inbox`
- thread metadata — `GET .../gmail/v1/users/me/threads/{id}` (`format=metadata`)

`comms sweep --dry-run` and `comms sweep` are equally read-only against Gmail;
`--dry-run` only controls whether proposals are written to the local store.

Two writes exist only behind the authenticated Comms mutation router and only
for a thread already present in the local proposal store:

- Archive removes the `INBOX` label through `POST .../threads/{id}/modify`.
- Move to Trash calls `POST .../threads/{id}/trash`; it does not permanently delete.

The dashboard requires a second confirmation for either Gmail write. Dismissing
a proposal in Sjel changes only local state and is labelled accordingly.

## Commands

```
comms sweep [--limit N=25] [--dry-run]   # read-only inbox triage proposals
comms ingest <url>                       # ingest any supported URL (see Extractors)
comms feed [--stream news|media] [--days N=7] [--include-dismissed]
comms keep <id> | dismiss <id>           # feed item: set status (+ export if configured)
                                         # mail: write the distilled note; no Gmail write
comms summarize --pending                # retry summaries for items that lack one
comms export-sources [--dry-run]         # reconcile the feed library with Resources/Sources/
comms --help
```

`comms keep <id>` additionally writes a distilled markdown note (title, URL,
date, summary — never the raw transcript) when `keeper_export_dir` is
configured, refusing to overwrite an existing file.

`comms export-sources` writes every saved item into the configured vault and removes the
notes of items no longer saved. It needs `obsidian.root` and nothing else, and it is the
path to use when the server is down or has never run — see "The feed library in the vault".

Media ingest shells out to `yt-dlp` (metadata + subtitles) via argument arrays
only, downloads subtitles into a temp dir that is always removed, and never
persists raw audio/video. Summaries come from the shared `summarization`
inference role through an OpenAI-compatible chat-completions endpoint; if the
role is absent or unreachable the ingest still succeeds with `summary` left null (fill it later
with `comms summarize --pending`).

## Extractors

A URL is routed by host and path. Two checks run before any extractor does, at
the one door every caller goes through, because this path is reachable over
HTTP and its input is a URL a stranger chose:

- **Scheme.** Only http(s). `file://` would otherwise make yt-dlp and the
  article fetcher read the local disk.
- **Destination.** The host is resolved and every address it answers with must
  be publicly routable. Loopback, RFC 1918 private, link-local (which carries
  the cloud metadata service at 169.254.169.254), unique-local, carrier-grade
  NAT, multicast and reserved addresses are all refused, in the IPv4-in-IPv6
  spellings as well as the plain ones. Every Sjel service binds `127.0.0.1`, so
  without this an ingested link reaches them all: `http://127.0.0.1:8086/api/plans`
  is still http. `http_client`'s redirect policy re-runs both checks on each of
  at most three hops, so a `302` is not the way around them. Q74.

**The operator-visible cost: a LAN or loopback URL can no longer be ingested.**
A link to a NAS, a router page or a home dashboard pasted into `/ingest` or
found in a `vault_link_sources` file now fails with
`refused: <host> resolves to a non-public address` instead of being fetched.
The one escape is `ingest_allowed_origins` in the config — an exact
scheme-host-port, empty by default, and written down rather than inferred.
`tools/demo-up` is its only user in this repository (`demo/README.md`).

Not closed: DNS rebinding. The name is resolved once for the check and again by
the connector, so a record with a one-second TTL can answer public to one and
private to the other. Closing it needs the checked address to be the address
the socket gets — a pinned resolver or a custom connector — which is a larger
change than the guard, and the doc comment on `check_destination` says so
rather than implying the hole is shut.

| Source | Matches | Title / author | Transcript |
|---|---|---|---|
| `youtube` | youtube.com, youtu.be | yt-dlp metadata | subtitles (de/en), VTT stripped |
| `instagram`, `podcast` | instagram.com; `.mp3`/`.m4a`/"podcast" | yt-dlp metadata | subtitles when present |
| `github` | github.com/`<owner>`/`<repo>` only | repo description; owner | stars/language/license/topics + raw README |
| `arxiv` | arxiv.org `/abs/` or `/pdf/` | paper title; authors (3 + et al.) | the paper if readable, else the abstract |
| `reddit` | a `/comments/<id>` permalink | post title; `u/<author>` | selftext + top-level comments |
| `article` | everything else | `<title>`; none | visible text, tags stripped |

A deeper GitHub path (an issue, a blob, a profile) is an `article` on purpose:
the generic path already renders those, while the repo API answers what no HTML
strip can. arXiv keeps the version suffix, because v1 and v2 are different
papers to a reader.

### Fetching is not extraction

Routing and protocol live in `media.rs`; turning bytes into text lives behind
`extraction::Extractor`, one implementation per input class (`html`, `pdf`,
`text`). The two were the same function until #77, which is how the same HTML
stripper ended up written twice with two different bugs.

The readers themselves moved out on 2026-09-02 (PRD Q63 → B30): they are
`libs/extraction` now, promoted at the second real consumer
(`capabilities/transit` reads ticket files through the same shape). `crate::extraction`
is a re-export of that crate, so every call site here still reads the same, and
the PDF rung arrives through its `xberg` feature, which this capability enables.
What stayed: `normalize` (extraction produces raw, this produces clean, #86),
`extraction_eval` (it scores the extractor→normalizer seam, which is a comms
judgement), and `provenance::TranscriptSource`, which says whether the SOURCE
offered the document or a stand-in — no reader can tell, since the bytes look
identical either way.

**An arXiv item stores the paper, not its abstract.** arXiv renders LaTeX
submissions to HTML at `/html/<id>`, backfilled classics included, so
`fetch_arxiv` reads the paper through the HTML implementation already here —
no PDF conversion and no new dependency. HTML beats a PDF for this on its own
merits: LaTeXML keeps document structure, while PDF extraction has to
reconstruct reading order from a two-column layout and mangles maths doing it.

A 404 there is a clean signal rather than a judgement about a bad conversion,
and it falls through to [ar5iv](https://ar5iv.labs.arxiv.org), the older
LaTeXML pipeline with far wider backfill. Measured 2026-08-04 over the 24
newest `cs.AI`/`cs.LG`/`cs.CL` papers: arxiv.org served 21, ar5iv answered all
three misses plus a 2007 paper, with real body text. It is labs-grade, so it is
the fallback and never the first request.

What is left after both is papers with no LaTeX source at all, scans and
PDF-only submissions, mostly old. Those fall through to the PDF, read by
[xberg](https://github.com/xberg-io/xberg) `=1.0.5` behind the same trait
(#77). Failing all three, the abstract is stored. No raw PDF is persisted.

**The PDF rung is last for a measured reason.** On arXiv 2608.02599 (2026,
two-column) xberg returns correct text in the wrong reading order, columns
interleaved line by line. On arXiv 0704.0001 (2007, Type1 fonts) it drops
every `c`: "Abstrat", "Mihigan", "quantum hromo dynamis", in 11.7s against
0.3s for the same paper as HTML. Degraded full text still beats an abstract
for retrieval, and it is worse evidence than LaTeXML output, so the item
records which extractor produced it rather than leaving the two
indistinguishable.

Which route ran is recorded per item as `transcript_source` (`full-text` |
`abstract` | `unknown` for rows predating the distinction).

`transcript_source` is a separate axis from `content_status`, which measures how
much text there is: a long abstract is `full` + `abstract`, a one-line article
is `thin` + `full-text`. It is a column rather than an `Abstract:` prefix inside
the body because a prefix has to be parsed back out by every reader, and the
embedder would score it as if the paper had said it (#78).

**Reddit needs credentials and does not have them.** Verified 2026-07-28: the
`.json` view answers 403 to every unauthenticated caller, on `www` and `old`,
with a descriptive UA, an API-format UA and a browser UA alike. The parser is
written against the shape that endpoint still returns for an authorized caller;
reaching it needs a registered Reddit app and an OAuth token against
`oauth.reddit.com`, which is a secret the operator provisions (CONTRIBUTING.md#secrets). Until then a Reddit paste fails loudly instead of storing an empty item.

**A site that publishes its own text is a rung nobody has built.**
[llms.txt](https://llmstxt.org) proposes that a site expose a curated index of
its content beside `robots.txt`, with `llms-full.txt` carrying the text itself.
Were it adopted here it would sit ahead of the HTML rung for the sites that
publish one, on the same argument the arXiv HTML rung already makes against the
PDF: reading what an author curated beats reconstructing what a renderer emitted.

It is an idea, not a plan. There is no `upstreams.toml` verdict, nothing is built
on it, and the proposal's adoption in the wild is not measured here. It is
recorded beside the routing it would change rather than in a standalone link
file, because a link nobody re-reads is a link nobody acts on.

## Feed source collectors

`feed_sources[]` is the general-awareness source registry. `github-trending`
reads the bounded daily/weekly/monthly Trending page, extracts repository
identity, description and visible momentum metadata, and preserves the Trending
page as provenance. `arxiv` calls the official Atom query API with a configured
`search_query`, newest submissions first. Both clamp a run to at most 30 items.

Each source declares a `data_class`, and it is required with no default. That
declaration is what every item the source stores is classified with, and the
only way a feed item becomes `c0` at all — an item that arrives through
`/ingest` or a Vault link, where no collector declared anything, is stored `c1`
with method `legacy` and stays local. A collector may raise an item's class on
a later scan and may never lower one, so a `legacy` row has no machine route to
`c0`: only `POST /feed/:id/data-class`, with a written rationale, can lower a
class, and it answers 400 without one.

A scan upserts by the canonical target URL, preserving `keeper`/`dismissed`
state, then records `feed_origins` and `source_state`. Re-seeing an existing item
is reported as known rather than new. Background enrichment compares the
item/context/evaluator revisions before embedding or evaluating, so an unchanged
second scan does not spend another model pass.

## comms-server

Axum HTTP API (dashboard-origin-only CORS), port from `SJEL_PORT`
(runner-exported) → config → default **8083**. JSON contract consumed by a
dashboard panel.

`src/server/main.rs` is the composition root: configuration, route assembly,
loopback bind, and shutdown. Its sibling modules own one reason to change each:
`auth.rs` owns the shared-secret boundary; `contracts.rs` owns response
projections; `error.rs` owns the stable JSON error shape; `feed.rs`,
`content.rs`, `cloud.rs`, `source_handlers.rs`, `vault.rs`, and `triage.rs` own
their existing HTTP contracts; and `background.rs` owns the periodic task
lifecycle. `BackgroundServices` retains every task handle and aborts its tasks
when the server lifecycle ends. Blocking storage, inference, and network work
continues to cross `spawn_blocking` inside the owning workflow.

Routes:

- `GET /feed?stream=&days=&include_dismissed=` → feed items (no `transcript`). Each item
  states its `data_class` since 2026-09-06, because the one consumer that needs it reads the
  LIST: finance's `item_is_quotable` fails closed and drops every item whose class is
  unstated, so its decision inbox carried no feed evidence at all until this field existed.
  A row nobody classified answers `c1`, the undeclared default, never an absence. The test
  asserts the SERIALIZED key, since a rename here would break nothing in finance — it would
  silently empty the evidence again. The LIST only: `FeedFullItem`, which `GET /feed/:id` and
  `POST /ingest` answer with, still states no class, so a caller that builds a list row from
  an ingest response has none to carry (`dashboard/README.md`).
- `GET /feed/:id` → one reader item incl. `transcript`, every stored TELOS relevance match,
  the factorized evaluation and Vault provenance
- `GET /content/:source/:id` where source is `feed` or `mail` → the shared versioned
  content reader contract, including the stored `digest` when there is one. Reads only: a GET
  that quietly runs a local model turns opening an item into a two-minute wait. Mail supplies
  Gmail's bounded snippet as `content` and leaves `summary` null; a preview is not a generated
  summary, and the digest beside it says which is which.
- `POST /content/:source/:id/digest` `{"depth":"standard"|"detailed","focus":["..."]}` →
  generate or refine the local digest and replace it in place. Both fields are optional; an
  unknown `depth`, or a body that is not readable JSON, is a `400` rather than a quiet fall back
  to the default. Synchronous, unlike `POST /ingest`: this is a button the operator is watching.
- `POST /content/:source/:id/diagram` → one validated Mermaid diagram, drawn from the digest
  when there is one. A model answer that is not a diagram is a typed rejection, not a stored
  string.
- `POST /content/:source/:id/chart` → extract one set of comparable numbers as a table, every
  value verified verbatim against the source. `skipped_short` when the content holds none, which
  is the answer for most prose.
- `POST /content/digests/refresh` `{"source":"mail"|"feed","limit":25}` → the bounded automatic
  pass over items with no digest, a stale producer, or a retryable failure with attempts left.
  It never touches a row an operator refined.
- `POST /content/:source/:id/cloud-preview` → builds a bounded local preview. `c0` content
  is copied as-is; `c1` content is pseudonymized locally into typed tokens, including names from
  the operator's registry or contextual cues, addresses, links, phone/account numbers and
  token-like secrets. The
  token map stays local. The response lists the recognized entity types, names the limitations,
  and always reports zero provider calls. **`c2` and `c3` content has no preview**: the request
  is refused with 400, because a preview is a hashable, approvable object and producing one for
  content that may never leave the machine means the refusal has to be remembered again at every
  later step. The store agrees — `content_cloud_derivatives.original_data_class` accepts only
  `c0` and `c1`.
- `POST /content/:source/:id/cloud-approval` `{"preview_hash":"..."}` → regenerates the
  current preview, rejects a stale hash, and stages the exact reviewed derivative locally.
  It does not select or contact a cloud provider.
- `GET /content/cloud-providers` → lists only explicit `cloud_*` inference roles backed by
  non-loopback HTTPS endpoints with a reviewed provider name, data tier and billing boundary.
  It exposes safe role/model/policy metadata, daily usage/remaining calls and an availability
  reason, never the endpoint, account ID, key-file path or key value.
- `POST /content/:source/:id/cloud-queue`
  `{"preview_hash":"...","provider_role":"cloud_summarization"}` → validates the exact
  derivative against the role's reviewed data tier and materialized credential, then idempotently
  records a queue job. Queueing is local and performs zero provider calls.
- `POST /content/cloud-jobs/:job_id/run` → explicitly sends only the staged, hash-approved
  derivative to the selected role first, then to configured fallbacks whose reviewed tier admits
  at least what the selected one does — never a narrower tier — in stable order: narrowest tier
  first, then priority. Widening the candidate list is not widening permission; `tier_allows` is
  asked again about every candidate immediately before its attempt. Same-tier-only was the rule
  until 2026-08-30, and because `public` is the narrowest tier and is deliberately selected first
  for public work, it left a public digest with exactly one possible provider: 39 feed digests
  parked at the attempt cap against one rate-limited role while two healthy wider-tier roles were
  never offered the job. It runs the fixed
  `content-analysis-v1` task, validates and bounds the structured response, and persists the
  tokenized result or a safe retryable error. Credentials, credit expiry, input-token upper bound
  and the UTC daily request ceiling are revalidated immediately before every attempt. A
  policy-disabled role causes zero provider requests. Every actual request records role, model,
  exact approved derivative hash and bounded error/result provenance. Jobs never run automatically
  and stop after five total provider calls. The original content is not loaded into the dispatch
  path. On the local content-reader response, a successful result is rehydrated only if its
  approved preview still matches the current source revision; otherwise it remains tokenized.
  The token map is rebuilt in memory and is never stored. A completed result still performs no
  Calendar write: each resolved date or dated action has a separate reader action that creates or
  refreshes one non-blocking Calendar proposal through Calendar's external-entry contract.
- `GET /feed/evaluation/status` → configured local model names, cheap endpoint reachability,
  TELOS profile count, active semantic/lexical mode and persisted ledger counts; no secret or
  API-key value is returned
- `POST /feed/relevance/refresh` `{"days":90,"force":false}` → inspect at most 200 stored
  items and evaluate only missing or revision-stale rows against the configured TELOS lenses.
  The response separates `considered`, `evaluated` and `skipped_current`. `force:true` is an
  explicit maintenance override, not the dashboard default.
- `POST /feed/:id/status` `{"status":"keeper"|"dismissed"|"new"}` → sets a feed
  item's status (validated). `keeper` is a save and the other two are an unsave, so this
  route is also what adds and removes a note in `Resources/Sources/`
- `POST /ingest` `{"url":"..."}` → `201` with the stored item, or `400` with a
  reader-facing `error`. Answers as soon as the item is stored and summarizes
  behind the response, so a fresh item legitimately comes back with
  `summary: null`; it appears on the next feed read. A paste box that blocks for
  the summarizer's two minutes is a paste box nobody uses.
- `POST /vault-links/scan` → metadata-only candidates from declared Vault sources
- `POST /vault-links/import` `{"source_id":"...","url":"..."}` → re-validates the
  candidate against the current source, fetches it, stores provenance and then enriches it
- `GET /sources` → enabled/configured general Feed collectors and last-run state; personal
  query text is not returned
- `POST /sources/scan` `{"source_id":"github-trending-daily"}` → scan one enabled
  collector; `{"source_id":null}` scans all. Returns per-source fetched/new/known counts and
  enriches only revision-stale items behind the response.
- `GET /triage?status=proposed` → triage items
- `POST /triage/sweep` `{"limit":100,"cursor":null}` → fetches one inbox page without Gmail
  lifecycle mutations, stores proposals, and locally analyzes event/ticket candidates for grounded
  Calendar proposals.
  The receipt includes counts only, never extracted mail text, and returns the opaque cursor for the
  next page. The receipt reports the people registry's state and name count, because this is the
  path that *persists* rows: a pass run with the overlay unmounted raises nothing to `c2` and stores
  the verbatim metadata of mail
  that names a vault-known person, and its counts look exactly like a pass that found nobody. The
  unattended schedule logs the same line
- `POST /triage/relevance/refresh` `{"limit":200}` → scores stored pending mail against
  TELOS through loopback-only local inference or the labelled lexical fallback; TELOS is read-only
- `POST /triage/data-class/refresh` `{"limit":500}` → locally classifies stored pending mail
  from sender, subject, snippet and category only; no provider is called and human overrides are
  preserved. A row this pass lands on `c2` or `c3` has its stored subject and snippet redacted in
  the same pass, and the response counts them as `redacted` — class and redaction are one decision
  in the sweep (`intake::from_thread`) and one decision here, so no row is left labelled Redacted
  while still holding the text. The response also reports the people registry's state, because a
  registry that did not load raises nothing to `c2` and would otherwise be indistinguishable from
  one that found nobody
- `POST /triage/bulk` applies one reviewed action to at most 100 stored proposals and reports
  per-item failures. `categorize` uses `stream`, `set-data-class` uses `data_class`, and
  `dismiss`, `archive`, or `trash` need only the selected `ids`. `set-data-class` also reports
  `narrowed`: how many of the rows it raised had stored text to remove.
- `POST /triage/:id/status` `{"status":"proposed"|"approved"|"dismissed"}`
  → local proposal state only; Gmail lifecycle states cannot be forged through this route
- `POST /triage/:id/stream` `{"stream":"aktiv"|"issue"|"feed"|"werbung"|"belege"|"steuern"|"sonstiges"}`
  → records a persistent human category override without resolving the proposal
- `POST /triage/:id/data-class` `{"data_class":"c0"|"c1"|"c2"|"c3"}`
  → records a persistent human trust-class override; the dashboard labels these Public, Mine,
  Others and Secret. Setting `c2` or `c3` redacts the row's stored subject and snippet in the
  same transaction — the operator selecting Secret is saying that text may not stay — and the
  response reports `narrowed` so a clean row is distinguishable from a skipped one
- `POST /triage/:id/gmail` `{"action":"archive"|"trash"|"restore"}` → performs the
  explicit action through a durable local intent. Replays first check Gmail's metadata labels,
  so a process failure after Gmail success can finish locally without duplicating the mutation.
  Trash retains its local copy for 30 days; restore returns either state to Inbox.
- `POST /triage/:id/gmail-job` `{"decision":"retry"|"cancel"}` → explicitly reopens the
  bounded retry window or cancels an action after five automatic failures. Queued work cannot be
  canceled while it may be in flight.
- `POST /triage/reconcile` → retries due Gmail intents and compares up to 200 stored threads with
  Gmail metadata labels. It updates Sjel Archive/Trash/Inbox state without fetching bodies or
  attachments. Gmail 404/410 becomes an explicit Missing state that retains Sjel's local record;
  a Trash cleanup deadline remains active. The server runs the same bounded maintenance on its
  configured interval.
- `GET /health` → liveness. Answers from the process alone, so a start completes without a
  database and an unreachable store does not read as a crash.
- `GET /ready` → readiness: liveness plus a reachable database, `503` when it is not. This is
  what `sjel-status` judges availability on, and what the dashboard reports.

Binds `127.0.0.1`, not `0.0.0.0`: `/ingest` makes the server fetch a URL on
request, so anything that can reach the port can use it to reach whatever the
host can. Remote access belongs in front of the process (Tailscale), not in an
open bind. Since Q74 the second half of that sentence is narrower than it
was — the destination check under "Extractors" refuses a fetch to this machine
or this network — but it is still the right bind, because the guard bounds
where a request can go and not who may make one.

## Configuration

Resolved in order (mirrors scouting):

1. `$SJEL_COMMS_CONFIG` (explicit path to a JSON file)
2. `$SJEL_PERSONAL_ROOT/config/comms.json` (the private overlay)
3. `capabilities/comms/comms.config.json` (gitignored dev fallback)

Every field is optional; the tool runs zero-config against the shared local
store with only the built-in classification heuristics.

The nineteen tables live in the shared SQLite file — `SJEL_DB_PATH`, else
`$SJEL_PERSONAL_ROOT/data/axon/axon.db` — under the table prefix `comms`, so
`comms.feed_items` is `comms_feed_items` (`libs/sjel-store/README.md`). PRD Q45
(2026-08-27) moved them there from a Postgres schema. The path is a deployment
fact rather than a capability one, so there is no `database_url` field any more:
`capabilities/places` joins mail against this data, and a file per capability
would drop that join.

Fields: `google_env_path` (default `$SJEL_PERSONAL_ROOT/config/comms.env`), `port`,
`gmail_maintenance_minutes` (default `15`; `0` disables the automatic pass),
`relevance {profile_paths}`. Model roles come from the overlay's shared
`inference.json`; see `libs/inference/inference.config.example.json`. The active producer revision
includes both embedding and reranking roles, so a model change invalidates stale evaluations. A
profile note may define a single-line `relevance_query` in
its frontmatter; this is the inspectable embedding input and changes that profile's revision,
while `summary`, `current_focus` and `category_affinity` remain reader-facing metadata,
`vault_link_sources[] {id, path, heading?, enabled}`,
`feed_sources[] {id, adapter, enabled, query?, language?, since?, limit, data_class}`,
`rules[]`, `keeper_export_dir`,
`ingest_allowed_origins[]` (exact `scheme://host:port` strings that `/ingest` may fetch even
though they resolve inside this machine; **empty by default and on every real machine**, and an
entry clears that one origin rather than the host — see "Extractors"),
`obsidian {root}` (the vault the feed library projects into; absent leaves the bridge off,
and the same block shape as `trips.json`). See
`comms.config.example.json`. Nothing personal lives in this repo — sender
addresses and personal rules belong in the overlay.

Every HTTP route except `/health` and `/ready` requires the shared token referenced
by `api_secret_file` — reads included, since a feed entry and a mail proposal are
personal content and the loopback bind is no longer treated as the boundary. The check
itself is `libs/sjel-server`'s inbound gate, shared with every other capability;
`api_secret_file` still wins over the deployment-wide `SJEL_INBOUND_TOKEN_FILE`, and an
unconfigured token closes those routes with `403` rather than opening them. The local
dashboard never puts the token in its browser bundle: its Vite proxy resolves the same
config at startup, rejects cross-origin mutations, and adds the bearer header on the
server side. Restart both `comms` and `dashboard` after a token rotation.
Browser-extension and direct HTTP clients must supply the token themselves.

Google credentials (`GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`,
`GOOGLE_REFRESH_TOKEN`) live in the overlay `comms.env`; mint the refresh token
once with `auth/get-refresh-token.ts` (bun). Token values are never logged, and
the DB connection string is redacted before any display.

## Schema (`comms` schema, shared instance)

`src/store.rs` is the stable `Store` facade and owns the public persistence
types. Its implementation lives in workflow modules under `src/store/`:
migrations and pool access; triage and Gmail actions; reviewed cloud jobs and
digests; feed ingest and relevance; evaluation and travel context; capture
origins; collector state; and database-row mapping. SQL stays beside the
workflow that owns it, while callers continue to depend only on `Store`.

- `triage_items` — one row per inbox thread: `id` (gmail thread id) PK,
  `from_addr`, `subject`, `snippet`, `internal_date`, `stream` (CHECK), `rationale`,
  `status` (`proposed`/`approved`/`archived`/`trashed`/`missing`/`dismissed`; `executed` is retained
  for legacy rows), `gmail_action`, `gmail_action_at`, `purge_after`, `first_seen`, `last_seen`,
  observed Gmail location/time and durable sync state/error, plus trust-class value,
  rationale, method and classifier revision. Human trust-class
  overrides survive later rule refreshes.
- `gmail_action_jobs` — content-free durable intent and bounded retry ledger for Archive, Trash,
  and Restore. Only one queued action may exist per thread; five failed attempts move it to the
  explicit attention state. An operator can retry that exact action or cancel it; canceled history
  remains auditable without blocking a later action.
- `triage_relevance` — additive per-thread/per-lens ranking rows containing the raw score,
  rationale, truthful scoring mode, profile revision and scoring time. Replacing these rows
  never changes the proposal category/status or its TELOS source.
- `content_cloud_derivatives` — the latest explicitly approved bounded derivative per source
  item, including source revision, preview hash, original/derivative class, transformation,
  redaction count and approval time. It is local staging state.
- `content_cloud_jobs` — idempotent provider intent and execution ledger for an exact approved
  derivative. Jobs move through queued/running/succeeded/failed only after an explicit request,
  count at most five provider calls, and retain a bounded structured result or safe error. No
  background worker sends queued content automatically.
- `content_cloud_attempts` — one immutable-identity row per actual provider request, including
  sequence, role, model and approved derivative hash plus bounded result/error provenance. It is
  also the local UTC-day usage ledger; candidates rejected by credentials, expiry or ceilings
  create no row because they made no provider request.
- `feed_items` — one row per ingested URL: `id` (sha256 of canonical URL) PK,
  `stream` (`news`/`media`), `kind` (`youtube`/`instagram`/`podcast`/`article`/
  `mail`/`github`/`arxiv`/`reddit`), `title`, `url`, `author`, `summary`,
  `transcript`, `day`, `created_at`, `status` (`new`/`keeper`/`dismissed`).
  Widening that `kind` set needs a `DROP CONSTRAINT` + re-`ADD` migration, not
  an edit to the `CREATE TABLE`: `IF NOT EXISTS` never touches an installed
  table's constraints, so the edit alone would work on a fresh database and
  silently reject the new kinds on every existing one.
- `content_digests` — one row per `(source, item_id)`: the generated text, its state, the rung
  and depth that produced it, the operator's focus terms, the producer, how much source was
  measured, the redaction count, the retry ledger, the Mermaid columns and the extracted chart
  table. One table rather than
  a column on each item table: a digest is derived data with the same axes everywhere, so three
  migrations would buy only drift. **No raw source is kept here** — a mail body is fetched,
  digested and dropped inside one call, and this row is all that survives it.
- `source_state` — per-source run bookkeeping.
- `feed_relevance` — additive per-item/per-lens match rows: raw score, rationale,
  truthful scoring mode and a profile revision fingerprint. Refresh replaces only relevance;
  it never changes the feed item's triage status.
- `feed_evaluations` — one revisioned, inspectable overall evaluation per item: normalized
  score, deterministic explanation, scoring mode and the item/context/evaluator cache key.
- `feed_evaluation_factors` — ordered normalized factors with label, value, weight and
  rationale. It is normalized rather than JSON so later trip/deadline factors remain queryable
  and individually inspectable.
- `feed_quality_flags` — the latest deterministic review signals per item. Each row stores the
  signal name, human-readable reason, inspectable evidence and derivation time. An explicit
  refresh replaces an item's complete set; the read route only returns stored rows.
- Stage provenance stays beside the value it describes: extraction tier/revision in
  `feed_raw_content`, normalization and summary tier/revision in `feed_items`, and ranking tier
  beside the existing revision tuple in `feed_evaluations`. Tiers are ordered
  `legacy < deterministic < model < human`; equal-tier reruns replace idempotently and a lower
  tier cannot downgrade the stored value.
- `feed_origins` — one row per exact Vault source/reference that introduced an item. An item
  may retain multiple origins without duplicating its canonical feed row.

Both item tables use a **status-preserving upsert**: `status` is set only on
first insert and is deliberately absent from the `ON CONFLICT DO UPDATE`, so a
human's triage/keeper decision survives the same item being re-swept or
re-ingested (`upsert_preserves_status_across_refetch_*` tests prove it).
The reader shows the human keeper/dismiss verdict above every processing
producer; migrated values whose producer cannot be recovered are labelled
`legacy-unknown` rather than assigned a guessed model or revision.

## Computed quality review

`POST /feed/quality/refresh` examines at most 500 stored items and replaces their review flags;
`GET /feed/quality` reads the resulting queue. The dashboard exposes both under **Feed → Review**.
The computation reads only stored content status, capture/extraction provenance, raw and canonical
text, summary retry state, and whether a ranking row exists. It never calls an inference provider,
never treats a model's self-confidence as evidence, and never changes the human Feed status.

The public defaults in `comms.config.example.json` come from the frozen extraction corpus rather
than an intuition: its passing input classes retain 39.7–89.6% of total raw text, preserve
100% of judged useful text, and leak 0% of judged boilerplate. Operational items do not have
per-item human judgements, so the review signal deliberately uses the rounded 39–90% observed
total-retention envelope and a 0% residual-normalizer-rule threshold as suggestions for inspection,
not correctness verdicts. Two failed summary attempts warn before the existing three-attempt cap.

**One fixture class starts before extraction.** Every other one stores the text an extractor
already produced, which is why the gate could not see the defect it exists to catch: the real HTML
path emitted a single line, every normalization rule is guarded by a max line length, and the
corpus reported 0% leakage while production stored consent walls verbatim as article bodies. The
`html` class holds a page (`eval/fixtures/page.html`) and runs extraction then normalization, so a
change to either is scored end to end — and a replacement extractor (#77) can be measured against
the built-in one on the same pages.

## Tests

`cargo test -- --list` counts them; the hand-written number here was wrong twice, so it is
gone (CONTRIBUTING.md#documentation-stays-owned-and-current). The store tests need no server since
PRD Q45: each takes a temp file of its own, which is the isolation the per-pid
schema used to buy, without a schema anyone can leak into a backup.

The config test that clears `SJEL_PERSONAL_ROOT` still restores it on drop, and
that is still load-bearing: Rust runs a crate's tests as threads of one process,
and an unrestored `remove_var` leaves every later store test resolving a
different file from the one it just wrote to. The same fix applies in `scouting`
and `transit`.
