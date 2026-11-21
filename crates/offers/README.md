# offers

Offers read from mail: discount code, discount and expiry, each with the line it was read from.
Each offer lives in `<overlay>/data/offers/<thread-id>/` as `offer.toml` beside the
`original.eml`, so the mail can be opened again until the offer expires. A mail with no offer is
recorded as `no-offer`.

## Use

```bash
comms mail export --out <dir> --from rituals     # originals of matching mail
crates/offers/offers import <dir>                # read and store them
crates/offers/offers list [--all]                # valid offers by last day
crates/offers/offers show <thread-id>            # one offer with its evidence lines
crates/offers/offers set <thread-id> used        # or active, dismissed
crates/offers/offers reread                      # extract stored originals again
crates/offers/offers released > released.json    # mail that may go
comms mail cleanup --released released.json      # rules with "gate": "released"
```

## Why this shape

- **Offers owns what an offer is.** Comms exports originals only; this crate reads them. The same
  split as `comms mail attachments` and `documents inbox`.
- **Deterministic extraction with evidence.** A percentage counts only with a minus sign or a
  discount word on its line; an expiry only with an expiry word, and only between the mail date
  and 120 days after it. Every value keeps its line, so a wrong reading is visible.
- **Release, not deletion.** `released` lists used, dismissed, `no-offer` and expired offers. An
  offer with no stated expiry stays valid for 30 days after the mail. Comms trashes only behind
  the owner's Allow.
- **Files, not a database.** One folder per mail, the same contract shape as `posts`.

## Not yet

No server route, no app page, and no link from an offer to a wish or a gift idea.
