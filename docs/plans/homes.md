# Homes

Status: deferred. Household deployment repositories own their configuration, documentation,
network investigation and operating procedures. A new Sjel capability is not required to hold
those documents. Reconciled 2026-11-08; the earlier site-notes capability proposal is retired.

## Boundary

Sjel owns personal computing requirements and reusable clients only when a concrete Sjel
consumer needs them. A household host that does not run Sjel keeps its standalone deployment
repository. Sjel reads an explicit interface, such as a backup receipt, rather than becoming a
second owner of its notes or configuration.

Instance facts remain private. Network observations involving other people's devices are
`c2`; credentials remain outside documents and prompts.

## Archived clients

Home Assistant, router and DNS clients require a consumer and an access-contract review before
promotion. Their private archive is reference material, not an active dependency. Review and
retirement belong to the repository holding that archive; no Sjel port is a prerequisite for
its cleanup. Automation materializers and dashboard generators remain retired.

## Open

- Verify boot, backup and power-loss recovery for the personal always-on host when selected.
- Add a generic client only when an observed workflow requires it; prefer existing interfaces.
- Printing integrations remain deferred until a printer and a concrete workflow require one.
