---
name: prior-art
description: Systematic prior-art discovery and build ladder verification before creating new code, modules, dependencies, or architectures. Investigates whether a problem needs solving, whether this codebase or monorepo already solves it, standard library availability, installed dependencies, or minimal composition before authoring new implementations. Trigger: prior art, check prior art, build ladder, should I build this, before writing, reuse over rewrite. Do not use for general codebase browsing without a proposal to build.
allowed-tools: Bash, Read, Grep, Glob
---

# Prior Art & The Build Ladder

Before authoring a new component, crate, dependency, or feature, check whether an equivalent already exists.
Never default to building from scratch without running the build ladder and executing a real prior-art search.

## The 7-Rung Build Ladder

Stop at the first rung that holds:

1. **Needs to exist at all?**
   - Question the premise: Is the feature solving a real problem or premature optimization/avoidance?
   - Can the workflow or requirement be satisfied without adding any code?

2. **This codebase already has it?**
   - Search Sjel monorepo: `libs/`, `capabilities/`, `tools/`, and the active overlay.
   - Run the prior-art search script:
     ```bash
     Packs/coding/skills/prior-art/scripts/search-prior-art "<concept_or_symbol>"
     ```
   - Query the knowledge graph:
     ```bash
     graphify query "<concept_or_symbol>"
     ```
   - Check `upstreams.toml` for prior verdicts, evaluations, or rejected prior art.

3. **Standard library?**
   - Rust: `std::sync`, `std::collections`, `std::time`, `std::path`, `std::fs`.
   - Node / Web / TS: Web APIs (`fetch`, `URL`, `crypto`, `Intl`), standard ECMAScript.
   - Do not pull an external dependency for something standard library provides cleanly.

4. **Platform-native?**
   - Operating system tools: macOS / Linux command-line utilities, kernel facilities, keychain, system services.

5. **An already-installed dependency?**
   - Check existing dependencies in `Cargo.lock` or `bun.lock`:
     ```bash
     cargo tree -i "<crate_name>"
     ```
   - Can an existing workspace dependency (e.g. `serde_json`, `chrono`, `reqwest`, `rusqlite`, `tokio`) solve it directly?
   - Do not add a new crate without checking `upstreams.toml` and PRD invariants.

6. **One line / simple composition?**
   - Compose existing primitives, helper functions, or pipelines before creating an abstract framework.

7. **Minimum code that works.**
   - Only when rungs 1 through 6 fail, implement the minimum viable solution.
   - Adhere strictly to zero-unsafe invariants, proportionate tests, and single source of truth.

## Running Prior Art Investigation

To run a prior-art investigation:

```bash
# 1. Search symbols, paths, crates, and upstreams:
Packs/coding/skills/prior-art/scripts/search-prior-art "<query>"

# 2. Check architecture knowledge graph:
graphify query "<query>"

# 3. Check for related decisions in ISA or PRD:
grep -in -C 2 "<query>" ISA.md ARCHITECTURE.md
```

Report findings ranked by the ladder rung:
1. **Decision**: Which rung stops the ladder (Reuse / Compose / Build)?
2. **Found prior art**: Concrete file paths, symbols, or crate references discovered.
3. **Action**: Adopt directly, compose with existing utilities, or proceed with minimal new code.
