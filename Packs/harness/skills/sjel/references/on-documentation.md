# Documentation and skill maintenance

Keep each fact with its owner:

- Root README: human-facing Sjel architecture and durable repository doctrine.
- Capability or Pack README: purpose, verdict, tradeoffs, provenance, and local decisions.
- Manifest or schema: machine-readable configuration contract.
- Source comment: reasoning needed exactly where an implementation is changed.
- Generated architecture: current manifest-derived structure; change its inputs or generator.
- Skill reference: reusable agent procedure or slow-changing decision test.
- Runtime tool: ports, inventories, health, issue state, graph size, and other changing facts.

- Owning `ISA.md`: open claims and decisions. Claims are short and falsifiable, never essays.
- `docs/capabilities/<name>.md`: a capability's technical manual (interface contracts, data
  flows, Mermaid diagrams, guarantees). The capability README stays a lean entry point with the
  why and a link to the manual.

Do not create status documents, duplicate live counts, or a detached decision-log hierarchy.
Git log owns history: no dates, PR numbers, or migration stories in code comments.

Sjel planning lives in the repository. The Obsidian vault keeps only what cannot be public:
private reasoning and vault law. When you move planning out of the vault, replace the vault copy
with a link to the repository file. Agent memory notes are not a planning surface either: write a
durable rule into the skill or `ISA.md` that owns it.

## Code comments

- Prefer expressive, strongly typed code (newtypes, type-state, explicit enums) over prose. If
  code needs a paragraph to explain what it does, rename and restructure it instead.
- Document every public boundary with rustdoc `///` or JSDoc, including `# Errors`.
- Internal comments: at most one or two lines, only for a non-obvious hardware, kernel, or
  concurrency invariant.
- Prefer a short Mermaid or ASCII diagram in a module README for data flow or state.
- When you edit a module, trim its narrative comments to these rules. Do not run a separate sweep.
- Dashboard routes in `dashboard/src/routes/` are thin orchestrators. Put boards, panels, cards,
  and sheets in `dashboard/src/lib/<domain>/<Component>.svelte`.
For a new skill, keep `SKILL.md` as a direct router and put detailed branches one level under
`references/`. Keep scripts deterministic and validate metadata plus every referenced path.

When changing a Pack, update its manifest and README, then verify both harness-neutral source and
materialized harness adapters. Preserve upstream attribution for adapted material.
