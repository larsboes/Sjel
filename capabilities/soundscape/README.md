

# soundscape

Generative audio that follows what you are doing. This process is the **conductor**:
it owns what should be playing. The sound itself is made in the browser.

## Why this shape: the conductor is here, the synth is not

Web Audio runs in a browser and cannot run anywhere else. A Rust process cannot
generate this sound, and rendering audio here to stream it back would be a worse
system for no gain: higher latency, an encoder in the path, and a server that has to
stay up for a page to make noise.

So the split is not "backend and frontend". It is **intent and execution**. This
process owns the preset, the six scene parameters, the layer mix, the seed and any
timed session --
the answer to "what should be playing" -- because that answer has to survive a
reload, be identical on every surface, and be changeable from a phone. The browser
owns oscillators, scheduling, and the arrangement clock.

That is what earns a capability rather than a `libs/` entry: a bounded domain with
its own state and its own contract, per `CONTRIBUTING.md#placement-guide`.

### What this process deliberately does not own

**The arrangement clock.** The musical phase is a function of the audio context's own time,
which exists only in the browser. Publishing a phase from here would put two clocks
in disagreement about the same bar, and the one that is wrong would be this one.
`GET /api/soundscape/state` therefore carries no phase; the surface computes it.
The conductor does own a session's duration and elapsed wall time because every
surface has to agree whether a 50-minute focus session is still running.

**Mood.** Deriving preset and parameters from calendar, presence and activity is a
real intention and a separate piece of work, kept separate on purpose: this process
first has to be the boring, correct home for state before it starts having opinions
about it.

## Endpoints

- `GET /health` -- liveness, plus the current preset and how many clients are on the stream
- `GET /api/soundscape/health` -- the same, under the API prefix the dashboard proxies
- `GET /api/soundscape/state` -- preset, params, layer mix, seed, playing, volume, energy, session
- `POST /api/soundscape/state` -- a **partial** update; every field is optional
- `GET /api/soundscape/stream` -- SSE, current state first and then every change
- everything else -- the built UI bundle, with an SPA fallback

### Why the update is a patch and not a PUT

Two surfaces drive this at once: the capability's own page and the spine's mini
player. With a whole-state PUT, a client holding a stale copy silently reverts
whatever the other one just changed. A patch can only assert what it actually
touched. Unknown field names are rejected rather than ignored, because a typo that
does nothing is the worst failure mode for an API two clients write to.

Values outside 0-1 are clamped rather than rejected -- a slider that sends 1.0001 is
not an error worth surfacing -- but an unknown preset is a 400 with the known list,
because that one is a real disagreement about what exists.

## Port and the UI bundle

Default `8088`, resolved through `libs/sjel-config` (`SJEL_PORT`, then
`SJEL_SOUNDSCAPE_PORT`, then the default). Declared once in `service.toml`;
nothing else hardcodes it.

`autostart = "false"`: nothing depends on this being up, and a soundscape that
starts itself on boot is a machine making noise decisions nobody asked for. The
dashboard brings it up when its surface is opened.

The UI is served from this same process, so the panel arrives and leaves with the
capability (CONTRIBUTING.md#three-architectural-nouns) instead of living in the spine shell. The bundle
path comes from `SJEL_SOUNDSCAPE_UI` and defaults to `capabilities/soundscape/ui/dist`,
the directory `bun run build` writes — a build output, never a checked-in one.

## Building the UI

`ui/` is a bun package built by `bun run build` (`svelte-kit sync && vite build`).
`service.toml` declares that as `panel_build`, so the runner produces the bundle
before it compiles the server that serves it. Dependencies are an operator step,
once per clone:

```bash
cd capabilities/soundscape/ui
bun install --frozen-lockfile --ignore-scripts
```

`--ignore-scripts` is not optional here — see `tools/check-bun-install-policy.sh`. The local
`bunfig.toml` also holds fresh npm resolutions for 24 hours and runs Socket's scanner; inspect
the committed tree with `bun pm scan`.

`vite dev` is untouched and is still how you work on the UI. `panel_build` owns the
served bundle, not the edit loop.

Until 2026-08-25 that bundle was a Bazel action (`//tools/bazel/bun:build.bzl`) with
a fetch-time dependency install. PRD Q44 retired Bazel; what the hermetic toolchain
bought here — a bundle nobody had to remember to rebuild — is now the `panel_build`
line, and reproducibility of the bytes is the lockfile's job alone.

One nondeterminism had to be fixed before any of that was worth anything: SvelteKit
defaults its app version to `Date.now()`, which reaches the entry chunks and
changes their content hashes, so two identical builds produced different bytes.
It now reads `SJEL_BUILD_VERSION`, defaulting to `dev`.

## Persistence

With `SJEL_PERSONAL_ROOT` configured, the conductor stores the latest scape at
`<overlay>/data/soundscape/scape.json`. Writes are atomic and coalesced across a
two-second window so a dragged slider is one disk update rather than dozens. Without
an overlay, state remains in memory.

Playback never resumes on process start. A running session is restored as paused with
its elapsed time preserved; a browser gesture is still required before sound can begin.
