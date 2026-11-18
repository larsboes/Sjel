# Posts

Open work for drafting, reviewing and later publishing social posts, LinkedIn first. Priority:
[`PLAN.md`](../../PLAN.md) Priority 3. No code exists yet.

## Verdict under the build rule

The rule is in [`docs/product.md`](../product.md#what-sjel-builds-ingests-and-links-to).

| Part | Verdict | Reason |
|---|---|---|
| Post draft | Build | A draft joins text revisions, ordered photos, mentioned people and the event. No single tool holds that join. |
| Past posts and connections | Ingest | LinkedIn holds them. A review compares a draft with past posts, and mentions resolve to people. |
| Photo editing beyond a grid collage | Link out | The photo library and design tools do this well. |
| Publishing | Link out until package 6 | The LinkedIn editor does this well. Package 6 adds the API write path. |

## Data

One folder per post at `$SJEL_PERSONAL_ROOT/data/posts/<slug>/`:

| File | Content |
|---|---|
| `post.md` | Current text. Front matter: status (`sketch`, `draft`, `ready`, `published`), event, published URL. |
| `revisions/` | Each sketch and each reviewed revision, numbered. |
| `media.toml` | Ordered list: Photos media item ID, role (`cover`, `body`, `collage-part`), people shown. |
| `out/` | Exported, numbered files and collages, ready for upload. |

## Work packages

1. **Draft folder.** `posts new <slug>` and `posts status [slug]`. Done when a new folder passes
   a schema check and `status` lists every post with its state.
2. **Ordered media export and collage.** `posts media export <slug>` exports the `media.toml`
   items through Photos, so iCloud originals download and edits apply, resized to a 2400 px long
   edge and numbered in post order. `posts collage <slug> --layout <name>` crops to fill and
   composites with ImageMagick. This package closes the tooling entry "Export Photos library
   items in a chosen order and build a collage" ([`tooling.md`](tooling.md#open)). Done when one
   command produces the numbered files and the collage, and the Photos library stays unchanged.
3. **Past-post ingest.** Import `Shares.csv` and `Comments.csv` from a complete LinkedIn data
   export, and the matching Member Data Portability API domain. Extend the existing reader in
   `crates/cv/src/linkedin.rs` (`import`, `pull`) instead of adding a second LinkedIn client.
   Done when a full export imports idempotently and each post keeps its date and source.
4. **Review skill.** A Pack skill runs sketch, review and revision. The review compares a draft
   with ingested past posts (openers, thank-you lists, closings) and applies `human-writing`.
   Done when a review cites the past posts it compared and adds no fact the sketch lacks.
5. **People.** Mentions in `post.md` resolve to `entities` people. `Connections.csv` imports into
   `entities` with its source. The `entities` capability owns the people and their skills. This
   package only links to them. Done when each mention resolves to one entity or is reported as
   unresolved.
6. **Publishing.** The LinkedIn Posts API with the `w_member_social` scope. Each publish waits for
   an explicit owner Allow, because a post is outward-facing. Starts after packages 1 to 4 ship.
   Done when a published post records its URL in `post.md` and a refused Allow sends nothing.

## Open questions

- Owner of the LinkedIn reader: it stays in `cv`, or it moves to a connector that `cv`, `posts`
  and `entities` share. Decide before package 3.
- Network scope: the schema stays network-neutral, but only LinkedIn has an importer.
