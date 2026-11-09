---
name: prior-art
description: >-
  Guides minimum-sufficient coding work, prior-art discovery and critical reassessment across implementation and review. Use when deciding whether to build, checking prior art, reconsidering an in-progress approach, questioning growing complexity, or deciding whether a coding task is complete. Do not use for ordinary symbol lookup, failure diagnosis (root-cause), prose editing (human-writing), or code/UI tell scanning (unslop).
allowed-tools: Bash, Read, Grep, Glob
---

# Prior Art and the Build Ladder

Read the affected implementation, its callers and existing checks before choosing a solution.
A short diff is not evidence that the change is understood.

Use the ladder before building and at meaningful implementation or review checkpoints.
Reassess when evidence changes the approach, not after every edit.

## Scope the investigation

For a small change with a known owner, inspect that owner, relevant callers and nearby tests.
Expand the search only when this leaves an ownership, reuse or dependency question unresolved.
A repository-wide scan or delegated investigation is not a prerequisite for every edit.

For a new component, dependency or public boundary, search the relevant repository areas and
installed dependencies before proposing a build. Include the active overlay when it can own
the behavior. In Sjel, use `sjel search <concept>`, `tools/sjel-context on <path>`, owner docs and
`upstreams.toml`; inspect scoped source directly for implementation evidence.
Do not turn a limited search into a claim that no equivalent exists anywhere.

## The build ladder

Stop at the first rung that meets the actual requirements:

1. **Necessary?** Drop an unneeded feature instead of implementing it.
2. **Already present?** Reuse the existing owner rather than add a parallel implementation.
3. **Standard library?** Prefer its supported primitive.
4. **Platform-native?** Use the operating system, database or UI platform facility.
5. **Installed dependency?** Reuse an appropriate dependency before adding another.
6. **Simple composition?** Combine existing primitives without a new framework.
7. **Build?** Implement the smallest justified solution when the earlier rungs do not hold.

Prefer understandable code, not minimum character count. Preserve trust-boundary validation,
data-loss handling, accessibility and explicitly requested behavior.
Check PRD invariants and upstream verdicts before adding dependencies. Keep zero-unsafe,
proportionate-test and single-source-of-truth invariants.

For a bug, trace the shared invariant and affected callers before choosing the repair boundary.
Fix a shared defect at its owner; a caller-only fix is appropriate only when the evidence
isolates the defect there. Keep a focused check for the failure and legitimate behavior.

## Report the evidence

For an investigation, name the stopping rung, the paths or facilities checked, and the proposed
reuse, composition or build. Report searches and checks that actually ran; mark unresolved scope.
An ordinary bounded edit needs only its reuse decision and relevant check, not a ritual report.
