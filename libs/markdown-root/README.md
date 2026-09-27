# libs/markdown-root

A declared markdown root, and the only way to get a file out of it. Spine-owned
shared code with no domain of its own — see
[Three architectural nouns](../../CONTRIBUTING.md#three-architectural-nouns).

Two capabilities read markdown out of a knowledge store the operator points them
at. Scouting resolves opportunity notes and interest profiles; calendar imports
event notes. Both take a root path from the private overlay plus a glob relative
to it, and both have to answer the same question before reading anything: is
this file actually inside the root that was declared?

## Why it exists

The answer used to be "assume so". `capabilities/scouting/src/score.rs` and
`capabilities/scouting/src/sources/obsidian_md.rs` each carried their own
`root.join(pattern)` resolver. They had already diverged — one handled an exact
file, the other only a directory — and neither checked containment, so a pattern
of `../../.ssh` resolved happily and a symlink pointing out of the vault was
followed without comment. Calendar's markdown importer would have been the third
copy of that.

That is the repo's stated extraction rule met exactly: the same shape in more
than one place, diverging, in a way no one decided.

## What the type guarantees

- A `MarkdownRoot` exists only for a directory that exists, canonicalized once.
- Every path handed back is inside that directory, proven **after** symlink
  resolution rather than by string prefix.
- A pattern containing `..` or an absolute path is refused before any read, so
  no evidence of what lies outside the root leaks through an error message.
- Anything that would escape is an error naming the offending path — never a
  quietly dropped file. An operator cannot fix a config the resolver hides.
- `relative_id()` returns a slash-separated, root-anchored identity, so the
  `(source, external_id)` a note is imported under does not depend on which
  machine imported it.

## What it deliberately is not

**Not a glob engine.** The patterns in play are `Some/Dir/*.md` and one exact
`Some/File.md`, which is all the config shape has ever declared. A real matcher
would be a dependency, and this crate stays std-only because those two shapes
cover the declared contracts.

**Not a parser.** What a markdown file *means* — frontmatter, an event, an
interest profile — belongs to the capability that declared the root.

**Not tilde expansion.** `sjel_config::expand_tilde` owns that; the caller
applies it before declaring a root.

## Writing back: the marked region

`region.rs` is the other half, added for #138. `capabilities/trips/README.md`
specified it in prose a month earlier and nothing implemented it: regenerate only
a marked Axon-owned section, preserve everything outside it, and record a conflict
rather than choosing between two changed revisions.

```text
<!-- axon:begin owner=finance v=1 sha=1a2b3c4d5e6f7890 -->
anything the machine generated
<!-- axon:end owner=finance -->
```

HTML comments, so Obsidian renders them as nothing. The owner sits on both markers,
which lets two capabilities hold a region each in one note without knowing about
each other.

The hash lives in the marker so the file is self-describing: the only input the
"did a human touch this" check needs is the file itself, not a sidecar that can go
missing or come back from an older backup. It is FNV-1a, because the question is
change detection and not forgery, and a cryptographic digest would be a dependency
this crate exists to avoid.

`apply()` is a pure function from string to string. The caller keeps the read, the
write, and the decision about what a conflict should do to the run. Two changed
revisions produce `RegionOutcome::Conflict` carrying both. Picking one silently is
the exact failure this was built to prevent.

## Writing back: the whole-file projection

`projection.rs` is the case the region leaves open — **there is no human note to write
a region into**. PRD Q31 (2026-08-23) named it pattern B, ruled it second-choice, and
gave it one home: `Resources/Axon/`. Q49 (2026-08-27) then ruled that the mechanism is
shared rather than per-capability, which is why it is here and not in `trips`.

The file carries a header instead of markers:

```text
<!-- axon:projection owner=trips v=1 -->
<!-- Axon generates this file and overwrites it whole. … -->
```

Placed **after** the frontmatter, because Obsidian reads frontmatter only when the
opening `---` is the file's first line.

There is no hash and no conflict outcome. The region's hash protects a human's prose
in a file they own; a projection is not that file, and pretending a safety copy can be
merged would be the wrong promise. What is guarded is the **path**: a file that carries
no projection header is somebody else's, and `ProjectionOutcome::NotOurs` refuses it
rather than overwriting it. That is also Q31's promotion path — a human writes a real
note about the subject, their note takes the path, and the capability moves to a marked
region inside it.

Bytes that already match are not written, so a scheduled export does not produce a
commit per run in the vault's git history.

## Why the writer lives here rather than in its own lib

This crate already owns more than root resolution: `frontmatter()` has been here
since the extraction. A writer also needs a containment-proven path before it
touches anything, so splitting them would put one concern in two crates and leave
the writer depending on the reader regardless.
