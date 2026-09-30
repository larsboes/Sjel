# Sjel Lens

A right-sidebar pane in Obsidian that answers one question about the note in front of you:
**what does Sjel know about this, and does it agree with what the note says?**

Two note kinds so far.

| Note | Reads | Shows |
| --- | --- | --- |
| `Atlas/People/*.md` | vault `GET /api/people` | `last_contact`, `met_at` and `mention_count` computed from the Journal, beside the values stored in the note, with each disagreement marked |
| `Resources/Sjel/Trips/*.md` | trips `GET /api/plans` | a staleness badge: whether the projection's `axon_revision` still matches the plan's `updated_at` |

The trip folder and the `axon_*` keys are what trips writes today
(`capabilities/trips/src/projection.rs`, `DIR` and `frontmatter`). They carry the old name,
and the plugin matches the code rather than the name.

A person note is found by its path, and the two sides spell a path differently: vault serves
back what the filesystem gave it, and three of the 89 files under `Atlas/People/` are stored
decomposed (NFD) on this APFS volume, while Obsidian normalises to NFC. The lookup therefore
compares canonical-equivalent paths. Measured 2026-09-09 with an exact `===`, those three
notes read *"vault has no facts for this note"* in the pane; with the comparison, all three
are found.

The badge is the useful one. A projection looks the same whether it is current or four
revisions behind, and until now the only way to find out was to re-run the export and watch
whether the file changed.

## The three properties that are not negotiable

**It reads and never writes.** No note is modified, no frontmatter is written, no command
mutates anything. vault computes the people facts and serves them beside the stored values;
it does not write them (`capabilities/vault/src/people.rs`). A pane that wrote the same keys
would be a second writer with no shared refusal. `tools/check-obsidian-plugins.sh` refuses a
write here.

**It uses `requestUrl`, never a browser request.** `requestUrl` is not a browser request:
Obsidian's renderer hands the call to the main process over the `request-url` IPC channel,
and the main process issues it with Electron's `net.request`, setting only `Content-Type`
and the caller's own headers. No `Origin` is produced, so the capability takes the same path
as curl and the runner's health probes.

A `fetch()` from the plugin would run in the renderer and carry `Origin: app://obsidian.md`.
The origin guard admits that exact origin (`libs/sjel-server/src/origin.rs`,
`origin_allowed_by`), and vault and trips carry a permissive CORS layer, so those two would
answer. Discovery would not: sjel-status has no CORS layer
(`capabilities/sjel-status/src/main.rs`, `build_router`), so the renderer withholds the
registry's reply from a cross-origin `fetch`. `requestUrl` needs neither the origin
allowance nor a CORS header, and the gate keeps it that way.

The claim that `requestUrl` sends no `Origin` comes from reading the `request-url` handler
in the Obsidian build that was installed on 2026-09-09, not from watching a request leave
Obsidian.

**No port literals.** A capability's port is a machine fact — `[capability.<name>] port` in
the overlay overrides the manifest — so every address comes from the registry `sjel-status`
serves at `/api/sjel-status/capabilities`, which is the same shape `tools/capability.sh
registry` prints. Each capability's base is derived from the `health_url` the registry
computed. One bootstrap address is left, because discovery has to start somewhere; it is a
setting, and it is the only one.

**It carries no token.** A deployment that declares `SJEL_INBOUND_TOKEN_FILE` answers `401`
on every route except `/health` and `/ready` to a caller without the token
(`libs/sjel-server/src/auth.rs`, "The contract"). The pane then prints that status in each
section. The plugin stores its settings in the vault, so it does not hold the token.

## Degrading

Per route, never globally. A capability that cannot be reached costs one muted line in its
own section and nothing else; another adapter on the same note still renders. The refusals
are distinct because their fixes are:

```
punctuality          REFUSED: punctuality is not running
knowledge-base       REFUSED: knowledge-base serves no HTTP surface
no-such-capability   REFUSED: no-such-capability is not enabled on this machine
```

Discovery failing is reported as itself — "sjel-status did not answer, so nothing could be
discovered" — rather than as "vault is down", which would be a guess with a different fix.

## No build step

Obsidian's plugin loader reads exactly `main.js` and evaluates it as
`(function anonymous(require, module, exports) { … })` with a `require` that resolves
`obsidian` and node packages and nothing relative. A second source file would not be loaded
at all, so this file **is** the plugin: nothing to compile, and no dependency that can go
unmaintained.

`load-plugin.js` beside this directory reproduces that loader, so the test and the probe
exercise the bytes Obsidian will run. A `main.js` Obsidian could not load fails there first.

## Install it

Nothing installs this for you, and the build produces no artifact: the three files in this
directory are what Obsidian wants. From the repository root, with `<vault>` the vault's root
directory:

```
mkdir -p "<vault>/.obsidian/plugins/sjel-lens"
cp capabilities/knowledge-base/plugin/sjel-lens/{manifest.json,main.js,styles.css} \
  "<vault>/.obsidian/plugins/sjel-lens/"
```

Copy again after a change here; Obsidian does not follow the repository. Then in Obsidian:
**Settings → Community plugins → Installed plugins**, reload the list, and turn on *Sjel
Lens*. Open the pane from the ribbon's scan-eye icon or from the command palette (*Sjel
Lens: Open the pane*). Open a note under `Atlas/People/`.

If Sjel's ports have been moved on this machine, set the registry address in **Settings →
Sjel Lens**. Nothing else in the plugin needs changing.

To uninstall, turn it off and delete the directory. It stores its settings in `data.json`
beside itself and touches nothing else.

## Check it

```
bun test capabilities/knowledge-base/plugin/sjel-lens.test.js
tools/check-obsidian-plugins.sh
bash tools/check-obsidian-plugins.test.sh
bun capabilities/knowledge-base/plugin/sjel-lens.probe.js --vault "<vault root>"
```

The first three are hermetic, and CI runs all three: the root `bun test` finds the test
file, the shell-tests loop runs every `tools/*.test.sh`, and the repo gates run the gate. The probe is the falsifier: it loads this `main.js` through
the same loader, points its adapters at the services that are running right now, and prints
what they answer for real notes — including one person whose stored values disagree and one
whose do not, and a planted revision one hour behind its plan, which must come back `stale`
rather than `current`.

## What it deliberately does not do

- **No writing at all.** vault computes these values and does not write them; this pane
  only shows the difference.
- **No third adapter.** Tasks and finance both have HTTP surfaces and neither has a note
  kind that carries a Sjel-owned key to compare against, so a pane for them would restate
  the note back to itself.
- **No caching beyond the registry.** People and plans are re-read per note. Both answer in
  under 30 ms on loopback, measured 2026-09-09, so a cache would buy nothing and could show
  a stale badge — which is the one thing this pane must never do.
