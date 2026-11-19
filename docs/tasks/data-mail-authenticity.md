---
id: "data-mail-authenticity"
title: "Accept measured mail authenticity flags"
state: "backlog"
area: "data"
kind: "investigation"
priority: 2
depends_on: ["data-mail-labels"]
spec: "docs/architecture/data-intake.md"
---

# Accept measured mail authenticity flags

## Outcome

Accept measured mail authenticity flags through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Measure the model rung on independently labelled phishing/legitimate mail and add an audit action to GPUI Mail; verdicts remain flags only.
- Ask whether every swept body may be read before enabling body audits; check forwards against their quoted sender.
- Evaluate fixed suffix-list limits, first-label look-alikes and a known-sender set contaminated by prior phishing; verify the authenticated redirect exemption against false positives.
