# Worked example: octet 0.10.0

A complete pass on a real project, kept as the reference for what each lens produces and how the
verdict is built from it. The target was a native terminal coding agent in Rust, MIT-licensed,
candidate for adoption and for integration into the Sjel agent. Audit date 2026-11-07, release
v0.10.0, from a `--depth 1` clone.

The project is not malicious. The interesting result is that its verdict turned on a default, not
on a payload — and that the strengths were as decisive as the weaknesses.

## What each lens produced

**Supply chain.** 338 lock entries, 310 unique crates, 124 direct dependency entries across six
workspace crates. Zero git dependencies, zero `[patch]` sections, every `source` a crates.io
registry. Two `build.rs` files, both inert: one reads checked-in JSON and writes generated Rust,
the other packs a documented asset list into an archive with a path-traversal guard. Five
dependencies compile C (`libsqlite3-sys` via `rusqlite`, `ring`, `tree-sitter`, `zstd-sys`,
`unsafe-libyaml`). 319 `unsafe` in first-party code across the two large crates. `deny.toml`
accepts four unmaintained advisories, each with a dependency path, an exploitability argument and
a removal plan, and a test asserts an ignored advisory cannot hide a newer vulnerable version of
the same crate. Releases build `--locked` in CI, cosign-sign each artifact, and **verify their own
signatures before upload** with the certificate identity pinned to the workflow commit; every
`uses:` is SHA-pinned; the npm launcher has no `postinstall`.

**Egress.** No telemetry, analytics or crash reporting: zero hits for `sentry`, `posthog`,
`segment`, `amplitude`, `datadog` across `crates/` and `extensions/`. The destinations are the
provider API, `api.github.com` for an update check on every launch, `models.dev` for model
metadata every six hours, and per-login OAuth hosts. No hostname, username, machine ID or git
remote is collected into a request. No base-URL env override. `update.rs:167` and
`models_dev.rs:135` call `.no_proxy()` deliberately, so an egress proxy cannot see those two —
but the containment profile this machine uses renders `(deny default)`, so they are refused at the
OS layer rather than reaching the network. An audit sentence that said this needed a DNS rule was
wrong, and reading the profile is what corrected it.

**Execution.** A genuinely careful effect broker — canonical intent hashing, single-use
time-limited grants, fail-closed with no human attached, the model's own text never counted as
consent, project config that can only tighten the policy. And it ships off:

```rust
// crates/octet-coding-agent/src/cli.rs
.unwrap_or(EffectPolicy::UnsafeHost)
// crates/octet-agent/src/effect.rs
EffectPolicy::UnsafeHost => EffectRequirement::Allow,
```

Plain `octet` prompts for nothing. `--safe-mode` or a policy value selects the broker, and the
default leaves `allow_external_paths` true, so the file tools reach the whole disk. Shell calls go
to `sh -c` as one string with no blocklist. Extensions have no isolation of any kind — the
project's own `sandbox.rs` says "octet is a trusted local agent, not an OS sandbox", and its
extension docs say capability declarations are consent metadata. The Pi compatibility adapter
declares `filesystem = "unrestricted"` and `builtin_tool_overrides` including `bash`, so enabling
it hands third-party code the account.

**Data at rest.** Sessions are append-only JSONL under `~/.octet/sessions/<workspace-key>/`, mode
`0600`, holding prompt and tool-result text verbatim — a `bash: env` result puts every exported
key in that file. Credentials are plaintext JSON under `~/.octet/credentials/`, mode `0600`, with
tokens redacted in `Debug` and scrubbed from error text. No OS keychain anywhere in the tree.

## Building the verdict

Three findings decided it, ranked by whether they change the install decision:

1. The approval broker is off by default, so every tool call is auto-allowed on a machine with
   private data. This is the finding.
2. The updater executes a fetched shell script, which sits outside the Sigstore chain that
   protects the binary. Install a pinned artifact and never invoke it.
3. Credentials are files, not keychain entries, so any process running as the user can read them.

The mitigations were all real, which is why the verdict was not "reject": `--safe-mode` restores
the broker, extensions can stay disabled, the update check has an offline switch, and containment
supplies the filesystem and network boundary the program does not. That produced an `evaluate`
verdict with named conditions rather than a refusal.

## The verification step

Seven claims were re-read in the source before they were reported, and one initial summary was
wrong in the direction of alarm: the `if` branch above the default selects the approval broker
only under `--safe-mode`, not under an ordinary configuration. Reading the branch changed the
sentence from "the default may be unsafe" to the exact conditional and its flag.

A second claim did not survive the trial either: the `.no_proxy()` fetches were reported as needing
a firewall rule, when the containment profile that runs this machine denies non-loopback sockets
outright. The cause was reading the program's proxy behaviour without reading the boundary it would
run inside — which is why `references/egress.md` now asks for both.

## What this pass could not tell you

Branch protection, tag signing and 2FA on the single account that owns source, lockfile, `deny.toml`
and every workflow. Whether release assets are produced only by the pinned workflow. Runtime
behaviour of anything — nothing was built, installed or executed. Whether an `AGENTS.md` or a
project skill can talk the model into a destructive command; no adversarial runs were made.
