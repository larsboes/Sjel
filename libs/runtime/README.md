# sjel-runtime

Per-device runtime admission, separate from capability installation, agent permissions and
privacy policy. Workers read the overlay files before each unit of work; only `sjel-status`
writes preferences and samples the power source. No extra service runs.

## Controls

The dashboard's Systems page offers Auto, Normal and On the go, plus persistent exceptions.
The shell badge shows the effective mode and active exceptions. The operator CLI uses the same
protected API:

```sh
sjel runtime status
sjel runtime mode auto
sjel runtime mode on-the-go
sjel runtime allow other-local-models on
sjel runtime allow remote-models off
```

A fresh installation stays Normal until the operator saves a selection. Auto uses On the go on
battery and Normal on AC. A manual choice lasts until the next detected power-source change;
category exceptions remain saved independently. Whole-set exception updates carry a revision so
a stale tab or CLI cannot overwrite newer preferences.

While On the go, the default model allowance is the loopback `foundation-models` backend serving
`apple-foundationmodel`. Other local and remote/peer models have separate exceptions. Heavy work
has its own categories: `bulk-indexing`, `transcription`, `media-conversion`. Every applicable
permission must be satisfied; permitting bulk work does not permit its model implicitly.
The transcription and conversion categories reserve admission for those workflows; this public
tree currently has no speech-transcription or media-conversion engine to wire into them.

Privacy classes, provider approval and request budgets still apply. A blocked operation is
policy-deferred, not a provider failure. Existing outputs and retry budgets survive mode changes.
AFM availability and context are checked before inference; no fallback or input truncation evades
the restriction.

## State and power

`config/runtime/<device>.json` holds private preferences; `data/runtime/<device>.json` holds the
latest power sample. Device identity comes from the selected `machines/<name>.toml`, or the host
name for a single-machine overlay. Writes use atomic replacement and mode 0600.

The status process runs a bounded `pmset -g batt` probe at startup and every 30 seconds on macOS.
An Auto profile with a missing, unknown or more than 90-second-old sample restricts work rather
than assuming AC. Detection is sampled, not instantaneous. Other operating systems support manual
selection but have no battery automation in this implementation.

This governs integrated Sjel requests, not external harnesses, arbitrary builds or an external
server's resident model memory. It does not promise offline operation or a measured battery saving.

Checks: `cargo test -p sjel-runtime -p sjel-inference -p sjel-summarize`.
