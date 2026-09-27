# libs/content-item

The `content-item-v2` reader contract in Rust. Spine-owned shared code with no
domain of its own — see [Three architectural nouns](../../CONTRIBUTING.md#three-architectural-nouns).

`schemas/content-item.schema.json` is the normative artifact. This crate exists
so that the capabilities emitting it cannot drift from one another by hand.

## Why a shared contract and not a shared table

A unified **read** contract pays for itself: the dashboard owns one renderer,
and a new source costs an adapter instead of a second reader. A unified
**store** would not. Calendar enforces an exclusive `ends_at` after `starts_at`,
a `(source, external_id)` unique index and a commitment CHECK; mail enforces a
retention window; feed enforces neither. Merged into one table, every one of
those becomes a conditional the database cannot express, and the guarantees are
gone.

So each capability keeps its own tables and projects into this shape on read.

## Ranking is a property of the source

`relevance` and `evaluation` exist because feed is an unbounded inbox that has
to be ranked. Calendar is not — an entry is something the operator already
decided about, and its triage axis is `commitment`, surfaced through `status`.

A source with no ranking leaves those fields empty. It does not synthesise a
score: a `0.0` on a committed event reads as a judgement of the event, which is
how `matched_focus: "Scholarship Profile", score: 0.0` ended up sitting on a real
calendar entry before this contract existed.

## Consumers

`comms` (feed, mail) and `calendar` (entries).

`tasks` was the third until PRD Q48 (2026-08-27) retired it. It inherited a data class
rather than deriving one, which is still the shape this crate is for; the capability that
demonstrated it is gone.

Consumers declare the workspace path dependency in Cargo. They share one Rust type, while
capability-to-capability communication remains the serialized JSON contract;
the shared crate is not permission to call into another capability's store.

## Adding a source

1. Add the value to `source` in the JSON schema and to the `allOf` branch that
   nulls the extensions it does not own.
2. Add an extension struct here if the source has fields nothing else does.
3. Project in the capability that owns the data. Do not reach into another
   capability's store to fill a field.
