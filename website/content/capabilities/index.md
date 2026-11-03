# Capabilities overview

A capability owns a domain contract and declares its runtime in `service.toml`. Public units live
under `capabilities/`; private units can live in the active overlay. The
[generated architecture](../../ARCHITECTURE.md#capabilities) lists public declarations. Use the
host registry for the enabled set and live status.

## Capability types

| Manifest `kind` | Runtime |
| --- | --- |
| `process` | A host process. `autostart` declares a persistent service; `schedule` declares a periodic job; neither means on demand. |
| `container` | A container managed by the selected Docker or Podman runtime. This is the default kind when omitted. |
| `data` | State with a backup contract and no process to start. |

Start with the [manifest guide](manifest.md) when adding a unit, then read
[contracts and health](contracts.md). The manifest's `requires` array declares runtime
prerequisites; it does not permit one capability to mutate another's tables.

## Reference guides

| Task | Manual |
| --- | --- |
| Connect domain identifiers and resolve people | [Typed IDs and `/api/links`](links.md) |
| Understand intake and privacy redaction | [Feed and communications](feed-and-comms.md) |
| Change availability, rhythms, or calendar sync | [Calendar](calendar.md) |
| Build a CV or compare a LinkedIn snapshot | [CV](cv.md) |
| Inspect media ingest, integrity, and organization previews | [Media](media.md) |
| Follow journey planning and trip intent | [Travel and routing](travel.md) |

The owning README explains purpose and design choices. These manuals describe interface and data
contracts; [PLAN.md](../../PLAN.md) and [docs/plans/](../plans/) hold open work and decisions.
