# soundscape

Generative audio that follows what you are doing. This process is the **conductor**: it owns
what should be playing. The native app makes the sound
(`apps/gpui/core/src/soundscape/`, see
[the GPUI README](../../apps/gpui/README.md#soundscape-a-slideover-with-its-own-synth)).

## Why this shape: the conductor is here, the synth is not

The split is **intent and execution**. This process owns the preset, the six scene parameters,
the layer mix, the seed and any timed session: the answer to "what should be playing". That
answer has to survive a restart, be identical on every device, and be changeable from a phone.
The app owns the oscillators, the scheduling and the arrangement clock, against its own audio
device.

That is what earns a capability rather than a `libs/` entry: a bounded domain with its own
state and its own contract, per `CONTRIBUTING.md#placement-guide`.

### What this process deliberately does not own

**The arrangement clock.** The musical phase is a function of the audio device's own time,
which exists only on the device that is playing. Publishing a phase from here would put two
clocks in disagreement about the same bar. `GET /api/soundscape/state` therefore carries no
phase. The conductor does own a session's duration and elapsed wall time, because every device
has to agree whether a 50-minute focus session is still running.

**Mood.** Deriving preset and parameters from calendar, presence and activity is separate work.

## Endpoints

- `GET /health`: liveness, plus the current preset and how many clients are on the stream
- `GET /api/soundscape/health`: the same, under the API prefix the shell proxies
- `GET /api/soundscape/state`: preset, params, layer mix, seed, playing, volume, energy, session
- `POST /api/soundscape/state`: a **partial** update; every field is optional
- `GET /api/soundscape/stream`: SSE, current state first and then every change

### Why the update is a patch and not a PUT

The Mac and the phone can drive this at once. With a whole-state PUT, a client holding a stale
copy silently reverts what the other one just changed. A patch asserts only what it touched.
Unknown field names are rejected rather than ignored, because a typo that does nothing is the
worst failure mode for an API two clients write to.

Values outside 0–1 are clamped rather than rejected, but an unknown preset is a 400 with the
known list, because that one is a real disagreement about what exists.

## Port

`8088`, `autostart = false`: nothing depends on this being up, and a soundscape that starts on
boot makes noise decisions nobody asked for. Start it from the app's Capabilities page, or with
`tools/service-runner.sh start soundscape`.

## Persistence

With `SJEL_PERSONAL_ROOT` configured, the conductor stores the latest scape at
`<overlay>/data/soundscape/scape.json`. Writes are atomic and coalesced across a two-second
window, so a dragged slider is one disk update. Without an overlay, state stays in memory.

Playback never resumes on process start. A running session is restored as paused with its
elapsed time preserved.
