# Skill detector trial

The 2026-11-08 trial keeps Sjel intake and does not add another routine scanner.
Both detectors passed four benign cases. Sjel held all six malicious cases; Cisco's
default local analyzers held one. These ten synthetic smoke cases exercise current
admission requirements. They are not held-out evidence of general detection accuracy.

## Reproduce

Requirements: Git, Docker, OSV Scanner, Bun and a built Sjel CLI. Clone the public
[upstream](https://github.com/cisco-ai-defense/skill-scanner) and check out the
`source_revision` in `cases.json`, then run from the repository root:

```sh
bun --no-env-file tools/skill-scanner-eval.ts <reviewed-clone> <sjel-cli> <output.json>
```

The driver archives the pinned source, applies `upstream-lock.patch`, checks its digest,
and requires a fresh OSV pass before building. The original lock had advisories in
LiteLLM, msgpack, pyasn1 and virtualenv; the patch updates those dependency groups.
Build steps download public dependencies. Base images are pinned by digest; the built
image identity is recorded in `results.json`. An advisory-free dependency report does
not establish that dependency code is benign.

Each scan runs as an unprivileged container with no network, a read-only root, dropped
capabilities and resource limits. Only generated synthetic fixtures are mounted.
Remote analyzers, semantic review and custom rule packs are disabled. No real skills,
host credentials or personal content are supplied. Scratch source and fixtures are
removed; Docker retains its normal image/build cache.

## Source review boundary

The pinned first-party source was examined for execution, egress, stored content and
installation behavior. Relevant locations within that revision:

- `core/analyzer_factory.py`: remote analyzers are opt-in; record completed analyzers.
- `core/cel/go_runtime.py`: the native helper starts even with CEL suppression disabled;
  source builds compile it with Go. Turning a feature off is not an execution boundary.
- `core/loader.py`: direct leaf loading skips symlinks. Recursive host discovery was avoided.
- `core/models.py`: reports can contain paths and source excerpts. Fixtures are public synthetic text.
- `config/config.py`: environment configuration can enable integrations; the container supplies
  its own environment. The CLI does not automatically load a host dotenv file.

This is a bounded source review and execution trial, not an audit of every transitive
dependency or a general safety certification. The local analyzers reported static,
bytecode, pipeline and correlation coverage. Each case took approximately 4–7 seconds,
including container startup. A missing-license informational finding appeared in every case.

Reconsider a local semantic configuration only after dependency preflight and a separate
corpus establish sufficient coverage and acceptable review cost. The verdict lives in
[`upstreams.toml`](../../../upstreams.toml).
