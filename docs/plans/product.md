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
| [soundscape](../../crates/soundscape/README.md) | Build; join open | Calendar- and presence-driven selection of preset and parameters; the native synthesizer stays in the app |
| [transit](../../crates/transit/README.md) | Build + ingest | Provider journeys, prices and tickets joined to reliability and trip constraints |
| [traveler](../../crates/traveler/README.md) | Build | Explicit constraints and ranking preferences shared by destination and journey decisions |
| [trips](../../crates/trips/README.md) | Build | Itineraries joining calendar, transport, places, people, costs and gear |
| [vault](../../crates/vault/README.md) | Ingest | Markdown properties, classes and links; files retain text authority |
| [web-links](../../crates/web-links/README.md) | Link out; reduction open | Saved links move to one Markdown file each under `Atlas/Links/` in the prose folder |

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

## Open boundary work

1. **Build Soundscape's join.** Soundscape stays and earns its Build verdict through the join its
   README leaves open: preset and parameters follow calendar, presence and activity. The
   conductor (`crates/soundscape/src/main.rs`) and the native synthesizer
   (`apps/gpui/core/src/soundscape/`) stay. This is expansion work under
   [`PLAN.md` Priority 3](../../PLAN.md#priority-3-agent-features-and-expansion). Before code, a
   spec names the input signals, their mapping to presets and parameters, and how a manual
   choice overrides a derived one. Done when a calendar entry or a presence change sets the
   preset and parameters with no manual step, a manual choice stays in force until its session
   ends, and the selection reads only through the owning capabilities' existing contracts.
2. **Reduce Links to files in folders.** `crates/web-links/src/server.rs` exposes saved links,
   open tracking, browser-history scans and candidate decisions. None feeds another capability.
   Each saved link becomes one Markdown file under `Atlas/Links/` in the prose folder
   ([storage](storage.md#one-vault-for-prose)), with `url:` in its frontmatter. The folders are
   the owner's organization; a link that belongs to a person or project names it in
   frontmatter. `links export-files` writes the first layout, one folder per site group.
   Done when every saved link has its file, the Finder opens a link file's `url:` directly,
   and history collection, usage tracking, candidate storage, the Links page and the standalone
   service leave the active build and lifecycle. Existing database rows are not deleted as part
   of code removal.
3. **Verify reductions inside retained capabilities.** Build + ingest rows are boundary verdicts,
   not feature-complete audits. Inspect feature routes and clients against the retained boundary,
   beginning with Media's organization features, Documents' reader and Finance's independent
   analysis. Keep source/evidence inspection and derived views; reject unrelated editor,
   photo-manager or accounting-engine expansion. Done when each removed path has an equivalent
   source or outbound path, dependent joins still pass their checks, and obsolete code is removed.

   Source inspection of the Documents reader retains the existing evidence boundary:
   `apps/gpui/ui/src/documents/viewer.rs` renders stored originals, searches extracted pages and
   opens originals through the platform renderer. Filing comparison and duplicate review use
   that reader; replacing it requires equivalent evidence inspection, not an unrelated editor.
   Reader controls may adopt shared components while PDF rendering and search remain intact.

   Media organization stays inside Media's original-byte custody boundary.
   `crates/media/src/organize.rs` moves whole collections by same-volume rename from an approved
   structure and records a reversal journal. It is frozen: no new organization features and no
   Media UI polish.
   `apps/gpui/ui/src/media/content.rs` separately reviews extracted tag/collection candidates;
   accepting metadata does not authorize a file move. This inspection ran no live file moves,
   reader acceptance checks or dependent-join tests.

The audit is not complete while items 2 and 3 or the private-overlay review remain open.
No capability, service, personal record or original file has been removed by this audit.
