# Which UI stack carries Sjel to every platform?

First written 2026-11-07. Sjel targets macOS and iOS now, with Linux, Windows and Android next and
the web as an option ([`gpui.md`](../docs/plans/gpui.md)). Since 2026-10-22 its one app is built on
GPUI ([`PLAN.md`](../PLAN.md#decisions)). This entry records what the GPUI ecosystem offered on
2026-11-07 and why the phone gets a trial on a different renderer. The plan that acts on it is
[`gpui.md` §11](../docs/plans/gpui.md#11-ui-stack).

## The claim

GPUI is the right renderer for the desktop and the wrong one for the phone. GPUI draws every pixel
itself. On a desktop that gives speed and full control. On a phone it has to rebuild text
selection, input methods, accessibility and system materials that UIKit and Android views already
provide. Native Shell (NSH) renders through each platform's own views and can be embedded in a GPUI
app, so one screen definition can serve the phone natively and the Mac inside GPUI.

The first half is supported by the sources below. The second half is unproven for Sjel. NSH was 24
days old when this was written, and nothing in Sjel has run on it.

## The projects

| Project | What it is | State on 2026-11-07 | Source |
|---|---|---|---|
| GPUI (Zed) | GPU-drawn Rust UI, developed inside the Zed repository | Platform crates split out, including `gpui_ios` | [zed/crates](https://github.com/zed-industries/zed/tree/main/crates) |
| `gpui-pre` | Longbridge's crates.io snapshots of Zed's GPUI | 0.3.11, the snapshot GPUI Kit 0.9.0 pins | [crates.io](https://crates.io/crates/gpui-pre) |
| GPUI Kit | Component kit on `gpui-pre`, Sjel's dependency | 0.9.0 on 2026-11-02, with an optional `gpui-fast` feature | [installation.md](https://github.com/longbridge/gpui-kit/blob/main/website/docs/installation.md), "Optional GPUI Fast backend" |
| GPUI Fast | Fork of GPUI with retained rendering and scroll layers | 0.1.13 on crates.io, Apache-2.0 | [README](https://github.com/longbridge/gpui-fast#readme) |
| `gpui-mobile` | GPUI platform for iOS and Android that a Swift or Kotlin host embeds | Last merge 2026-11-03 (#45), on `gpui-pre` 0.3.10 | [pulls](https://github.com/longbridge/gpui-mobile/pulls) |
| `gpui-pre-ios` | Zed's own `gpui_ios`, published by Longbridge | 0.3.11 since 2026-10-19 | [crates.io](https://crates.io/crates/gpui-pre-ios) |
| NSH | Rust UI framework that renders with each platform's views | v0.4.0 on 2026-11-04, created 2026-10-14, Apache-2.0 | [README](https://github.com/longbridge/nsh#readme) |

One team at Longbridge leads GPUI Kit, `gpui-pre`, GPUI Fast, the `gpui-mobile` compatibility fork
and NSH ([contributors](https://github.com/longbridge/nsh/graphs/contributors)).

## What was checked

1. **GPUI Fast is a feature switch for Sjel's desktop.** GPUI Kit 0.9.0 selects it with
   `features = ["gpui-fast"]`, with no `[patch]` and no alias (installation.md above). Its README
   reports 73 to 88 percent less main-thread CPU per frame than `gpui-pre`, measured by Longbridge
   on Linux. Sjel's own baseline is the table in [`gpui.md` §9](../docs/plans/gpui.md#9-frame-cost).
   Retained rendering redraws only what a tracked entity, global or list state changed. Other render
   state needs `cx.notify()` ([retained-mode.md](https://github.com/longbridge/gpui-fast/blob/main/docs/retained-mode.md)).
2. **GPUI Kit 0.9.0 needs `gpui-pre` 0.3.11, and `gpui-mobile` was on 0.3.10.** A fork at 0.3.11
   compiled for the iOS simulator with every feature after one change: `WgpuContext::new` takes the
   surface configuration in 0.3.11. Sjel's core, ui and desktop tests then passed, except one test
   that already failed before the bump (`gpui.md` §6.1).
3. **GPUI Fast has no published iOS path.** `gpui-fast-platform` 0.1.13 depends on
   `gpui-pre-ios =0.3.11` on iOS, and that crate depends on `gpui-pre`, not on `gpui-fast`
   ([dependencies](https://crates.io/crates/gpui-fast-platform/0.1.13/dependencies)). The fork's
   own `crates/gpui_ios` is not published. Inferred from the manifests, not built.
4. **Zed's `gpui_ios` is not a drop-in for Sjel's phone.** It owns `UIApplication` and the scene
   delegate ([application.rs](https://github.com/zed-industries/zed/blob/main/crates/gpui_ios/src/ios/application.rs)).
   Sjel's phone app starts in Swift and embeds GPUI as one view
   ([`App.swift`](../apps/gpui/ios/host/App.swift)), with RoomPlan, audio, pairing and
   notifications in that host. Only `gpui-mobile` supports that shape.
5. **NSH embeds into an existing iOS app.** A Swift app links `NativeShell.xcframework` and places
   an `NshAppView`, a `UIView`, in its own hierarchy ([ios.md](https://nsh.dev/docs/ios.md),
   "Embedding in an iOS app"). Host views such as maps can appear inside NSH screens
   ([native-components.md](https://nsh.dev/docs/native-components.md)).
6. **NSH embeds into a GPUI app on the same GPUI Fast.** NSH's GPUI backend pins `gpui-fast`
   0.1.13 ([`crates/gpui/Cargo.toml`](https://github.com/longbridge/nsh/blob/main/crates/gpui/Cargo.toml)),
   the version GPUI Kit 0.9.0 selects. An `AppView` is a GPUI view ([gpui.md](https://nsh.dev/docs/gpui.md)).
7. **NSH states the phone argument itself.** Its comparison page says NSH builds on each platform's
   views "so text input, selection, input methods and system interaction are the platform's own"
   ([comparison.md](https://nsh.dev/docs/comparison.md), "GPUI").
8. **Sjel's data layer is GPUI-free.** `apps/gpui/core` (32,941 lines) has no GPUI dependency
   ([`core/Cargo.toml`](../apps/gpui/core/Cargo.toml)). The pages in `apps/gpui/ui/src` (70,942
   lines) use GPUI's state model in 1,057 places, counted with
   `grep -c "cx.notify\|Entity<\|impl Render" -r apps/gpui/ui/src`.

## What was measured

On 2026-11-07, Sjel's Mac app was built twice in release mode, once on `gpui-pre` 0.3.11 and once
on GPUI Fast 0.1.13, and `apps/gpui/bench-scroll` scrolled five pages on each build. The table is
in [`gpui.md` §9](../docs/plans/gpui.md#9-frame-cost).

- On systems, inventory and feed, the median frame costs 28 to 41 percent less CPU on GPUI Fast.
- On documents the median is about equal, and the slow tail is shorter.
- On packs the median rises by about a third, and p95 halves.
- Frames over the 8.3 ms budget fall from 58 to at most 12 across the five pages.

This is well below the 73 to 88 percent in the GPUI Fast README. Sjel's pages already draw few
rows per frame after the virtual-list work in `gpui.md` §9, so less work was left to retain. That
explanation is an estimate, not measured. Tests and clippy give the same result on both builds.

## What the sources do not show

- How NSH behaves on Sjel's screens and on a real iPhone. Text selection, VoiceOver and scrolling
  are claims of the NSH documentation, not measurements.
- Whether an NSH view and the GPUI view run well side by side in one iOS app, in memory and
  startup time.
- Whether NSH's API holds still. It went from v0.1.1 to v0.4.0 in 17 days
  ([releases](https://github.com/longbridge/nsh/releases)).
- Whether an embedded NSH build needs Longbridge's Cargo fork. The NSH CLI is built with
  [huacnlee/cargo](https://github.com/huacnlee/cargo). Plain `cargo run` and `cargo test` drive a
  Rust app ([rust.md](https://nsh.dev/docs/rust.md)).
- GPUI Fast's effect on Linux and Windows builds of Sjel, on the phone, and on pages outside the
  five measured.
- How much of `ui/src` ports to NSH by translation. NSH reuses GPUI's style vocabulary
  (`view().px_4()`) but runs a component once and updates through signals, so state wiring is a
  rewrite. An estimate, not measured.
- Claims from a model-generated survey of the ecosystem, such as relative scores and benchmark
  percentages in [nsh-perf-bench](https://github.com/longbridge/nsh-perf-bench), were not
  reproduced.
