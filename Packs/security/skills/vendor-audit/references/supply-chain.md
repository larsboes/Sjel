# Lens 1: supply chain

What gets built, from what, and who can alter the result.

## Dependency graph

```bash
# Rust
grep -c '^\[\[package\]\]' Cargo.lock            # total crates
grep -n 'source = ' Cargo.lock | grep -v 'crates.io'   # non-registry sources
grep -n '^\[patch' -r . --include=Cargo.toml       # patched deps

# Node
ls package-lock.json bun.lock pnpm-lock.yaml 2>/dev/null
grep -n '"scripts"' -A6 package.json           # postinstall is the vector

# Python
grep -n 'git+\|http' requirements*.txt pyproject.toml 2>/dev/null
```

What each result means:

- **A `git` dependency** is mutable: no crates.io/npm provenance, no registry yank, and the
  publisher can rewrite the ref. It is not automatically bad — it is unauditable by version.
- **A `path` dependency with no `source`** ships in the repo. That is *safer*, and it means
  verifying it is reading the tree rather than trusting a publisher. Say which it is.
- **`[patch]`** redirects a crate somewhere else. Read every one.
- **A typosquat check is worth doing and usually comes up empty.** A name that looks like an
  impersonation is often a known maintainer's crate (`dtolnay`, `BurntSushi`). Verify against the
  publisher, then drop it from the report rather than leaving it as unease.

## Build scripts

```bash
find . -name build.rs -not -path './target/*'
grep -n 'Command::new\|TcpStream\|reqwest\|curl\|std::process' <each build.rs>
```

A `build.rs` runs at compile time with the developer's full account. The finding is network access
or process spawn; reading checked-in files and writing generated Rust is ordinary and says so in
its own header.

## Where memory safety stops

```bash
grep -rn '\-sys = \|cc = \|bindgen' --include=Cargo.toml .
grep -rn 'cmd!(\|cc::Build' . | head
```

Any `-sys` crate, `cc`, or `bindgen` means C is compiled into the binary. A project claiming Rust
safety is claiming it for the Rust it wrote. Count the C-compiling dependencies and name them.

`unsafe` in first-party code is worth a count, not a verdict — but a crate carrying
`#![forbid(unsafe_code)]` is evidence someone enforced it rather than intended it.

## Advisory posture

Read the project's own vulnerability config (`deny.toml`, `.cargo/audit.toml`, `.osv-scanner.toml`,
an allowlist in CI). The interesting part is not the exceptions but whether each carries a
dependency path, an exploitability argument and a removal plan. Exceptions with reasons are a
maintained project. Exceptions without reasons are a suppressed scanner.

Check for the enforcement too: a test that asserts an ignored advisory cannot hide a newer
vulnerable version of the same crate is a stronger signal than the exception list itself.

## Release and install integrity

```bash
ls .github/workflows/
grep -n 'cosign\|sigstore\|minisign\|gpg\|--provenance\|--locked' .github/workflows/*.yml
grep -n 'uses:' .github/workflows/*.yml | grep -v '@[0-9a-f]\{40\}'
```

Rank what is found:

1. Artifact built by CI `--locked` from a tagged commit, signed, and **verified before upload** —
   the strongest form, because provenance cannot be claimed after the fact.
2. Signed only, verified downstream.
3. A checksum manifest downloaded from the same release as the binary. That catches corruption,
   not compromise.
4. Hand-uploaded artifacts. Nothing binds them to the source at all.

Every `uses:` not pinned to a 40-hex SHA is a workflow that can change under the project. Count
them; a project with zero unpinned actions has someone watching.

The install path is part of this lens: read `install.sh` / `install.ps1` / the npm launcher. What
does it verify before running a downloaded binary, and does it fail closed? A launch script that
pipes a remote script to a shell in its own *documentation* is telling the reader to skip the
check the release pipeline performs.

## Licences

`deny.toml`'s `[licenses]` allowlist, or the equivalent. Look for copyleft that binds more than
the author intended. MPL-2.0 is file-scoped and usually fine; AGPL is not, for most uses.
