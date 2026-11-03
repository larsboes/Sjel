# wish-prices

The daily price refresh for wish comparisons. Once a day it runs `inventory prices refresh`,
which reads the product page in each candidate wish's `link` and records the lowest offer as a
dated price observation. A wish group holds the candidates for one purchase. The comparison
itself is `GET /inventory/api/wish-comparison`, `inventory compare`, and the wishlist tab of the
GPUI app.

What it reads, and what it does not:

- Only geizhals product pages: `https`, a geizhals host, no query string, a path ending in
  `.html` (`intake::refresh_source`). geizhals' `robots.txt` disallows its search pages
  (`/?fs=`) for every agent, so a search URL is never read.
- It identifies itself as Sjel, never as a browser. A site that refuses that client fails the
  run (exit 1), and nothing works around the refusal.
- One request per candidate, three seconds apart. A second run on the same day records nothing.

Every other price, such as a refurbished or used offer, is a manual entry:

```
inventory price <wish-id> 1.401,02 --source kleinanzeigen --config "32 GB/256 GB" --on 2026-11-03
```

The first observation at or below a wish's `cost_max_cents` is marked as the alert. The run
prints `ALERT <wish-id>` for it, and the comparison view shows it.

Run it by hand:

```
inventory prices refresh
```
