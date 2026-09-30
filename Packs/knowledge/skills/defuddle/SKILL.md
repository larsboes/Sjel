---
name: defuddle
description: Reads a web page as clean markdown with navigation, ads and clutter removed. In pi, use the built-in `fetch_content` tool — it needs nothing installed and also handles PDFs, GitHub repos and YouTube. Otherwise use the `defuddle` CLI, which must be installed first (see below). Use when a URL's content must be read as markdown. Do not use for URLs already ending in .md.
---

# defuddle

Two ways to read a page as clean markdown. Prefer the first.

## In pi: use `fetch_content`

pi has a native `fetch_content` tool, from the vendored pi-web-access extension. Use it:
one call rather than a shell command, nothing to install, and it also handles PDFs, GitHub
repositories and video transcripts, which the CLI does not.

**Do not shell out to the CLI while that tool is available.** It is the fallback, and reaching
for it in pi costs a subprocess to do a worse job.

## Otherwise: the `defuddle` CLI

For a harness with no fetch tool, this skill needs the CLI on `PATH`. It is installed
globally through its own release tooling, not vendored into Sjel:

```bash
npm install -g defuddle        # → $(npm prefix -g)/bin/defuddle
defuddle --version             # expect 0.19.3 or newer
```

If `defuddle` is not found, that install is the fix — run it, rather than switching to a
fetch tool you do not have. Nothing else in Sjel depends on it.

### Usage

Always pass `--md` for markdown output:

```bash
defuddle parse <url> --md                  # readable content to stdout
defuddle parse <url> --md -o content.md    # save it
defuddle parse <url> -p title              # one metadata field
```

| Flag | Format |
|------|--------|
| `--md` | Markdown — the default choice |
| `--json` | JSON carrying both HTML and markdown |
| (none) | HTML |
| `-p <name>` | one metadata property: `title`, `description`, `domain`, … |

## Provenance

[kepano/defuddle](https://github.com/kepano/defuddle), MIT, by Steph Ango — see the
`[defuddle]` row in `upstreams.toml`. It sits alongside the same author's
`kepano/obsidian-skills`, which `Packs/knowledge/LICENSE` covers; these are two different
repositories with two separate notices, and only the second is vendored.

## One thing not to "fix" later

The `defuddle` library also ships inside the vendored `pi-web-access`, which imports it as a
dependency for its own extraction path. That is a different role, not a duplicate: an extension
needs a module it can `import`, and this skill needs a command on `PATH`. Collapsing either into
the other makes it fragile — `node_modules` is wiped and reinstalled, a global binary is not.
Both are pinned to the same version on purpose. If they drift, one path renders pages with a
different extractor than the other, so move them together or not at all.
