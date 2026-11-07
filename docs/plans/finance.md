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
key. Create, replace, note, and review-verdict routes publish request schemas derived from the
same serde request types they parse; the review action itself has no request body. Checked live
2026-11-05: the three example ideas exist, an edit kept the earlier thesis in `history`, and
`S68.SI` got a price, with SGD added to the ECB fetch.

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

Built: `GET /api/ideas` attaches up to 8 comms feed items of the last 90 days whose title or
summary names the idea's label, ticker symbol (the part before the exchange suffix) or a
keyword, as whole tokens (`match_research` in `crates/finance/src/ideas.rs`, the tokenizer of
`crates/finance/src/decision.rs`). The app lists them under Research. Checked 2026-11-05: the
90-day feed held 1,037 items (651 arXiv, 325 GitHub) and none named the three example ideas, so
research depends on a finance source (section 4).

### 4. Mauerstrassenwetten source

Built: the comms adapter `reddit-rss` (`crates/comms/src/sources.rs`) reads a subreddit's top
posts from its public Atom feed (`query` = subreddit, `since` = day, week or month) and delivers
each entry's title, author and HTML body with its URL, because Reddit answers 403 to the
per-post JSON and 429 to the per-post feed. An optional `title_tokens` keeps only posts whose
title holds one of the words. The overlay declares `reddit-mauerstrassenwetten` (week, 25 posts,
`c0`) in `comms.json`. Measured 2026-11-05: one scan stored 19 posts, and the Rheinmetall idea
listed one of them as research.

Limits: the feed carries no flair, so "DD only" is a title filter; the subreddit search feed
answered with 0 to 1 entries for `flair:DD` and `DD`. No comments and no scores without OAuth.

Open: a Reddit app with OAuth (the operator registers it and stores the token in the keychain)
for comments and scores on posts that match an idea.

### 5. Model digest and proposals

Built: `crates/finance/src/review.rs` and the table `finance_idea_reviews`. Once a week per
open idea, at the end of `finance-cli prices fetch`, and on demand through
`POST /api/ideas/{id}/review` ("Review now" in the app), the `assistant` capability's cloud
role (`ideas_model_role` in finance.json, default `cloud_assistant_google`) receives the idea's
name, ticker, stance, thesis, levels, notes, latest price and the summary or transcript of its
matched feed items, numbered as sources. It never receives a quantity, a value or an account.
The answer is a digest, an `enter`, `exit` or `hold` proposal, a rationale and the cited
sources; a citation of a source the prompt did not give is dropped. A failed call stores
nothing and the next run asks again. `POST /api/ideas/{id}/reviews/{review_id}/verdict` records
`accepted` or `rejected` once. Each night the price move 30 and 90 days after a proposal is
recorded; `enter` is right when the price rose, `exit` when it fell, `hold` is not judged. The
30-day track record goes into the next prompt and shows in the app.

First live review 2026-11-06 with `gemini-3.8-flash` after the operator allowed remote models
while On the go and raised the role's `max_requests_per_day` from 1 to 10 (`inference.json`):
`hold` on the Rheinmetall idea, with the digest naming its one source as thin. A model call gets
120 s. Known limit: a Reddit link post's feed entry carries only its link, so such a source
gives the model a title and little else.

### 6. App

Built: the Finance view "Ideas" (key `5`, `apps/gpui/ui/src/finance/ideas.rs`). A list with
stance, price and the gap to the nearest level in the same currency; a form for a new idea; and
for the selected idea its thesis, stance, price levels (a typed `1200,50` is read as `1200.50`),
notes, close or reopen, and earlier theses. Open: the operator's manual acceptance test in the
running app.

## Order

1, 2, 6 (ideas usable without research), then 3 and 4, then 5 once its provider is decided.
