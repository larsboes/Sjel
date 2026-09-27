# entities

Axon's system of record for people, organisations, places and the operator's own context.
PRD Q117 (2026-09-25) ruled that Axon holds these facts and that Obsidian, Google Contacts
and TELOS are adapters, each optional. Prose stays in Obsidian; an entity links to its note
through `note_ref`.

## Model

Q117 ruled the shape: a typed core per kind, plus fields the operator declares.

- **Kinds:** `person`, `organisation`, `place`, `self`. `self` is the operator, for the
  personal context TELOS holds today.
- **Fields:** a registry per kind (`GET /api/fields`). Each field has a type (`text`,
  `bool`, `date`, `number`, `enum`, `emails`, `phones`, `url`) and a data class (C0–C3,
  PRD §6.1). Every value is checked against its field on write. A key the kind does not
  have is refused. The operator adds fields with `POST /api/fields`, and no code change is
  needed. The built-in person fields are `sleeping_option` (`none`, `ask`, `yes`),
  `sleeping_note`, `emails` and `phones`.
- **Values** record where they came from: `operator`, `obsidian`, `google`,
  `places-register` or `import`.
- **Dated facts** place an entity somewhere. `home_base` holds from `valid_from` on, and a
  move is a new fact, so the old home keeps its dates. `away` holds from `valid_from` to
  `valid_to`, and wins over the home base on the days it covers. `GET /api/located?day=`
  answers where each person is on a day.
- **External ids** link an entity to its id in another system, so a sync updates the
  entity instead of adding a second one.

A fully generic attribute store was rejected (Q117) because it checks nothing on write.

## Coordinates

A fact's place text goes to `capabilities/places` (`POST /api/geocode`) for a coordinate.
Places owns the provider, its rate limit and the cache. Only the place text is sent, never
the entity's name. The fact is stored even when places is down or finds nothing, and the
reply's `geocode` says which. Refusing the fact would lose what the operator typed.

## Data

C2 throughout: facts about named people. The rows live in the overlay's database
(`sjel_config::database_path`). The server has no CORS, and the shared origin guard refuses
a foreign browser origin. The dashboard reaches it through its same-origin proxy.

## Import

`entities-server import-obsidian [--dry-run]` creates one person per `Atlas/People` note,
as `capabilities/vault` reads it (`GET /api/people`). `home` becomes a `home_base` fact.
`host: yes` becomes `sleeping_option: ask`, because a note saying someone could host is a
reason to ask, not a booking. `host_note` becomes `sleeping_note`. A note that is already
linked is skipped, so re-running the import changes nothing.

## Duplicates, merge and sources

`GET /api/duplicates` lists probable pairs, strongest first: a shared email or phone, the same
name, or a first name that begins a full name. Only first-name pairs that share a field go to
the on-device model, and it sees only the shared fields; measured on 2026-09-25, shown whole
records, it invented differences for fields only one side had.

`POST /api/entities/:id/merge` takes `{ other, name?, pick? }`. Values the kept record lacks
come across; where both have one, the kept value stays unless its key is in `pick`. Facts and
Google/Obsidian links move. Every merge stores the removed record whole, with its links, in
`entities_merges`, because a merge is otherwise lossy exactly where both sides disagreed.

`GET /api/entities/:id/sources` asks each linked system what it says now: the Google contact
(live) and the Obsidian note (through vault). The People page shows where they differ from
Axon, and "Use this" writes the source's value back with that source as its owner, so the
next sync keeps it current. On 2026-09-25 this found 9 values across 8 people that the first
18 merges had dropped.

## Not built yet

- **Writing back to Google.** The sync is read-only today. The groundwork is there: every
  value records its source, the contact's `etag` is stored, and `sources` shows the diff. A
  write-back would push operator-owned values for the fields Google has (emails, phones,
  birthday, company, role), needs the `contacts` scope instead of `contacts.readonly`, and
  sends C2 to Google, so it needs a ruling on which fields may go (PRD §6.1).
- Undoing a merge from the app; the snapshot in `entities_merges` makes it possible.
- Folding the places companion register (`places_person_places`) into dated facts.
- Changing a field's type after values exist.
