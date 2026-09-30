# Documentation and skill maintenance

Keep each fact with its owner:

- Root README: human-facing Sjel architecture and durable repository doctrine.
- Capability or Pack README: purpose, verdict, tradeoffs, provenance, and local decisions.
- Manifest or schema: machine-readable configuration contract.
- Source comment: reasoning needed exactly where an implementation is changed.
- Generated architecture: current manifest-derived structure; change its inputs or generator.
- Skill reference: reusable agent procedure or slow-changing decision test.
- Runtime tool: ports, inventories, health, issue state, graph size, and other changing facts.

Do not create status documents, duplicate live counts, or a detached decision-log hierarchy.
For a new skill, keep `SKILL.md` as a direct router and put detailed branches one level under
`references/`. Keep scripts deterministic and validate metadata plus every referenced path.

When changing a Pack, update its manifest and README, then verify both harness-neutral source and
materialized harness adapters. Preserve upstream attribution for adapted material.
