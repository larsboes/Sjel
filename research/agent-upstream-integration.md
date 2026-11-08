# Agent upstream integration

External tools have one admission and deployment path. Keep upstream implementations upstream;
retain the useful decisions here. `upstreams.toml` owns each source's verdict and rationale.

## Anatomy

Source: [wheresryan22/anatomy](https://github.com/wheresryan22/anatomy), MIT.
The consumer is technical teaching figures and presentation illustrations.

Install with `tools/skills store wheresryan22/anatomy --skill anatomy`. The store preserves
the screened bytes, records their digest and source identity, and exposes the skill as a Pack.
Select that Pack through `profiles.toml`; deploy through `tools/harnesses`. Re-storing the
source is the update path. Do not vendor a second copy in `Packs/`. Revision metadata is retained
when the upstream CLI supplies it; the screened content digest always pins the installed bytes.

Sample PDFs and images are part of the upstream package. Intake keeps binary assets in the
snapshot and digest without sending them to the text reviewer. Text remains bounded at
2 MiB, each file at 32 MiB, and the complete snapshot at 128 MiB. The gate still refuses
symlinks, special files, unreadable content and unreviewed text. Renaming text as an asset
does not change its screening limit.

## Intelligent UI

Source: [razorback16/cc-intelligent-ui](https://github.com/razorback16/cc-intelligent-ui), MIT.
The inspected checkout was `c8314a6`. This is a Claude Code rendering plugin, not a portable
agent skill. Its hooks and terminal graphics protocol support are part of that implementation.

Retain these ideas for the existing terminal and rich-content readers:

- Keep Markdown as the durable answer and derive rendered output from it.
- Derive colours from the surrounding terminal or artifact theme.
- Keep copying and ordinary text fallback usable when graphics are unavailable.
- Reuse Typst, CeTZ and fletcher when a typeset terminal figure has a concrete consumer.

No Sjel renderer or hook is adopted. A future plugin installation must screen its hooks and
verify compatibility with the active harness and terminal. Do not turn a plugin into a
duplicated skill or replace the existing Mermaid figure workflow solely to share a renderer.

## Octet

Source: [skaft-software/octet](https://github.com/skaft-software/octet), MIT.
The v0.10.0 runtime trial is complete. The audit method and evidence are in
[`vendor-audit`](../Packs/security/skills/vendor-audit/references/worked-example.md) and
`upstreams.toml`. Octet is an idea source, not a supported Sjel harness.

The reusable ideas are its canonical effect-intent hashing, single-use grants and explicit
extension contracts. Compare them with the existing `crates/sjel-agent` policy and extension
interfaces before porting code. An extension needs an identified consumer and an acceptance
test; a second agent runtime is not required to reuse a design.

Running Octet remains declined while the audited default effect policy allows calls without
approval, the updater executes a fetched shell script outside the release verification chain,
and credentials use plaintext files. The contained trial also found configuration and catalog
verification blockers. Re-evaluate a changed upstream against those concrete findings before
changing the verdict. Removing the disposable trial does not remove the audit evidence.
