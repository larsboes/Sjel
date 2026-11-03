# website

The public site at `larsboes.github.io/Sjel/`, built from tracked files by `website/build`.
Nothing on it runs a capability, and nothing is fetched while a page renders.

| Path on the site | Source | Built by |
|---|---|---|
| `/` | hand-written copy, counts and the capability index from `self.json` | `scripts/landing.ts` |
| `/docs/**` | `website/content/**/*.md` | `scripts/docs.ts` |
| `/reference/<unit>.html` | `self.json`, each capability's `const ROUTES` block, the unit's README | `scripts/reference.ts`, `scripts/routes.ts` |
| `/reference/self-model.html` | `self.json` | `scripts/self-model.ts` |

## Build and preview

```sh
website/build                      # writes site/, then checks links and the payload
cd site && python3 -m http.server  # preview at http://127.0.0.1:8000
```

`website/build` fails when an internal link or `#anchor` has no target (`scripts/links.ts`) or
when the payload carries a private marker (`tools/check-site-payload.sh`). CI runs the same
command in `.github/workflows/pages.yml`, and deploys only from `main`.

## Writing docs

Add a Markdown file under `website/content/<section>/`. The sidebar groups files by folder; a
folder's `index.md` comes first, the rest sort by their first heading. Links between docs use
relative `.md` paths and become site pages; any other link into the repository becomes the file
on GitHub. Markdown is rendered by Bun's built-in renderer, so the site has no npm dependency.

Contributor material (plans, open work) stays in `docs/`, not here.

## Layout

- `build`: the one entry point.
- `scripts/`: the generators, `style.ts` (the shell every page shares) and `markdown.ts`.
- `assets/`: files copied into the site as they are.
- `content/`: the user docs.
