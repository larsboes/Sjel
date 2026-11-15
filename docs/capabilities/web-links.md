# Web Links

The `web-links` capability owns saved browser destinations and reviewed history-import candidates.
The native app labels the surface **Links** and routes it as `sjel://links`. The existing `libs/links`
crate and `docs/capabilities/links.md` continue to own typed record references; the two domains are
not interchangeable.

## Data model

`web_links_items` is one row per normalized destination URL. A record stores its title, normalized
host, optional site-specific tenant, optional user-defined group override, pin state and local open
observations. The effective group is the override when present, otherwise a deterministic site label
or normalized host. The destination itself is the identity, not the site: multiple communities or
accounts at one host remain distinct rows.

`web_links_candidates` stores URLs discovered by an explicit browser-history scan. Candidate state
is `pending`, `kept` or `dismissed`. Keeping a candidate and creating its saved row happen in one
SQLite write transaction. Repeated scans upsert counts only for pending candidates and never reopen
a dismissed decision. URL uniqueness prevents duplicate saved rows.

## URL rules

- Accept only absolute `http` and `https` URLs with a host. Reject user information, non-web schemes,
  blank input and values longer than 2,048 bytes.
- Lowercase the host and remove fragments. Preserve query parameters for manually entered links;
  history candidates drop query parameters and fragments because a browsing-history query can carry
  search terms or session data.
- Normalize known host aliases into visible site labels. Recognize Reddit communities/users, X
  accounts and GitHub repositories/notifications as tenant labels. Never use those labels as record
  identity.

## Browser history

The scan is explicitly initiated, bounded to 1–365 days and 1–100 minimum visits, and limited to
Chrome profiles named `Default` or `Profile *` plus Safari's `History.db`. Each database is opened
read-only and copied to a temporary SQLite snapshot before querying. Queries are bounded to 5,000
rows per profile; failed or unavailable sources are reported to the service log while available
sources continue. A scan stages suggestions only. The operator must keep or dismiss each candidate
before it enters the launcher.

The history reader does not upload browser data, inspect page content, or call a model. Browser
history, saved destinations and visit counts are C2 and remain local. No model is part of this
release.

## HTTP contract

The service is local-only and rejects foreign browser origins.

| Method | Path | Purpose |
|---|---|---|
| GET | `/web-links/api/links` | List saved destinations |
| POST | `/web-links/api/links` | Add a destination; duplicate normalized URLs return the existing row |
| PATCH | `/web-links/api/links/{id}` | Edit title, group override or pin state |
| DELETE | `/web-links/api/links/{id}` | Remove a destination |
| POST | `/web-links/api/links/{id}/open` | Record an open |
| POST | `/web-links/api/history/scan` | Scan and stage history candidates |
| GET | `/web-links/api/history/candidates` | List pending candidates |
| POST | `/web-links/api/history/candidates/{id}/decision` | Keep or dismiss a candidate |

The app opens saved URLs through GPUI's external URL handler after the explicit open action is
recorded. Browser history is not queried while rendering or at service startup.

## Boundaries

- Capability-owned tables use the `web_links_` prefix in the shared store; `capabilities/store`
  covers the database in its normal backup.
- The service reads only standard macOS browser-history locations. It writes no browser files.
- The initial surface is Mac-first. Phone replication, share-sheet intake, co-visit grouping and
  shadow-first local model names remain future work.
