# web-links

A local-first launcher for web destinations the operator returns to: social accounts, communities,
work tools and service pages. This is not an article archive. Articles and page captures remain in
[`comms`](../comms/README.md) and the Sjel Clipper.

The native page is `sjel://links`. The `links` command adds, lists and removes destinations and
stages Chrome and Safari history suggestions for explicit review. The service owns the
`web_links_` tables in the shared SQLite database; the shared store backup covers them.

## Behavior

- Destinations accept HTTP and HTTPS only. URLs with embedded credentials are refused. Hostnames
  are normalized for grouping; paths remain distinct, so `reddit.com/r/rust` and
  `reddit.com/r/golang` are different destinations. Fragments are removed; path and query
  information on manually entered links is preserved.
- Site groups are deterministic, with built-in labels and tenant parsing for Reddit, X, GitHub,
  YouTube, Google and LinkedIn. Other links group by host. A group override, title and pin are
  operator-controlled and survive refreshes.
- Opening a link records a local open count. It does not fetch the destination or its page content.
- History is read only after an explicit scan action. Chrome profile databases and Safari History.db
  are opened read-only and snapshotted with SQLite's online backup API before queries. Query strings
  and fragments are removed from history candidates to avoid importing search terms or session data.
  Candidates are staged and cannot become saved links until kept; dismissal is durable.
- C2: history-derived destinations and usage observations stay local. No history content is sent to
  an inference provider. Model-suggested names and co-visit groups are not part of this delivery.

## Commands

```text
links list
links add <url> [--title TITLE] [--group GROUP] [--pin]
links remove <id>
links history scan [--days 90] [--min-visits 3]
links history candidates
links history keep|dismiss <candidate-id>
```

The API and data flow are documented in [`website/content/capabilities/web-links.md`](../../website/content/capabilities/web-links.md).
