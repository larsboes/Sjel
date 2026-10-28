---
name: obsidian
description: Operates the user's Obsidian vault as a durable knowledge surface. Resolves the active vault, gates writes on its contract, creates and maintains knowledge notes, Bases, links, and clippings intended for knowledge, and recalls what earlier sessions said. Use for work inside the vault or questions about what it contains. Do not use for Sjel-owned records, generic Markdown outside the vault, prose style, or authoring skills.
allowed-tools: Read, Write, Edit, Bash, Glob, Grep
---

# Obsidian

The active vault holds durable knowledge. Sjel owns operational records and unsorted source
material. Keep one canonical home for each record and link to its owner instead of copying it.

## Resolve and gate

Always resolve the configured vault and read its contract before changing vault contents:

```bash
scripts/vault root
scripts/vault contract
```

The contract is at `Knowledge/Meta/Vault/Vault-Contract.md`. If the gate fails, stop and report it;
do not invent a replacement structure. Follow the contract's note types, metadata, placement and
link rules.

## Read before writing

Search for an existing owner and read it before answering a question or adding material. Type new
notes at birth with the required properties. Update the owner in place, link related concepts, and
avoid duplicate claims. Use the existing `Knowledge/` domains, MOCs and `Knowledge.base` views.
Create no capability data, operational inbox, or second copy of a Sjel record in this vault.

## References

| Task | Read |
|---|---|
| Wikilinks, embeds, callouts, properties | `references/markdown.md` |
| `.base` views, filters, formulas | `references/bases.md` |
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
