# GPUI Kit components and recipes

## Resolve the documentation

Run `scripts/show_revision.py` through Python with the application's workspace `Cargo.toml`.
The helper reads the adjacent lockfile and the cached release's `.cargo_vcs_info.json`; its
GPUI Kit links use that publication commit. A missing cache leaves the versioned package source
as the fallback, not upstream `main`. Read the installed API before using any example.

To locate a component before reading broadly, run
`python3 scripts/find_component.py <workspace/Cargo.toml> <ComponentOrModule>` from the skill
root. It resolves locked, cached Cargo sources for the Rust host, then reports declaration,
consumer-use and test-candidate locations. Use `--package gpui-kit` for Kit instead of Ely and
`--limit N` for more locations. It is offline and refuses missing or locally patched sources;
its matches do not establish interaction parity.

The matching upstream skill references have distinct uses:

| Reference under `skills/gpui-kit/references/` | Read for |
| --- | --- |
| `conventions.md` | Constructor shape, state owner, callback contract and extension traits |
| `recipes.md` | A complete view with retained state, subscriptions and window overlays |
| `coding-guides.md` | A specific lifecycle, focus, layout or scrolling question |

The helper prints immutable links to these files. If a release predates a reference, inspect
its package source and examples instead. `https://gpui-kit.com/llms.txt` locates component pages;
the website describes its current release and does not establish compatibility with a local pin.

## Choose the ownership model

Inspect the constructor and event definition for the actual component family. A transient
builder does not necessarily own a value. Controlled controls request a change from their
owner; retained controls use entities and subscriptions. Store retained state and subscriptions
in the owning view, not in `render`. A dropped subscription stops delivering updates.

Keep command callbacks distinct from requested-value callbacks. Check argument types and
whether an API is a compatibility alias before adapting a neighboring control. Repeated
controls retain domain identity when sorting or filtering changes their position.

Read component extension traits as well as inherent methods. Styling, sizing, theme access and
window overlays may require explicit trait imports. A capability on one control does not imply
that another control implements it.

## Use recipes at the application boundary

Use the matching complete recipe to check initialization, retained ownership and `Root`
composition together. Inspect `gpui_kit::open_window` before adding overlay layers: when that
helper supplies the window root, application views must not mount duplicate dialog, sheet or
notification layers. Existing windows may construct their root directly; preserve that contract.

Compile the smallest affected application target. Upstream recipe commands validate the
upstream repository, not a downstream Sjel view. Cover the changed interaction through the
application's production view using [kit-testing.md](kit-testing.md).

## Keep design and architecture locally owned

`ui-craftsmanship` owns Sjel's product and visual rules. The upstream design guide can clarify
native behavior such as overlay dismissal, focus restoration and scroll containment; load only
the relevant section at the resolved revision. Its architecture, naming and public-field
preferences do not become Sjel requirements merely because a library skill calls them normative.
Keep domain behavior behind Sjel's capability and shared-core boundaries.
