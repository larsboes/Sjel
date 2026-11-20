# posts

Post drafts with text revisions, ordered Photos media, two-photo collages and numbered exports
for upload. The file contract, data flow and media rules are in
[`docs/architecture/posts.md`](../../docs/architecture/posts.md).

## Use

```bash
crates/posts/posts                       # usage
crates/posts/posts new event-recap --title "Event recap"
crates/posts/posts media add-selected event-recap   # what is selected in Photos
crates/posts/posts media pair event-recap 3 --accent lowpoly
crates/posts/posts export event-recap    # -> <overlay>/data/posts/event-recap/out/
crates/posts/check                       # every verb against a throwaway overlay
```

The Mac app's Posts page (`g j`) drives the same launcher through `sjel-status`.

## Why this shape

The launcher is the only writer, so the CLI and the page cannot disagree. External tools run as
plain argv with validated IDs: no shell parses a Photos ID or a path.
