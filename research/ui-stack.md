# Which UI stack carries Sjel to every platform?

First written 2026-11-07, extended 2026-11-09. Sjel targets macOS and iOS now, with Linux, Windows
and Android next and the web as an option ([`gpui.md`](../docs/plans/gpui.md)). Since 2026-10-22
its one app is built on GPUI ([`PLAN.md`](../PLAN.md#decisions)). The 2026-11-07 sections record
what the GPUI ecosystem offered and why the phone got a trial on a different renderer. The
2026-11-09 section records the long-term stack question, the alternatives that were rejected, a
red-team pass and a council. The plan that acts on it is
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

## The phone trial

On 2026-11-07 the phone's Documents reader was built in NSH and run on a paired iPhone 14 Pro
([`gpui.md` §11](../docs/plans/gpui.md#11-ui-stack)). The list filled from the Mac, a long press
on the text opened the system menu, and VoiceOver read the list and the reader. NSH and
`gpui-mobile` linked into one library with stock Cargo. Two NSH defects were found on the way:
the published `nsh-platform-ios` 0.4.0 includes a file outside its package, and the `v0.4.0` GPUI
backend does not compile against `gpui-fast` 0.1.13. NSH's main branch had fixed the second the
same day.

## What the sources do not show

- How NSH behaves on screens other than Documents, and its scrolling and memory cost on a phone.
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

## 2026-11-09: the long-term stack, and how generated UI runs

### The question

Sjel is expected to grow into a large personal system: many documents, mail, finance records and
dashboards, with fast typing and scrolling, agent features, and later Linux, Windows and Android.
The question was whether GPUI and NSH carry that load, which alternative would carry it better,
and how agent-generated ("dynamic") UI should run. On the same day,
[`b59879d0`](https://github.com/larsboes/Sjel/commit/b59879d0) moved every remaining phone screen
to NSH and removed `gpui-mobile` (58 files, +7014/−3207). After that commit the open questions are
the Mac and generated UI.

### What NSH offers beyond the phone

Read in the NSH checkout pinned by `apps/gpui/Cargo.toml` (`website/docs/` at `058a745`):

| Capability | Source |
|---|---|
| Native views on iOS, Android, macOS (AppKit), Windows, Linux (GTK 4); HTML on the web; GPUI in GPUI apps; a terminal backend | `README.md`, "Platforms" |
| Host views (`UIView`, `NSView`, Android `View`) inside NSH screens | `native-components.md` |
| TSX modules loaded at run time in one QuickJS VM per view, with memory, time and node limits, no `eval` in release builds, and capabilities declared in `nsh.json` that a host may narrow | `sandbox-and-capabilities.md` |
| Signed `.nshpkg` packages with root and release keys, revocation and rollback protection | `signed-packages.md` |
| A documented workflow for AI-generated TSX screens | `goals.md`, "Embedding, dynamic delivery and AI" |
| `DataTable` virtualization claimed for a million rows; `Editor` claimed for files of any length | `elements/data-table.md`, `elements/editor.md` |

The NSH comparison page reports one cross-framework load test, on Linux with GTK 4 and a 500-row
quote table: at 1,000 updates per second NSH uses 18 % of a core and GPUI Fast 20 %; at 20,000
updates per second GPUI Fast uses 25 % and NSH 73–78 % (`comparison.md`, "GPUI and GPUI Fast").
No measurement exists for NSH on AppKit, for NSH inside GPUI, or for either on Sjel data.

### Precedent

On 2026-11-04 Discord announced that its desktop client moves from Electron to Rust and GPUI with
its own patches, that mobile keeps native UI and shares app code through Rust, and that web
content stays in the system web view. It published no CPU or accessibility figures
([gHacks](https://www.ghacks.net/2026/11/06/discord-is-rebuilding-its-desktop-app-in-rust-with-the-new-client-using-290-mb-of-ram/),
[Windows Latest](https://www.windowslatest.com/2026/11/05/discord-is-killing-its-electron-app-that-can-eat-4gb-of-ram-on-windows-11-and-the-rust-rebuild-uses-just-290mb/)).
The shape matches Sjel's: GPUI on the desktop, platform UI on the phone, one Rust core.

### Alternatives considered

Verdicts live in [`upstreams.toml`](../upstreams.toml). This table records why each lost. "Read"
means a current source was read on 2026-11-09; "not re-read" means the reason rests on known
properties of the project and was not checked against a current release.

| Alternative | State | Why it lost |
|---|---|---|
| NSH everywhere, AppKit on the Mac | Read | Unmeasured on Sjel's load. The only load test favours GPUI Fast under dense updates. Kept as the target that the measurement below can confirm. |
| Native per platform (SwiftUI/AppKit, Kotlin/Compose) over a Rust core | Not re-read | Two more UI languages, a UniFFI boundary, and no UI loaded at run time. Its gain, same-day Apple APIs, is already covered by the Swift host (`apps/gpui/ios/host`). |
| GPUI on the phone (`gpui-mobile`, Zed's `gpui_ios`) | Read 2026-11-07 | Rebuilds text input, selection and accessibility that UIKit provides. Removed in `b59879d0`. Discord did not take this route either. |
| Tauri or another web view shell | Read | Sjel retired its Tauri dashboard (`d9c77aae`). Strongest for generated HTML, weakest for dense data and platform text. |
| Dioxus Native (Blitz) | Read | Blitz is beta and Dioxus 0.8 is alpha ([Blitz](https://github.com/dioxuslabs/blitz), [lib.rs](https://lib.rs/crates/dioxus-native)). |
| Valdi (Snap) | Read | TypeScript compiled to native views, used inside Snapchat for eight years, open-source beta since 2025-11, Bazel build, few third-party libraries, no Rust core ([The New Stack](https://thenewstack.io/snapchat-open-sources-cross-platform-ui-framework/)). |
| Compose Multiplatform | Read | Stable on iOS since 1.8 and on desktop, but Kotlin on the JVM ([Kotlin docs](https://kotlinlang.org/docs/multiplatform/supported-platforms.html)). |
| Flutter, React Native | Not re-read | Dart or JavaScript runtime, Rust only over a bridge. |
| Slint, Iced, Xilem, Makepad, Floem | Not re-read | No phone path with platform text input and accessibility at the level NSH showed on 2026-11-07. |

### Red-team pass

Six lenses (engineer, architect, operator, adversary, newcomer, economist) attacked a first
proposal: GPUI hosts the Mac's dense surfaces, NSH draws every shared screen, and feature
extensions and agent-generated "lenses" ship as signed NSH TSX packages through
`distribution-api`. The citations the findings rest on were re-read before the council. Findings, most
severe first:

1. **"One screen serves both devices" was false in the tree.** Capture and People each keep a Mac
   GPUI page beside a reduced NSH phone screen (`gpui.md` §11, Built table). The placement rule
   ("dense stays GPUI") and the convergence claim contradict each other for Mail, Documents and
   People. Four lenses reached this independently.
2. **The TSX package path has no safe trust model yet.** QuickJS bytecode from a verified package
   "can corrupt memory" (`signed-packages.md`, "Why packages are signed"), so a package signed by a
   key on the same Mac that generates it escapes the sandbox when that pipeline is steered.
   Revocation needs the root key. Native components that the host registers ignore NSH
   capabilities (`sandbox-and-capabilities.md`). The pseudonymizer replaces entity spans only
   (`crates/pseudonymize/src/types.rs`), so instruction text in mail can steer generated code
   ([agent-safety](agent-safety.md)).
3. **`distribution-api` cannot carry a screen package.** `Package::validate` requires
   `service.toml` and a host target triple (`crates/distribution-api/src/lib.rs`, `host_platform`,
   `validate`), and the model has no signature field. A `.nshpkg` would add a second trust chain.
4. **The phone cannot roll back a bad package.** On iOS `openPackage` verifies and accepts in one
   step, which raises the build floor (`ios.md`, "Signed packages"). The health trial ignores errors
   thrown in event handlers (`ota-updates.md`, "The trial").
5. **Typed `host.call` declarations do not follow from the `*-api` schemas.** The schemas describe
   shapes, not which call name takes and returns which type, and `route-manifest` has no response
   schema (`crates/route-manifest/src/lib.rs`, `request_schema`).
6. **NSH is not contained.** Direct `use nsh_` imports in `apps/gpui` grew from 44 lines in 11
   files to 65 lines in 19 files during the session (`grep -rn "use nsh_" apps/gpui`). NSH's GPUI backend sets a
   `gpui-fast` version floor, so NSH and GPUI pins move together.
7. **Gates did not gate.** Phone waves shipped while the device check in `gpui.md` §11 was "Not
   run yet" and simulator rendering had not passed.
8. **Priority.** `PLAN.md` Priority 1 reads "Finish existing seams before introducing another
   framework", and agent UI and plugins are Priority 3. No area plan owned lenses or extensions.

### Council

A three-round council (placement, interface, adopt-first, build-here and operator members, each
round checked by a clerk) weighed the Mac and generated UI separately.

Mac: A, keep GPUI pages and leave the Mac alone · B, converge each area on one NSH screen · C,
measure first, with B as the target. Four of five members closed on C. All five agreed that a
Sjel-owned NSH facade comes first, because every measurement and every later change passes
through it. NSH Mail already runs inside the Mac app (`SJEL_NSH_TRIAL=mail`), so the first
measurement needs no new screen, only a `bench-scroll` that can launch the NSH trial
(`apps/gpui/bench-scroll` passes only `SJEL_GPUI_*` variables). Minority, the build-here member:
choose A, because measuring Documents or Finance needs NSH versions of those pages first, which is
a migration under another name. That member moves to B if NSH Mail inside GPUI matches GPUI Mail
in `bench-scroll`.

Generated UI: T, NSH TSX packages · D, a declarative catalog that Sjel's own components render · W,
model-written HTML in a web view · P, defer. T and W were rejected outright: T for findings 2 to 4,
W because model-written HTML runs model JavaScript behind a message bridge nobody has designed.
D is the only option whose contract is a schema and in which no model code runs. It is deferred to
Priority 3, after a call dispatcher with grants and a response schema exist. Minority, the
build-here member: W, because it reuses a platform component and the catalog is a component set
to version for as long as it exists.

Sjel had already set this direction before the council and neither the red-team nor the council
read it: [`projects-ui.md` §2](../website/content/architecture/projects-ui.md#2-generative-and-intelligent-ui-evolution)
describes generated UI as a declarative component catalog, with OpenUI Lang 1.0 as a candidate
format, bindings to local capability endpoints, and confirmation for mutating actions. The
council's option D reached the same shape independently.

### Decision

Recorded in [`PLAN.md`](../PLAN.md#decisions) (2026-11-09) and specified in
[`gpui.md` §11](../docs/plans/gpui.md#11-ui-stack).

### What the sources do not show (2026-11-09)

- Whether NSH inside GPUI, or NSH on AppKit, matches GPUI on Sjel's Mail, Documents or Finance
  pages. Nothing is measured.
- How the catalog's components and their schema stay small enough for one maintainer. An estimate
  is not available.
- Whether Discord's patched GPUI and Longbridge's `gpui-fast` fork converge or drift apart.
- Whether NSH's API settles. It went from v0.1.1 to v0.4.0 in 17 days.
- The phone build after `b59879d0`: `gpui.md` §11 records that simulator rendering and a linked
  host build have not passed.
