---
name: ha-dashgen
description: Generates Home Assistant Lovelace dashboards from an overlay-owned house model and deploys them, backing up first. Public mechanism, private inventory - this Pack owns every card, layout and deploy decision and knows no entity ID; the active overlay owns model.ts. Use when building, checking, or deploying generated dashboards, or when adding a room, entity or view to the generated set. Do not use for hand-editing a dashboard in the UI, backing up or restoring a single dashboard (use ha-dashboard), querying or controlling entities (use ha-cli), or filling automation templates (use homectl).
allowed-tools: Bash, Read, Edit, Write
---

# ha-dashgen

Dashboards as code. A TypeScript model of the house goes in, three deployed dashboards come
out, and `check` refuses to let a dashboard reference an entity that does not exist or quietly
lose a control the previous one had.

## Commands

```sh
scripts/ha-dashgen build     # model -> JSON in <overlay>/config/home-assistant/dashboards/
scripts/ha-dashgen check     # build + validate against the LIVE instance (exit 1 on a problem)
scripts/ha-dashgen ensure    # create any dashboard that does not exist yet
scripts/ha-dashgen deploy    # build, back up each dashboard, then push
```

Needs `SJEL_HOME_ROOT` (or `SJEL_OVERLAY_ROOT`) and, for anything touching the instance, an
unlocked `BW_SESSION` — it shells out to `ha-cli` and `ha-dashboard`, which own the credentials.

## What check actually checks

1. **Every entity reference exists** on the running instance, including IDs inside Jinja.
2. **No control appears on two peer views.** The `start` view is exempt: it is a shortcut
   surface, so a favourite tile pointing at a light that also lives in Räume is intended.
3. **Nothing was silently dropped.** It diffs against `dashboards/baseline-<date>/` and fails
   on any entity the old dashboard had that the new one lacks, unless `model.ts` lists it in
   `deliberatelyDropped` with a reason. A stale entry — declared dropped but back on a view —
   also fails.

Run `check` before `deploy`. `deploy` always backs up to `dashboards/pre-deploy/` first;
`lovelace/config/save` overwrites in one shot with no undo.

## Gotchas

Each of these was found the expensive way against **HA 2026.6.3** with a probe dashboard, and
the first four blank the **entire view**, not just the offending card, with nothing in the
browser console:

- **`condition: template` in a `conditional` card** renders "Konfigurationsfehler".
- **A template in section `visibility`** hides the section even when it is `{{ 1 == 1 }}`.
  Put the logic in a `binary_sensor` and use `condition: state`, which works.
- **`custom:mushroom-template-card`**: a static `secondary` renders, a templated one comes out
  empty, and inside a `conditional` the card takes the view with it. `mushroom-chips-card` is
  fine. Prefer a native `tile` whose entity is a klartext sensor.
- **A `weather` entity as a view badge**, and a `binary_sensor` badge. View-level badges are
  not used at all for this reason; heading badges are fine.
- **`picture-glance` with `camera_image`** and **`picture-entity` on a camera** both render
  nothing here, at `camera_view` `live` or `auto`. Cameras are plain tiles that open the
  live stream on tap.
- **`column_span` greater than the view's `max_columns`** collapses the grid to a blank page.
- **A dashboard `url_path` must contain a hyphen.** HA rejects `zuhause`, accepts `mein-zuhause`.
- **`notify:` platforms are not reloadable.** A new notify group needs a full restart, not
  `automation.reload`.

## Boundary

Never put a real entity ID, room name, device or household fact in this Pack. Those belong in
`<overlay>/capabilities/home-assistant/dashboards/model.ts`, which is the only file that should
need editing to change what the dashboards show.
