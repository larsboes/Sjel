# Finance: investment ideas and research

Area plan for the investment side of the `finance` capability: ideas about instruments held or
not held, price triggers, research routed from `comms`, and model proposals. Ledger, import and
price work stays in [`data.md`](data.md#5-domain-backlog) until it moves here.

## Decisions

- **Ideas live in finance storage only.** Decided 2026-11-05. No vault note, no Obsidian
  frontmatter. The Mac app and the API are the only editors.
- **A model may propose entries and exits.** Decided 2026-11-05. This overturns the rule in
  `crates/finance/src/decision.rs` (module header) that no rule mints `sell`. Constraints that
  keep it honest:
  - A model proposal carries `rung = "model"`, the model id, the cited feed items and the price
    at proposal time. It is advice. Nothing in Sjel places an order.
  - Every proposal needs a verdict, as rung-1 proposals do today.
  - Every proposal is scored after 30 and 90 days against holding the instrument unchanged.
    The score shows next to each new proposal, so the track record is visible where the advice
    is.
- **Research reaches an idea by match plus a model digest.** Decided 2026-11-05. Matching is
  deterministic (ticker, name, keywords on the idea). The digest is a short weekly summary per
  idea that cites the matched items.
- **Mauerstrassenwetten comes in by RSS now and OAuth later.** Decided 2026-11-05. The public
  feed `https://www.reddit.com/r/mauerstrassenwetten/top/.rss?t=week` answered HTTP 200 without
  login on 2026-11-05; the JSON API answered 403. RSS carries title, link and post text, no
  comments and no score.

- **Any cloud model provider may see finance data.** Decided 2026-11-05: positions, ideas and
  feed text may go to the provider the `assistant` capability is configured with.
- **Subscription notes are Sjel-managed.** Decided 2026-11-05. The module that reads them from
  `projects` is `crates/finance/src/subscription_notes.rs`, and its routes are
  `/api/import/subscription-notes` and `/api/import/subscription-notes/scan`. Finance declares no
  Obsidian import.

## Spec

### 1. Ideas ledger

Built: `crates/finance/src/ideas.rs` and the append-only `finance_idea_events` table. An idea
has `label`, `ticker`, an optional held `instrument`, `stance` (`hold`, `watch`, `enter`,
`exit`), `thesis`, `keywords`, `triggers` and `status`. Routes: `GET`/`POST /api/ideas`,
`GET`/`PUT /api/ideas/{id}` (`PUT` appends a full snapshot), `POST /api/ideas/{id}/notes`. An open
idea's ticker without a held instrument joins the nightly Yahoo fetch under the ticker as its
key. Checked live 2026-11-05: the three example ideas exist, an edit kept the earlier thesis
in `history`, and `S68.SI` got a price, with SGD added to the ECB fetch.

Open: the ideas in the app (section 6).

### 2. Triggers

Built: `check_triggers` in `crates/finance/src/ideas.rs` runs at the end of
`finance-cli prices fetch` (the `finance-prices` unit). A level is compared in its own currency,
or through EUR at the newest ECB rate when the quote is in another one. A crossing creates a
`projects` task due that day in `ideas_project` (finance.json, default `Inbox`), and the hit is
recorded in `finance_idea_trigger_events` only after the task exists, so a failed delivery
retries on the next run. A level fires again only after the price is back on the near side.
The phone announces the task at 09:00 through the task rule in `apps/gpui/core/src/notify.rs`.
Checked live 2026-11-05: a test level produced one task, and a second run produced none.

### 3. Research routing

`comms` feed items match an open idea on ticker, label or keywords. The matcher generalizes
`match_feed_items` in `crates/finance/src/decision.rs`. Matches show under the idea with source,
date and link.

Done when: a planted feed item naming a keyword shows on its idea and on no other.

### 4. Mauerstrassenwetten source

A `comms` source kind for a subreddit RSS feed that reuses the Atom parsing in
`crates/comms/src/sources.rs`. It keeps posts whose title or flair marks a DD. Later: a Reddit
app with OAuth (the token in the keychain, provisioned by the operator) adds comments and scores
for posts that match an idea, through the adapter in `crates/comms/src/media/reddit.rs`.

Done when: one weekly fetch lands DD posts in the feed and a non-DD post is dropped.

### 5. Model digest and proposals

A weekly digest per open idea cites its matched items. Proposals
follow the constraints in Decisions.

### 6. App

A Finance view "Ideas": the list with stance, price against the nearest trigger, and new
research count; the detail with thesis history, triggers, matched research, digest and
proposals. Built with synthetic fixtures; the manual acceptance test is the operator's.

## Order

1, 2, 6 (ideas usable without research), then 3 and 4, then 5 once its provider is decided.
