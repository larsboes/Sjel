# Product boundaries

[`PLAN.md`](../../PLAN.md) owns priority. The
[build rule](../product.md#what-sjel-builds-ingests-and-links-to) owns the decision criterion.
This plan owns the remaining reductions and their acceptance checks.

## Capability audit

Source inspection on 2026-11-15 covers the 45 public `service.toml` declarations: 25 capability
owners in `crates/` and 20 supporting declarations in `services/`. The evidence is each owner's
README, manifest and the implementations named below. This covers declared ownership, not live
workflow acceptance. Private overlay capabilities need a separate local audit; their names,
configuration and findings stay in the overlay.

An Ingest verdict permits source bytes, provenance, classification and mappings used by a join.
It does not justify rebuilding the source application's unrelated features. A Build verdict
applies to the stated join or policy boundary, not every feature in its crate.

| Capability and evidence | Verdict | Retained boundary |
|---|---|---|
| [agent](../../crates/agent/README.md) | Build | One governed run, approvals and tool calls across capabilities |
| [assistant](../../crates/assistant/README.md) | Build | Class-aware role selection and context from multiple capabilities |
| [calendar](../../crates/calendar/README.md) | Build + ingest | Source events and the availability, rhythms and trip-fixpoint join |
| [comms](../../crates/comms/README.md) | Ingest | Source observations, provenance, classification and reviewed evidence for other owners |
| [devices](../../crates/devices/README.md) | Build | Pairing, signed-request admission, revocation and replay prevention |
| [documents](../../crates/documents/README.md) | Ingest | Originals, extraction, classification and source-linked evidence |
| [entities](../../crates/entities/README.md) | Build + ingest | Canonical identities, classified facts and source identifiers across tools |
| [finance](../../crates/finance/README.md) | Build + ingest | Journal/import projections joined to subscriptions, trips and places |
| [import](../../crates/import/README.md) | Build | Routing a mixed source tree to its owning capabilities with retry receipts |
| [interior](../../crates/interior/README.md) | Build + ingest | Measured geometry joined to owned items and clearance constraints |
| [inventory](../../crates/inventory/README.md) | Build + ingest | Item history joined to reviewed mail, rooms, prices and packing |
| [media](../../crates/media/README.md) | Ingest | Original-byte custody, source identity and extracted attributes; no independent photo-manager roadmap |
| [operator-profile](../../crates/operator-profile/README.md) | Build | Private preferences and governed projections into multiple harnesses |
| [places](../../crates/places/README.md) | Build + ingest | Canonical place identities joining transactions, people and trips; external geocoding inputs |
| [projects](../../crates/projects/README.md) | Build + ingest | Project/task/source identity and reviewed mail/inventory joins; prose authority moves to Markdown under [storage](storage.md#one-vault-for-prose) |
| [punctuality](../../crates/punctuality/README.md) | Ingest | Source stop observations and reliability aggregates consumed by Transit |
| [scouting](../../crates/scouting/README.md) | Build + ingest | Source opportunities joined to qualification rules, interests, dates and journeys |
| [sjel-status](../../crates/sjel-status/README.md) | Build | Authenticated discovery, lifecycle and health across declared owners |
| [software](../../crates/software/README.md) | Build + ingest | Platform installation facts joined to storage ownership and reviewed removal decisions |
| [soundscape](../../crates/soundscape/README.md) | Link out; reduction open | Standalone playback state is not a join; native synthesis and its controller need retirement |
| [transit](../../crates/transit/README.md) | Build + ingest | Provider journeys, prices and tickets joined to reliability and trip constraints |
| [traveler](../../crates/traveler/README.md) | Build | Explicit constraints and ranking preferences shared by destination and journey decisions |
| [trips](../../crates/trips/README.md) | Build | Itineraries joining calendar, transport, places, people, costs and gear |
| [vault](../../crates/vault/README.md) | Ingest | Markdown properties, classes and links; files retain text authority |
| [web-links](../../crates/web-links/README.md) | Link out; reduction open | The launcher/history recommender has no implemented cross-capability join |

### Supporting declarations

Supporting jobs inherit their owner's boundary. Their existence does not authorize another
domain store or an independent application.

| Declarations under `services/` | Retained boundary |
|---|---|
| `entities-sync`, `entities-google-sync`, `people-registry` | Entities intake and the policy projection of known-person identities |
| `feed-sweep`, `finance-prices`, `punctuality-ingest`, `wish-prices` | Scheduled intake for Comms, Finance, Punctuality and Inventory |
| `sparpreis-watch` | Reprice references in existing trip plans through Transit |
| `archive`, `knowledge-base`, `packs`, `store` | Preservation and backup contracts for their existing owners |
| `container-refresh`, `host-patch`, `host-watch`, `storage-reclaim` | Declared installation, health and storage lifecycle operations |
| `foundation-models`, `macmon`, `sqld`, `vaultwarden` | Managed upstream runtimes; consume their interfaces without rebuilding their applications |

## Open reductions

1. **Retire standalone Soundscape.** `crates/soundscape/src/main.rs` stores playback intent and
   broadcasts it; the native synthesizer lives in `apps/gpui/core/src/soundscape/`. The README
   explicitly leaves calendar/presence-driven selection unimplemented, so that proposed join
   cannot justify the current standalone feature. Apple's
   [Background Sounds](https://support.apple.com/en-mo/guide/mac-help/mchl3061cdc6/mac) supplies
   local focus/rest audio, but does not establish parity with the custom generative music.
   Replacement selection is pending; code removal is deferred until that product choice is
   resolved. Preserve an export of existing settings before retirement. Done when the Mac and
   phone open the selected destination, playback ownership is clear, native synthesis and the
   conductor are no longer built or started, and preserved state remains recoverable.
2. **Reduce Links to destinations.** `crates/web-links/src/server.rs` exposes saved links,
   open tracking, browser-history scans and candidate decisions. None feeds another capability.
   Browser bookmarks already provide saved destinations
   ([Safari](https://support.apple.com/en-gb/guide/safari/ibrw1039/mac)). The browser/destination
   choice is pending; retirement is deferred until saved-link export and destination acceptance
   can be verified. Done when saved destinations are preserved and usable in the selected tool,
   Sjel retains only the configured outbound destination, and history collection, usage tracking,
   candidate storage and the standalone service leave the active build and lifecycle. Existing
   database rows are not deleted as part of code removal.
3. **Verify reductions inside retained capabilities.** Build + ingest rows are boundary verdicts,
   not feature-complete audits. Inspect feature routes and clients against the retained boundary,
   beginning with Media's organization features, Documents' reader and Finance's independent
   analysis. Keep source/evidence inspection and derived views; reject unrelated editor,
   photo-manager or accounting-engine expansion. Done when each removed path has an equivalent
   source or outbound path, dependent joins still pass their checks, and obsolete code is removed.

The audit is not complete while these reductions or the private-overlay review remain open.
No capability, service, personal record or original file has been removed by this audit.
