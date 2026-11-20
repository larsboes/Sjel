---
id: "gpui-nsh-measurement"
title: "Measure NSH Mail before desktop convergence"
state: "cancelled"
area: "gpui"
kind: "investigation"
priority: 1
depends_on: ["gpui-nsh-facade"]
spec: "docs/architecture/native-ui-boundaries.md"
reason: "Native UI convergence moves phone screens to shared GPUI."
---

# Measure NSH Mail before desktop convergence

## Outcome

Measure NSH Mail before desktop convergence through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Extend the owned bench launcher for nsh-trial/SJEL_NSH_TRIAL and compare both Mail views on identical data.
- Require p95 within the proposed 10% threshold, no additional frames over 8.3 ms, and keyboard/VoiceOver reachability for every row action; identify the threshold as an estimate and inspect actual Mac content.
- Choose the Mac renderer from evidence: pass converges one area at a time and deletes its parallel GPUI page; fail retains two renderers over shared core. Record results in the existing UI-stack research.
- Keep Mac GPUI pages until that choice; do not infer that removing phone GPUI authorizes deleting the Mac UI.
