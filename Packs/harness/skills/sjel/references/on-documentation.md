# Documentation and skill maintenance

Keep each fact with its owner:

- Root README: human-facing Sjel architecture and durable repository doctrine.
- Capability or Pack README: purpose, verdict, tradeoffs, provenance, and local decisions.
- Manifest or schema: machine-readable configuration contract.
- Source comment: reasoning needed exactly where an implementation is changed.
- Generated architecture: current manifest-derived structure; change its inputs or generator.
- Skill reference: reusable agent procedure or slow-changing decision test.
- Runtime tool: ports, inventories, health, issue state, graph size, and other changing facts.

- `PLAN.md` and `docs/plans/<area>.md`: priority, open work with a done-when check, decisions not
  yet built.
- `docs/capabilities/<name>.md`: a capability's technical manual (interface contracts, data
  flows, Mermaid diagrams, guarantees). The capability README stays a lean entry point with the
  why and a link to the manual.

Do not create status documents, duplicate live counts, or a detached decision-log hierarchy.
Git log owns history: no dates, PR numbers, measurements, decisions, or migration stories in code
comments or config comments. Put them in the commit message.

Sjel planning lives in the repository. The Obsidian vault keeps only what cannot be public:
private reasoning and vault law. When you move planning out of the vault, replace the vault copy
with a link to the repository file. Agent memory notes are not a planning surface either: write a
durable rule into the skill or the area plan that owns it.

## Code comments

- Prefer expressive, strongly typed code (newtypes, type-state, explicit enums) over prose. If
  code needs a paragraph to explain what it does, rename and restructure it instead.
- Document every public boundary with rustdoc `///` or JSDoc, including `# Errors`.
- Internal comments: at most one or two lines, only for a non-obvious hardware, kernel, or
  concurrency invariant.
- Prefer a short Mermaid or ASCII diagram in a module README for data flow or state.
- When you edit a module, trim its narrative comments to these rules. An edit does not add
  comment lines unless it adds a public boundary. The one planned sweep is `docs/plans/restructure.md`.
- Write Sjel, never Axon. Rename an old mention in the lines you touch.

## Config comments

- A TOML, YAML or JSON-with-comments key gets at most one comment line: what the value means or
  its unit. Rationale goes in the owner README's `## Why this shape`, history in the commit.

## Plans

- A shipped work package shrinks to one line that names its commit. The commit holds the detail.
- Plans live in `PLAN.md` (order, state, decisions) and one file per area in `docs/plans/`. A
  README or manifest describes what is built and why. It holds no open-work list: move that to the
  area plan or an issue.
- Dashboard routes in `dashboard/src/routes/` are thin orchestrators. Put boards, panels, cards,
  and sheets in `dashboard/src/lib/<domain>/<Component>.svelte`.

## Register

Write every tracked document like a well-run open-source project, for a reader who was not in
the conversation.

- Name the project, the user, a capability or the agent. "Operator" names the role of whoever
  runs a deployment, never a person who decided something.
- No "you", "I" or "we", and no dialogue between an assistant and its owner.
- A decision carries its date, not who made it or how it was asked: "Decided 2026-10-25", never
  "(operator, 2026-10-25)".
- State facts with their source (a path, a command, a commit). Mark an estimate or an unverified
  claim as such.

## Skills and Packs

For a new skill, keep `SKILL.md` as a direct router and put detailed branches one level under
`references/`. Keep scripts deterministic and validate metadata plus every referenced path.

When changing a Pack, update its manifest and README, then verify both harness-neutral source and
materialized harness adapters. Preserve upstream attribution for adapted material.
