---
name: obsidian
description: Operates the user's Obsidian vault as a durable knowledge surface. Resolves the active vault, gates writes on its contract, creates and maintains knowledge notes, Bases, links, and clippings intended for knowledge, and recalls what earlier sessions said. Use for work inside the vault or questions about what it contains. Do not use for Sjel-owned records, generic Markdown outside the vault, prose style, or authoring skills.
allowed-tools: Read, Write, Edit, Bash, Glob, Grep
---

# Obsidian

The active vault holds durable knowledge. Sjel owns operational records and unsorted source
material. Keep one canonical home for each record and link to its owner instead of copying it.

## Resolve and gate

Always resolve the configured vault and read its contract, if one is configured, before changing
vault contents. The contract path is set per machine with `contract_path` in the overlay's
`config/knowledge.toml`; the contract is optional. When one is configured, it owns placement, the
no-doubling law, naming, and frontmatter typing. When none is configured, follow the vault's
existing conventions: read neighbouring notes, their folders and frontmatter before writing.

In Pi, use the governed `obsidian_*` tools. Call `obsidian_contract` before the first mutation in
a session. It loads the configured contract from the vault through Obsidian and gates writes
against its current digest, or reports that no contract is configured for this machine. If a
contract is configured but missing, stop and say so — do not improvise structure or substitute an
overlay-local contract.

Outside Pi, resolve the vault and contract through the bundled helper:

```bash
scripts/vault root
scripts/vault contract
```

`scripts/vault contract` prints the configured contract's path, or reports on stderr that none is
configured. If it fails, stop and report it; do not invent a replacement structure. Follow the
contract's note types, metadata, placement and link rules.

In Pi, use the narrowest governed operation that fits: `obsidian_read` supports whole-note, heading,
block, and frontmatter targets; `obsidian_outline`, `obsidian_search`, `obsidian_backlinks`,
`obsidian_properties`, and `obsidian_tags` cover low-token discovery. Read `.base` files with
`obsidian_base_read`; validate drafts with `obsidian_note_validate` or `obsidian_base_validate`
before writing. Create with `obsidian_create`, extend with `obsidian_append`, and make structural
edits with `obsidian_patch`. Reserve `obsidian_write` for deliberate whole-note replacement.
Moves and deletes require confirmation; never work around a refusal by switching to shell or
direct filesystem access.

## Read before writing

Search for an existing owner and read it before answering a question or adding material. Type new
notes at birth with the required properties. Update the owner in place, link related concepts, and
avoid duplicate claims. Use the existing `Knowledge/` domains, MOCs and `Knowledge.base` views.
Create no capability data, operational inbox, or second copy of a Sjel record in this vault.

## References

| Task | Read |
|---|---|
| Wikilinks, embeds, callouts, properties | `references/markdown.md` |
| `.base` views, filters, formulas | `references/bases.md`, then `obsidian_base_validate` |
| `.canvas` files | `references/canvas.md` |
| Obsidian CLI | `references/cli.md`, then `obsidian help` |
| Ingest, synthesize, or lint knowledge | `references/wiki.md` |

## Knowledge capture

Capture a web source into Sjel first when it is an unsorted source or operational record. Add a
vault note only when the result is durable knowledge or a source note allowed by the vault
contract. Preserve provenance and link the knowledge note to its Sjel-owned source where useful.
Do not treat the vault as a general clipping inbox.

## Verification

After edits, check frontmatter, links, and the contract's lint requirements. Bases and canvases
require visual verification in Obsidian; report when that check remains outstanding. Never place
secrets in notes.
