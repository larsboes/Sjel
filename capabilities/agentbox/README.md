# agentbox

A coding agent in a disposable container, talking to a model server on the host. One project
directory in, nothing left behind on exit. Since 2026-09-02 the network is one of two things and
nothing between them: none at all, or the open internet.

A modern coding agent reads your files, runs shell commands and installs whatever it decides it
needs. That is what makes it useful, and it is also why running one directly on a work machine
hands it `~/.ssh`, `~/.aws`, every other repo on the disk, and outbound network reach under your
identity. The interesting question is not whether the model is good. It is what exactly this
process can touch, and whether you can show it.

## The pieces, and why each one is there

```
  host (macOS, Apple Silicon)               one shared Linux VM (OrbStack)
 ┌────────────────────────────┐  default    ┌──────────────────────────┐
 │  model server              │  bridge,    │  the agent (pi)          │
 │  OpenAI /v1 + tool-calling │◄────────────┤  git, ripgrep            │
 │  bound 0.0.0.0             │  --online   │  /workspace  ← 1 project │
 │                            │  ONLY       │  /home/agent/config      │
 └────────────────────────────┘             └──────────────────────────┘

  Without --online the box runs on `--network none`: loopback and no other interface.
  No route, no resolver — and no model endpoint either.
```

- **[pi](https://github.com/earendil-works/pi)** is the agent: an OpenAI-compatible coding CLI
  with `read`/`write`/`edit`/`bash` tools. It is here because it points at *any* endpoint
  speaking chat-completions, which is what makes a local model a first-class option rather than
  a downgrade, and because it ships a real release binary.
- **Docker is the runtime, and the boundary is two things rather than one.** On this Mac the
  daemon is [OrbStack](https://orbstack.dev), which runs every container inside ONE shared Linux
  VM: that VM is the boundary between a container and macOS, and namespaces plus cgroups are the
  boundary between one container and the next. This capability was adopted for a stronger
  sentence — `apple/container` gave each container its own VM — and that sentence stopped being
  true here on 2026-09-02 (Q75). It is written this way rather than dropped, because a
  claim that quietly weakens is worse than one that says what it is worth.
- **`--network none` is the whole egress control, and it takes the model with it.** Docker offers
  no state between "no interface" and "the default bridge, which reaches the internet and the
  Mac". `docker network create --internal` is not the middle: measured 2026-09-02, a container on
  one has no default route, cannot resolve `host.docker.internal`, and reaches neither 1.1.1.1
  nor the host. apple-container's host-only network *was* the middle, because its gateway was the
  Mac. That is the property this retirement gave up, and it is why the default box can no longer
  answer a prompt. See "The boundary, and how it was checked".
- **The model server stays on the host** because MLX needs Metal and the ANE, and a Linux VM
  exposes neither. That constraint is not a nuisance to work around; it is what produces the
  split above.

Verdicts and licences for pi and for the retired `apple/container` live in
`upstreams.toml`, not here. No version does: `agentbox build` and `agentbox host-install`
resolve the latest release themselves (Q77, 2026-09-02). Docker has none: `toolchain.toml [docker]` records that Axon assumes
the `docker` CLI, and *which* daemon provides it — OrbStack here, Docker Desktop or colima
elsewhere — is a machine fact, not adopted code.

## The model side

The box needs one thing from the host: an HTTP endpoint speaking OpenAI chat-completions **with
tool-calling**. Without tool-calling the agent is inert — it answers, it never edits, and nothing
in the logs says why. That is the single most common way this setup looks broken while being
correctly configured.

Which server is a config value, not a design decision. [oMLX](https://github.com/jundot/omlx) and LM Studio
are the MLX-native options on Apple Silicon; llama.cpp's server and Ollama's `/v1` speak the same
protocol. Swapping one for another is the overlay's `model_base_url` and nothing else — the
provider inside the box is called `local` precisely so it names the endpoint rather than the
software behind it. Whichever runs has to bind `0.0.0.0` rather than loopback, and therefore has
to enforce an API key: see Setup.

For the model itself, the useful class is a coder-tuned MoE at 4-bit — around 30B total with a
few billion active parameters, roughly 17 GB resident, which is what makes this viable on a
laptop at all. A general instruct model works and is noticeably worse at holding a multi-step
tool loop together. The concrete id, its context window and its token limits are instance
values and live in the overlay's `agentbox.toml`, because the answer differs per machine and per
how much RAM you are willing to give up.

## Verdict

**Adopt pi, contain it with docker.** pi's own `docs/security.md` says it ships no sandbox by
design and that isolation has to come from the OS or a virtualization boundary, so this follows
the tool's security model rather than bolting one on. Three things follow from that, and they are
the whole design:

1. **The agent binary lives only in an image.** Consumed as the release tarball, sha256-verified
   on the host before the build and again inside it, so the image needs no Node and no npm and
   CONTRIBUTING.md#language-tooling is satisfied rather than excepted. Which release is decided at
   build time — GitHub's `/releases/latest` — and the checksum comes from that release's own
   published `SHA256SUMS`, never from a literal in this repository.
2. **The box is disposable, and its default network is nothing.** `--rm` discards the container
   and its writable layer on exit; `--network none` gives it loopback and no interface. Under
   `apple/container` this bullet also said "and it still reaches the model," because that
   runtime's host-only network put the Mac on the gateway. Docker has no such network — see the
   second bullet above — so the honest version of this claim is narrower: *the closed box reaches
   nothing, including the model; the open box reaches everything.* The allow-list Q24 wanted is
   not reproduced here, and pretending otherwise would be the failure this repo names in
   CONTRIBUTING.md#decisions-live-with-their-owner.
3. **Exactly two things cross the boundary,** both by mount: one project directory at
   `/workspace`, and the agent's config directory. Never the host's own agent config — mounting
   that would hand the container your sessions, settings and credentials, which upstream's
   security doc calls out by name.

Inference stays native on the host. MLX needs Metal and the ANE, which a Linux VM does not
expose, and that constraint is what produces the split: model on the host, tool-calling in the
box. What changed on 2026-09-02 is only how the box reaches across it.

## Considered and declined

- **Gondolin** ([earendil-works/gondolin](https://github.com/earendil-works/gondolin), pi's own
  micro-VM extension) — keeps pi on the host and routes only its built-in tools into a VM.
  Rejected because it inverts the property this capability exists for: the agent process, its
  extensions and its credentials all stay on the host, and it needs Node ≥ 23.6 plus QEMU
  installed there. The point here is that the host runs the model and nothing else.
- **NVIDIA OpenShell** — policy-controlled sandbox with filesystem, process, credential and
  inference controls, the richest of the three upstream patterns. Rejected for now because every
  sandbox needs a running gateway: a second service to operate and audit for a box that a
  closed network and two mounts already contain.
- **`apple/container`** — what this capability was built on until 2026-09-02, and the reason its
  first security sentence was *each container is its own VM*. Retired by Q75: one
  container runtime on this machine instead of two, on a machine where no enabled capability was
  container-backed at all. What the retirement costs is stated in "The pieces" and in the Verdict
  above, not buried here. `upstreams.toml [apple-container]` holds the dated verdict
  of what ran.
- **A relay container forwarding one port**, to keep the model reachable from an otherwise-closed
  box: a second container attached to both an `--internal` network and the default bridge,
  forwarding `model_port` to the host. Measured working end to end on 2026-09-02, so this is a
  declined option and not an untried one. Declined on cost: the agentbox base image has no
  `socat`, `nc`, `busybox` or `python3` (only perl), so it needs a *new* external image with an
  `upstreams.toml` verdict, for external code holding a live route from an isolated
  network to the Mac. It also has a lifecycle `agentbox run` cannot supervise — the launcher
  `exec`s the box, so no exit trap survives to tear the relay down — and a leaked relay is
  exactly the open path the box exists to prevent. A closed box that says it has no model beats a
  box whose isolation depends on cleanup that may not run.
- **A bridge network plus an egress filter** — the other way to keep the endpoint without a
  relay. Declined: docker has no per-destination allow-list, so the filter lives either inside
  the container, where it needs `NET_ADMIN` and an agent with `NET_ADMIN` can remove it, or on
  the host, where `capabilities/host-firewall` is unconfigured on this machine.
- **podman** — the CLI verbs match and the network semantics do not, and nothing here has
  measured them; it also spells the host `host.containers.internal`. `agentbox` refuses it by
  name rather than accepting it untested. `tools/service-runner.sh` still supports it for
  container capabilities, where the claim is availability rather than isolation.
- **npm-installing the agent into a `node:22` image**, the shape of the writeup this was built
  from. Declined on two counts: CONTRIBUTING.md#language-tooling bans npm in Axon code, and the release binary makes the
  entire Node layer unnecessary. The writeup also predates the project's rename, so its package
  (`@mariozechner/pi-coding-agent`) stopped receiving releases at 0.73.1 while pi moved on to
  0.82.x under `@earendil-works`.

## Layout

```
Containerfile                  agent-neutral: base image, git + ripgrep, non-root user, the archive
agentbox                       build / run / shell / doctor
agentbox.toml.example          the shape of <overlay>/config/agentbox.toml
profiles/pi/
  profile.toml                 release + checksum URLs, config-dir contract, default model args
  AGENTS.md.tmpl               the operating contract, rendered into every session
  models.json.tmpl             provider definition, rendered with the endpoint and key
  extensions/protected-paths/  the seatbelt for the day a mount gets widened
```

Nothing under `profiles/` names a machine, and nothing anywhere names a version. Bumping the
agent is `agentbox build` — it resolves the latest release, verifies it against that release's
`SHA256SUMS`, and tags the image both `:latest` and `:<version>` so what was built stays
readable. Changing the model or the endpoint is the overlay's `agentbox.toml` and nothing else.

## Setup

1. **The model server bound to `0.0.0.0`**, not only `127.0.0.1`. A container reaches the host
   through a bridge address, never the host's own loopback; that is the most common reason "it
   can't reach the model". Binding wider also puts the endpoint on whatever network the machine
   is currently joined to, so its API-key check has to stay on — from that moment the key is the
   access control, and on an untrusted network the honest move is to bind back to loopback and
   let the box stop working.
2. **The key**, in the overlay's gitignored env file. Write it yourself, in your own terminal;
   never through an agent turn (CONTRIBUTING.md#secrets). `secrets/agentbox-model-key.md` in the overlay has the
   command.
3. **`<overlay>/config/agentbox.toml`** — copy `agentbox.toml.example`, set the model id and
   port.
4. `agentbox build`, then `agentbox doctor`.

Grant macOS Local Network access if it asks. That requirement was verified for `apple-container`
and is **unverified for OrbStack**: the 2026-09-02 container-to-host probe below succeeded with
no prompt, but the machine's application firewall has "automatically allow signed software"
enabled, so a prompt-free success does not prove the permission is unnecessary. If requests to
the model hang with no error, look here first: *System Settings → Privacy & Security → Local
Network*.

## Use

```sh
agentbox run --online --project ~/code/some-repo   # the working agent, on that project
agentbox run --online                              # the working agent, current directory
agentbox run                                       # --network none: no model, so no answers
agentbox shell                                     # a prompt in the closed box, no agent
agentbox run --online -- --tools read,grep,find,ls -p "Review src/"   # after -- goes to the agent
```

**`--online` is not optional for a session that has to think.** The default box has no network,
which since 2026-09-02 also means no model endpoint, so the agent starts and every turn fails.
The launcher prints that to stderr rather than letting you find out from a stack trace. The
default is still the closed one, because the safe mode losing a capability is not a reason to
make the open mode the one you get by not choosing.

What the closed box is still good for: `agentbox shell`, and running a build or a test suite from
an untrusted checkout with no route off the machine.

`--project` is the only host path the box sees, so set it deliberately — unset, it mounts the
current directory. Home and `/` are refused outright.

Files the agent writes into the project are owned by you: the image is built with your uid and
gid, which is why they do not land as a foreign uid 1000.

## What host-install refuses

`agentbox host-install` puts the latest agent release on the host itself, outside the box, and
writes `<overlay>/data/agentbox/host/installed.json` recording which version landed, verified
against which sha256, on what day.

The receipt's `verified` field says which run did the hashing, because a `host-install` whose
resolved version is already on disk installs nothing and therefore hashes nothing. It reads
`this-run` after an install or a `--force`, `earlier-run` when the digest was carried over from
the previous receipt for the same release, and `unknown` — with an empty `sha256` — when there
was no such receipt to carry it from.

What it enforces:

| Check | Kind | Source of the rule |
|---|---|---|
| sha256 of the release archive | hard refusal, in `fetch_archive` on the install path | the release's own published `SHA256SUMS` |
| published GHSAs against the version | **not checked — yours to do** | see below |

**The self-update shim is gone (Q77, 2026-09-02).** `host-install` used to write a wrapper
that intercepted `pi update|upgrade|self-update` and refused them. It existed to protect a
version literal: a self-updating binary replaced the file on disk and left `profile.toml`'s
`version` and `archive_sha256_*` describing software the machine had stopped running — which
happened here, `pi update` about 15h after a release, with the pin written afterwards to
describe an adoption that had already occurred (Axon#126). Those literals are deleted, and this
launcher now resolves the latest release on every build and install. The shim would be refusing
the outcome the ruling asks for, so it is removed rather than left as a rule with nothing behind
it. `<host-root>/bin/<agent>` is a plain symlink through `current` now.

The cost, stated: a self-update fetches its own bytes, and this launcher never sees them. The
sha256 above covers what `agentbox host-install` puts on disk and nothing else.

There was a separate `agentbox gate` verb, and a cooldown on how old the release was, until
2026-09-02. Q74 removed the adoption hold (CONTRIBUTING.md#patch-first), and with it gone the verb
refused nothing at all — it resolved a repository, printed two lines and returned 0. It is
deleted rather than kept, because a step that cannot say no is how a green line comes to read as
a passed check. What remains is the sha256, which was never in the gate, and the printed
advisory reminder, which `host-install` now prints itself.

**Advisories are a manual step, and host-install prints that every run.** Until 2026-08-28 it
fetched published advisories and refused an install whose target sat inside an affected range.
PRD Q41 retired the script behind that (`tools/lib/advisories.sh`) with the rest of Axon's
homegrown supply-chain plumbing, and no standard tool replaces it *for this dependency*: the
agent arrives as a release tarball, so it appears in no lockfile, which is exactly what
GitHub's Dependabot alerts and `osv-scanner` both read. Dependabot does not watch release
tarballs either, so the tool swap changes nothing here — and a new tag is freshness, not
exposure.

So before you accept a bump, open
[the repository's advisories](https://github.com/earendil-works/pi/security/advisories) and read
them against the version you are about to install. This is a genuine reduction in what the
machine checks for you, stated here rather than discovered later — and host-install refuses to
be silent about it, because a step that prints nothing reads as a step that passed.

## The boundary, and how it was checked

**Current, 2026-09-02.** M4 Pro / macOS 26.5.2 / docker 29.4.0 on OrbStack 2.2.3, from the
`debian:trixie-slim` base the agentbox image is built on (a dated `trixie-20260713-slim` tag
until Q77; the measurement was taken on that day's build of it). Same probe, three networks, so
the controls show what each mode actually buys:

| Probe from inside the box | `--network none` (default) | `--internal` | default bridge (`--online`) |
|---|---|---|---|
| interfaces | `lo` only, no route table at all | `eth0`, one on-link /24, no default route | `eth0` + default route |
| TCP 1.1.1.1:443 | fails in ~1 ms | fails in ~1 ms | reachable in ~9 ms |
| DNS lookup | fails in ~2 ms | fails in ~1 ms | resolves in ~22 ms |
| `host.docker.internal` | does not resolve | does not resolve | `0.250.250.254` |
| the model on the host, `:8000` | unreachable | unreachable | `HTTP/1.1 401` |

A 401 is the pass condition on the last row: it proves the endpoint answered. It was taken
against a throwaway listener bound to `0.0.0.0:8000` on the Mac and torn down afterwards, because
nothing was serving the model on this machine that day.

**`--internal` is in the table to close the obvious question.** It is the shape that looks like
apple-container's host-only network and is not one: it blocks egress *and* the host, so column
two buys nothing over column one except a NIC and a resolver the box can talk to. Docker has
nothing between column one and column three, and that gap is the whole of what Q75 traded
away.

Two smaller findings from the same afternoon. The fast DNS failures are a property of the image,
not of docker: the same `--network none` probe on Alpine takes 10.03 s, because musl retries the
unreachable nameserver until both timeouts expire. And `--dns 127.0.0.1` changes none of these
numbers — docker applies it as an upstream forwarder rather than as the resolver — so the flag
was dropped rather than carried over from the apple-container launcher.

**Historical, 2026-07-28**, M4 Pro / macOS 26.5.2 / `container` 1.0.0 — kept because it is the
record of the stronger claim this capability used to make, and the thing the table above should
be read against:

| Probe from inside the box | `agentbox` (hostOnly) | `default` (control) |
|---|---|---|
| TCP 1.1.1.1:443 | blocked | reachable |
| DNS lookup | dead | resolves |
| model endpoint on the host gateway | `HTTP/1.1 401` | — |

One column, blocking egress and keeping the model. That is the row docker cannot produce.

Then the same five things through a real session, because a blocked socket says nothing about
what the agent does with one (2026-07-28, under `apple/container`):

| Asked of the agent | Result |
|---|---|
| answer a prompt | round-trips the model on the host |
| list and read a file in `/workspace` | reads it, reports the contents |
| edit a file in `/workspace` | lands on the host file, still owned by the invoking user |
| `cat` its own config directory | `Blocked by agentbox: /home/agent is off limits inside the box` |
| `git ls-remote` a GitHub url | `Could not resolve host: github.com`, exit 128, reported honestly |

The fourth is the extension doing its job, not the mount: `/home/agent/config` is mounted and
readable, and holds the model API key. The boundary keeps the host out; the extension keeps the
agent out of its own credentials.

Both of that day's findings were about `apple/container` and neither survives the port. The
`--no-dns` fallback to an unreachable 1.1.1.1 was that runtime's behaviour; docker writes its own
resolver into `/etc/resolv.conf`, and on `--network none` a lookup fails immediately rather than
timing out, so the `--dns 127.0.0.1` workaround is gone. And a host-only network's bridge
interface existing only while a container is attached was the reason `doctor` probed from inside
a throwaway box. `doctor` still does — for a better reason. Probing from the host answers whether
the *server* is up, which passes when it is bound to loopback only; probing from a container
answers whether a container can reach it, which is the question.

## Troubleshooting

- **Every turn fails to reach the model.** Check for `--online` first. The default box has no
  network at all, so the endpoint in its rendered `models.json` resolves to nothing; the launcher
  says so on stderr before the agent starts.
- **The agent answers but never edits.** The model has no tool-calling, or the server does not
  support it. A chat-only model looks like it works and silently does nothing.
- **Requests hang with no error.** macOS Local Network permission, historically. Unverified for
  OrbStack — see Setup — but it costs nothing to check.
- **`doctor` says silence at the endpoint.** The server is bound to loopback only, or it rejects
  `host.docker.internal` as a Host header.
- **A build inside the box uses more parallelism than it was given.** `--cpus` is a CFS quota,
  not a vCPU count: `nproc` reports all 12 host cores and the box is throttled at its share
  rather than refused. Under `apple/container` the box was a VM and `nproc` matched.
- **A task genuinely needs the network.** `--online`. It is a deliberate, per-run decision, and
  the session's operating contract is rendered to say so.

## Not covered

Multi-project and concurrent sessions, image lifecycle beyond `build`, and supply-chain
attestation for the image itself. **Egress filtering is no longer covered either.** The writeup
this came from left it out of scope; this capability had it until 2026-09-02 and now has the two
ends of it instead — all or nothing. A per-destination allow-list is the shape Q24's v1 profile
wants, and building one on docker is future work, not something here.

## Origin

The pattern comes from Michael Hannecke's [*A Sovereign Coding Agent on macOS — PI in an Apple
Container; Zero npm on the
Host*](https://medium.com/@michael.hannecke/a-sovereign-coding-agent-on-macos-pi-in-an-apple-container-zero-npm-on-the-host-46f62ffade0a)
([repo, archived](https://github.com/michaelhannecke/pi-container)). The architecture is his: the
agent in a container, model native on the host, config and one project as the only mounts. The
Apple-container half of it is no longer what this repo does.

Two things in it no longer hold, which is worth knowing before following it directly. The project
was renamed — `badlogic/pi-mono` is now `earendil-works/pi`, and its npm package moved from
`@mariozechner/pi-coding-agent` (last release 0.73.1) to `@earendil-works/`, so the article's
`npm install -g` pulls a version months behind a project that ships a self-contained binary and
needs no Node at all. And its `192.168.64.1` is a default, not an address: address the host by
name, which is what the launcher does (`host.docker.internal`).

Everything else that differs is deliberate and argued above: binary over npm, a closed network
over policy-only egress, host uid over uid 1000, and the API key resolved from the overlay rather
than written into a tracked `models.json`.
