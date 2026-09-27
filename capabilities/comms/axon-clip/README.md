# Axon Clip — Browser Extension

`axon-clip` is a Manifest V3 browser extension for Chrome and Brave that clips web pages, text selections and URLs into the local Axon Feed (`capabilities/comms`).

It captures and posts, nothing more. Extraction, normalization, storage, scoring and grouping all belong to `comms-server`; this holds no parsing logic and makes no model calls. It stays a human-triggered capture of a page the operator is already looking at. It does not automate access, and it collects nothing in the background.

## Why it lives here and not in `capabilities/`

Settled 2026-07-31 (#82). It is the client half of one capability's `/ingest` route, with no other consumer, no data of its own and no process to run, so CONTRIBUTING.md#integrate-first-topology's integrate-first default holds and none of its escape hatches apply. What was chosen against: a top-level `capabilities/axon-clip/`. That would have bought a directory and a concept while the thing still only ever talks to comms over HTTP, and a capability with no `service.toml` has nowhere to declare `requires = ["comms"]` anyway.

## What it does

Three things get clipped: the whole page, a selection, or a bare link. The first is the one that matters, because it reads the rendered DOM out of the tab you are already in. That is the only way to capture a page behind a login or one that renders client-side, and it is why the server never needs your credentials.

Every path can be driven three ways: the toolbar popup, a right-click, or `Cmd+Shift+Y`, which clips the current page without opening anything. `Cmd+Shift+K` opens the popup instead. Both are Chrome *suggested* keys, so if something already owns them, rebind under `chrome://extensions/shortcuts`.

Whatever you clip, you get its id back. The popup prints it; the shortcut and right-click paths have no popup to print into, so they raise a notification carrying it. That id is what finds the item in the feed afterwards, and what tells you a second clip of the same URL updated one row rather than adding another. A failure is equally loud: there is no path where a capture quietly does not happen.

Requests go to `POST /ingest` with your `comms-server` shared secret, tagged `client: "axon-clip"`. The server stores that tag, so an item you handed over stays distinguishable from one it fetched itself.

## Installation

1. Open Chrome or Brave and navigate to `chrome://extensions`.
2. Enable Developer mode in the top-right corner.
3. Click Load unpacked and select `capabilities/comms/axon-clip`.

## Configuration

1. Click the Axon Clip icon in the toolbar.
2. Open Settings with the gear icon at the top right of the popup.
3. Set the comms server base URL (`http://127.0.0.1:8083`) and the API shared secret, the one your overlay `comms.json` points at via `api_secret_file`.
4. Save.
