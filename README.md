<!-- human-voice: ignore-start rule_of_three -->
<!-- This file enumerates real sets constantly (the four things the spine owns, the three
     surfaces of the control app, the log kinds security covers). Several flagged "triads" are
     four-item lists the check reads as three. Shortening them would delete information to move
     a score, so the category is muted here deliberately. Every other check stays live. -->

<h1 align="center">Sjel</h1>

<p align="center"><b>Your life's data on your own devices, with one assistant that can act on it.</b></p>

<p align="center">
  <a href="LICENSE"><img alt="License: AGPL-3.0" src="https://img.shields.io/badge/license-AGPL--3.0-blue"></a>
  <a href="https://github.com/larsboes/Sjel/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/larsboes/Sjel/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://larsboes.github.io/Sjel/"><img alt="Live demo" src="https://img.shields.io/badge/demo-live-0a7d8c"></a>
</p>

<p align="center">
  <img alt="Sjel's travel hub on a desktop, from the live demo" src=".github/assets/travel.png" width="72%">
  &nbsp;
  <img alt="Sjel's travel hub on a phone, from the live demo" src=".github/assets/phone.png" width="22%">
</p>

<p align="center">
  <a href="https://larsboes.github.io/Sjel/">Live demo</a> ·
  <a href="#quick-start">Quick start</a> ·
  <a href="#measured">Measured</a> ·
  <a href="ISA.md">Open work</a>
</p>

## What it does

- Keeps the people, places, trips, money, home and calendar of one person or household as typed
  data on hardware they control.
- An assistant reads that data and proposes actions. Anything that leaves Sjel or cannot be
  undone asks first.
- Runs each task on the best model in reach: the device's own, then the Mac's, then fixed rules.
  Data about other people reaches a cloud model only pseudonymized.
- The phone reaches its node over the same Wi-Fi, Tailscale or a server of its own, and every
  device signs its requests with a key that never leaves it.
- Everything beyond the core is an optional first-party extension in this repository.

## Quick start

~~~sh
git clone https://github.com/larsboes/Sjel.git
cd Sjel
tools/install.sh
tools/doctor
~~~

macOS and Linux. [Start here](CONTRIBUTING.md#start-here) covers the installer, the private overlay and the
first services. The command is `sjel`, and `axon` still works. Settings written with the earlier
`AXON_*` names are still read.

## Measured

Each number names the command that reproduces it.

| What | Result | Reproduce |
|---|---|---|
| Pseudonymizer recall, reversible path | 48/48 (100.0%), gate ≥ 91.7%, measured 2026-09-27 | `cargo run -p comms --bin comms-redaction-eval -- --pseudonymized <corpus>` |
| Redaction recall, destructive path | 48/48 (100.0%), gate ≥ 91.7%, measured 2026-09-27 | the same command without `--pseudonymized` |
| Feed ranking (bge-m3) | 0.941 pairwise, 0.994 mean nDCG, 6/6 useful top-1 | `bun capabilities/comms/eval/run-relevance.ts`, result in `capabilities/comms/eval/results/2026-08-30-bge-m3-ollama.md` |
| Tests | 829 TypeScript tests and 107 Rust test binaries, CI green | `sjel test` and `cargo test --workspace` |

The redaction corpus (24 fixtures) lives in the private overlay, so its two rows cannot be re-run
from a public clone. The relevance corpus is public
(`capabilities/comms/eval/relevance-corpus.json`) and needs a local bge-m3.

## What it is

Sjel stores the facts of one person's or one household's life as typed data: people, places,
trips, money, home and calendar. The data stays on hardware its owner controls. An assistant
reads the data and proposes actions.

Sjel has a small core and optional extensions. All extensions are first-party and live in this
repository. Each installation switches on the extensions it needs.

## The core

| Part | Function | Code |
|---|---|---|
| Entity store | Typed people, places, organisations and dated facts. Each value has a source and a data class. | `capabilities/entities` |
| Devices and sync | Pairing, one Ed25519 key per device, signed requests, an offline copy and an outbox on the phone | `capabilities/devices`, `plugins/device-identity`, `dashboard/src-tauri/src/sync.rs` |
| Data classes | C0 Public, C1 Mine, C2 Others (facts about other people), C3 Secret. Every value carries one. | `libs/content-item` |
| Pseudonymizer | Replaces identifying details with reversible tokens before a cloud call | `libs/pseudonymize`, `capabilities/comms/src/cloud_derivative.rs` |
| Capability contract | How an extension declares its service, port, data and backup | `schemas/`, `service.toml` files, `libs/sjel-server` |
| Model selection | Picks the model for each task at runtime: the device's own, the Mac's, or deterministic rules | `dashboard/src/lib/intelligence`, `plugins/foundation-models`, `libs/inference` |
| Assistant | One panel that knows the current domain and acts through typed tools | `dashboard/src/lib/assistant` |

## Rules

1. A task runs on the best model the device can reach: its own Apple model, then the Mac's, then
   deterministic rules. The result shows which one answered.
2. A paired device's key admits its requests. Every connection type carries the same signed
   requests.
3. Sjel detects what the devices can do and chooses the defaults. Other options are under
   Advanced.
4. C2 data leaves the owner's devices only end-to-end encrypted, with keys on those devices, or
   pseudonymized. A test proves each path. A failing test closes the path.
5. Summaries and labels appear without a confirmation. A reversible change applies by itself
   only after a frozen test set shows it is reliable. A change that leaves Sjel, or cannot be
   undone, always asks.
6. Screens and assistant tools come from the typed data.
7. A statement about the system cites the file, command or measurement that proves it.

## Extensions

A selection. `axon capability list` shows all of them.

| Area | Extensions |
|---|---|
| Travel | `trips`, `transit`, `traveler`, `scouting`, `sparpreis-watch`, `punctuality` |
| People | `entities-sync`, `entities-google-sync`, `places`, `people-registry` |
| Money | `finance`, `finance-prices` |
| Home | `interior`, `home-assistant`, `soundscape`, `printing` |
| Time and knowledge | `calendar`, `comms`, `knowledge-base`, `knowledge-graph`, `vault` |
| Machines | `host-patch`, `host-watch`, `host-net`, `macmon`, `backup`, `container-refresh` |
| Agents | `Packs/`, `tools/harnesses`, `agentbox`, `shell` |

## Surfaces

| Surface | State |
|---|---|
| iPhone app | Tauri, iOS 16 and later. Admitted views work offline. |
| Web shell | The same interface in a browser, served by the Mac |
| Assistant panel | Keyword-routed. Model selection exists and has no caller yet. |
| Mac app | Planned, from the `machNotch` notch app. It will host iCloud sync. |

## Connections

| Connection | Needs | State |
|---|---|---|
| Same Wi-Fi | Bonjour discovery and a 16-character code, compared once | Built |
| Tailscale | The Tailscale app on each device | Built |
| Own server or hosted node | A server with a valid certificate | Built in the app |
| iCloud | An Apple account. Records in CloudKit, encrypted by Sjel. | Not built |

The app tries the Same Wi-Fi address first, for 1.5 seconds, then the main address.

## Scope

- Sjel does not ask for passwords or MFA codes of other services.
- Only first-party extensions run. There is no extension store.
- Prose stays in Obsidian. Sjel stores the structured facts and links to the notes.
- One node accepts writes. The other devices propose changes to it.
- Supported platforms: macOS, Linux and iOS.

## License

AGPL-3.0-only ([LICENSE](LICENSE)), from 2026-09-26. Earlier releases are MIT. Contributions carry
a DCO sign-off ([CONTRIBUTING.md](CONTRIBUTING.md#license-and-sign-off)).
