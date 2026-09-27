# libs/summarize

One home for **how much digest a thing is worth**.

A shared library, not a capability: no domain of its own, no upstream verdict, no CLI
(CONTRIBUTING.md#three-architectural-nouns). Consumers declare an `sjel-summarize` path
dependency in the workspace.

## Why it exists

`comms` had one summarizer, one prompt and one `max_tokens: 800`, reachable only from
`media.rs` and only for a feed item that had a transcript. A forty-word mail and a forty-page
paper got the same three bullets — the first longer than its source, the second discarding
most of it. Mail got nothing at all.

Three capabilities want the same artifact, so it stopped being feed's private helper.

## A digest is not a summary

Deliberately a different noun. `summary` on `content-item-v2` is what the *source* said it
is: calendar reads it from the entry's own description, and overwriting that with generated
prose destroys the only verbatim text an entry has. A **digest** is what the local model wrote
about the thing, carrying its own shape, directive and provenance.

## The ladder

The shape follows the source's length, and nothing else chooses it:

| Source | Shape | Asked for | Ceiling |
|---|---|---|---|
| under 600 chars | `none` | nothing — see below | — |
| 600–2,499 | `brief` | two to three bullets | 200 tokens |
| 2,500–8,999 | `standard` | bullets plus one context sentence | 500 tokens |
| 9,000+ | `sectioned` | grouped bullets under at most four headings, plus context | 1,000 tokens |

Below the floor **no digest is produced at all**. A paraphrase of a two-line mail is a second
thing to read, not a shorter one, and it costs a model call to make. `Outcome::SkippedShort`
is a verdict, not a failure, and it is never retried.

**"More detailed" is a rung, not an adjective.** Asking a model to be more detailed gets you a
longer version of the same guess. `Depth::Detailed` moves the shape exactly one step up this
same ladder — `brief → standard → sectioned`, saturating — which changes both the requested
structure and the ceiling, and stays inspectable afterwards because the stored row records
which rung produced it.

That rule also gives the short floor its escape hatch: `none → brief`. The automatic pass
skips a short source; an operator looking at the item can force one anyway, because they can
see something the character count cannot.

**Focus terms** are the second half of the affordance: up to eight bounded, de-duplicated
operator terms, named individually in the prompt with an explicit instruction to say so rather
than invent when the source is silent on one. They are stored with the digest, so the reader
can show what was asked for rather than leaving a differently-shaped digest unexplained.

## The remote refusal lives here

`digest()`, `diagram()` and `chart()` take the caller's data-class verdict as a `Reach`.
Anything the caller's cloud gate did not admit arrives as `Reach::LoopbackOnly`, and a
non-loopback target is then refused outright — `Outcome::RemoteRefused`, never a quiet
downgrade and never retried into success. The check sits at the one place that makes the
request, because a policy enforced by each caller separately has as many holes as it has
callers.

`Reach` is an enum rather than a `bool` because the value is a verdict about one item's
stored class, reached against one specific endpoint. `digest(t, text, &d, false)` records
neither half, which is the shape the two hardcoded `"public"` literals in the feed path had:
both compiled, both read as deliberate, and neither had asked anything.

## Diagrams are validated, not trusted

A model asked for a diagram will cheerfully answer with prose. `extract_mermaid` unwraps a
```` ```mermaid ```` block or accepts a bare diagram, then requires the first meaningful line
to start with one of twelve known Mermaid headers. Anything else is a typed rejection.
Unrenderable text stored in a diagram column fails at the reader, which is the hardest place
to work out what went wrong.

## Charts: the gate is the feature

`chart.rs` pulls one set of comparable numbers out of prose. The interesting part is not the
extraction, it is the refusal: **every value must appear verbatim in the source** before it
reaches a figure. `value_appears_in` tries the renderings prose actually uses — the plain
decimal, the German decimal comma, the bare integer, thousands separators both ways — and a
number it cannot find is dropped. Conservative on purpose: a real value written in some form
not listed costs one row, while the opposite mistake puts an invented number in a chart.

Three more constraints follow from what the data and the palette can honestly support:

- **One measure.** The figure palette is a print palette; even two of its hues fail a
  categorical-separation check at the normal-vision floor. A single series needs no categorical
  scale and no legend, so the palette is never asked to do what it cannot.
- **The form is derived.** An ordered run of three or more categories gets a line, everything
  else bars. Two ordered points stay bars, because a line through two dots implies everything
  between them.
- **Data, not a specification.** The output is rows plus one derived mark. The consumer compiles
  the chart, so scales, transforms and data URLs are never something a model reaches.

"Nothing to chart here" is `Outcome::SkippedShort`, the same verdict the digest ladder uses for
a source too short to bother with. It is the right answer for most content and is not an error.

## Dependency rule

`serde_json` and blocking `reqwest`, and nothing else. Keeping this explicit dependency
surface small limits the runtime and review cost inherited by every consumer.

It deliberately does **not** name `libs/inference`'s types. A caller builds a `Target` from
whatever role it resolved, so a capability that has no inference module still compiles:

```rust
let target = role.map(|role| Target {
    endpoint: role.chat_completions_endpoint(),
    model: role.model.clone(),
    api_key: role.bearer_key(),
    loopback: role.is_loopback(),
    // `None` is unbounded, which is what every caller did before this existed.
    // A loopback target should carry a `LocalGate`: it is how concurrent
    // callers stop pushing one GPU past its memory ceiling. The mechanism is
    // yours to supply, because this lib may not hold a database handle — see
    // the dependency rule above.
    gate: None,
});
let outcome = summarize::digest(target.as_ref(), text, &directive, reach);
```
