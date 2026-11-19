# entities

Canonical registry for people, organisations, places and the operator's own context. Google Contacts is an optional inbound adapter; Sjel owns the registry facts and their provenance.

## Why this shape: typed facts with provenance

Each entity kind has a declared field registry. Writes are checked against field type and data class, so an unknown key or value of the wrong shape is refused instead of silently becoming an untyped attribute. Values retain their source, and external identifiers let adapters update an existing entity rather than create a duplicate.

Dated location facts preserve moves and absences without overwriting earlier history. Places supplies coordinates from place text; the entity's name is not sent for geocoding.

People records are C2 data and live in the private overlay's shared store. The service has no permissive CORS, and its origin guard refuses foreign browser origins.

## Import

Google Contacts synchronization is inbound and read-only. Imported values retain `google` as their source, so later synchronization can distinguish adapter-owned values from facts entered in Sjel.

LinkedIn connections come from `Connections.csv` in LinkedIn's "Get a copy of your data" export.
`entities-server sync-linkedin <Connections.csv> [--dry-run]` reads the file locally and writes
through the same sync as Google, with `linkedin` as the source. The profile URL is the external
ID. A connection matches an existing person by exact name when only one unlinked person has that
name. LinkedIn never overwrites a value that another source holds, such as a company you entered.
The export lists only your first-degree connections, so it cannot show who your connections know.

## Run

The service is managed through `tools/service-runner.sh` and its `service.toml` manifest. The live HTTP contract is available at `GET /routes`.

## Considered and declined

- **No generic untyped attribute store.** Declared fields provide validation at the write boundary.
- **No Google Contacts write-back.** The adapter uses the read-only Contacts grant and does not send C2 records to Google.
