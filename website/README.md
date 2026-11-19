# website

The public site at `larsboes.github.io/Sjel/`, built from tracked files by `website/build`.
Nothing on it runs a capability, and nothing is fetched while a page renders.

| Path on the site | Source | Built by |
|---|---|---|
| `/` | hand-written copy, counts and the capability index from `self.json` | `scripts/landing.ts` |
| `/docs/**` | `docs/**/*.md`: user docs and the area plans in `docs/plans/` | `scripts/docs.ts` |
| `/reference/<unit>.html` | `self.json`, each capability's `const ROUTES` block, the unit's README | `scripts/reference.ts`, `scripts/routes.ts` |
| `/reference/self-model.html` | `self.json` | `scripts/self-model.ts` |
| `/roadmap/` | `docs/roadmap/*.md` and canonical tasks | `scripts/roadmap.ts` |
| `/roadmap/delivered/` | published-release receipts and outcome membership | `scripts/roadmap.ts` |
| `/releases/` | `docs/releases/*.md` and publication receipts | `scripts/roadmap.ts` |
| `/docs/tasks/index.html` | open canonical tasks | `scripts/roadmap.ts` |

## Build and preview

```sh
website/build                      # writes site/, checks links and the payload
website/build --serve              # builds and serves at http://127.0.0.1:8000
```

`website/build` fails when an internal link or `#anchor` has no target (`scripts/links.ts`) or
when the payload carries a private marker (`tools/check-site-payload.sh`). CI runs the same
command in `.github/workflows/pages.yml`, and deploys only from `main`.

## Writing docs

`docs/` is the one home for documentation; the site only renders it. Add a Markdown file under
`docs/<section>/`. The sidebar groups files by folder; a
folder's `index.md` comes first, the rest sort by their first heading. Links between docs use
relative `.md` paths and become site pages; any other link into the repository becomes the file
on GitHub. Markdown is rendered by Bun's built-in renderer, so the site has no npm dependency.

The area indexes in `docs/plans/` render as the Plans section and link canonical tasks.
`PLAN.md` and `research/` stay on GitHub. The [work-tracking contract](../docs/architecture/work-tracking.md)
defines canonical records. Task pages show metadata and acceptance; individual records stay out
of the documentation sidebar. The open-task index excludes completed and cancelled work.

Roadmap state derives from required tasks. Delivered history requires a publication receipt;
a source tag, accepted task or release draft alone never counts as launched. These pages use
the existing shell and static renderer, without a GitHub API call during the build.

## Layout

- `build`: the one entry point.
- `scripts/`: the generators, `style.ts` (the shell every page shares) and `markdown.ts`.
- `assets/`: files copied into the site as they are.
