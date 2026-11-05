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

## Open decisions

1. **Notification channel for a trigger hit.** Unverified which surface the Mac and the phone
   both receive today. Candidates: a comms capture, the phone's local notification host
   (`ios/host`).

## Spec

### 1. Ideas ledger

An idea is one thought about one instrument, held or not.

| Field | Meaning |
|---|---|
| `id` | Stable id. |
| `instrument` | An `instruments` id when held, else empty. |
| `ticker`, `label` | The Yahoo symbol and a display name. A ticker makes the nightly fetch price it. |
| `stance` | `hold`, `watch`, `enter`, `exit`. |
| `thesis` | Free text: why, and what would prove it wrong. |
| `keywords` | Extra match terms for research (for example `SGX`, `Singapore Exchange`). |
| `triggers` | Zero or more `{direction: above\|below, price, currency}`. |
| `status` | `open` or `closed`, with a closing note. |

Changes are append-only events, like `finance_decision_events`, so the history of a thesis
survives edits. Routes: `GET /api/ideas`, `POST /api/ideas`, `PATCH /api/ideas/{id}`,
`POST /api/ideas/{id}/notes`.

Examples the first version must hold: exit levels for the Rheinmetall position, a re-entry
level for Rocket Lab, and a `watch` idea for a non-cyclical exchange operator (`S68.SI`,
Singapore Exchange, in SGD).

Done when: the three examples are created through the API and in the app, an edit keeps the old
thesis in the event history, and a ticker on a not-held idea gets a nightly price.

### 2. Triggers

After each nightly price fetch, every open trigger is compared with the newest price, converted
with the newest ECB rate when currencies differ (the rule in `crates/finance/src/money.rs`,
`unit_price_in_eur`). A crossing records one hit event and notifies once. It does not fire again
until the price crosses back.

Done when: a synthetic test crosses, holds and re-crosses a level and gets exactly two hits.

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
