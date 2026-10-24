# Interactive figures in Obsidian and Sjel notes

Read this when a figure belongs inside an Obsidian or Sjel note, whichever skill or hand drew
it (the upstream Anatomy skill, `upstreams.toml` `[anatomy]`, is one source). Interactive
figures are the normal output in both.

## Obsidian: embed the interactive figure in the note

This plugin's `interactive` code block mounts local HTML, styles and scripts directly inside the note;
it follows Obsidian's theme and reloads when the asset changes. Produce a self-contained HTML
asset with inline CSS and JavaScript, then embed it in the owning note:

````markdown
```interactive
file: Knowledge/Computing/Systems/Platforms/Assets/dial-indicator.html
height: 560
```
````

The path above illustrates the installed block contract; resolve the actual destination from
the active vault's current rules. The optional `title:` names the figure; omit `height:` for
automatic height. Keep selectors and DOM lookups scoped to the figure's root so multiple embeds
do not interfere. Register a one-shot `interactive-dispose` listener on that root to clean up timers, animation
frames and listeners before a rerender or note teardown. Read `native-renderer.ts` before adding
lifecycle code. Inspect `theme.ts` and use the
plugin's theme variables and `obsidian-theme-change` event instead of fixing one theme.

Use the plugin's `html-embed` iframe mode when isolation is needed. Check its script, resize
and asset-loading contract first; native and iframe mode are different renderers.
A static SVG or PNG is an optional export for print, sharing or a client without the plugin.
It does not replace the interactive note embed by default.

## Sjel: embed a registered figure in the note

Project notes use the existing figure registry in `dashboard/src/lib/figures/index.ts`.
The washing-machine figures demonstrate the contract:

```markdown
![[figure:miele-w961-filter]]
![[figure:miele-w961-machine|Whole-machine schematic]]
```

`dashboard/src/lib/projects/database.ts` parses these into figure segments, and
`ProjectsBoard.svelte` mounts the registered Svelte component inside the note body. Inspect the
Miele wrappers and their controller modules before adding another figure. Reuse the registry;
do not replace the embed with a separate page or introduce React. Dispose animation and event
handlers on component teardown, support several instances, and retain keyboard and reduced-motion
behavior. Keep the model separate from presentation; the capability owns domain values.

## Placement and verification

Resolve the active vault and read its current contract through the Obsidian skill before writing
notes or assets. Update the owning note; use its permitted asset location and link syntax.
For Sjel, use the Sjel skill to resolve the owning component, documentation or capability.
Keep reusable source in Sjel and private values and machine paths in the overlay or vault.
Local integration never authorizes publishing a private figure to the community gallery.

Run the figure's geometry and visual checks. Verify the embed in its actual note at the note's
width, including pointer and keyboard interaction, theme changes, reduced motion, navigation away
and back, and multiple copies. Check that local asset paths resolve. Check a static export only
when one is requested. State which host rendering was not verified instead of treating a browser
preview as proof that an Obsidian or Sjel embed works.
