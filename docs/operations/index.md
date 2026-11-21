# Operations & host maintenance

Start with Sjel's discovery surface to find the command and owning capability:

```bash
sjel help
sjel search <task>
```

Use the active overlay and runtime output for machine identity, enabled capabilities, ports, and
health. The [generated architecture](../../ARCHITECTURE.md) describes public declarations.

## CLI-only coding installation

The `cli` delivery contains the prebuilt CLI, Claude configuration and MCP helpers, the
`coding-harness` Pack selection, and its Pi extension dependencies. It does not contain GPUI,
node services, model weights, credentials or a source checkout. Every packaged harness binds to
`coding-harness`, including Pi; the source installation's profile bindings remain unchanged.
Harness executables such as Pi or Claude Code are separate installations. Their SDK peer
dependencies are not bundled; the installed harness supplies them. Pi 1.5.0 is the tested host.
Extension runtime requirements still apply when those extensions are selected.

Built CLI archives include `THIRD_PARTY.md` and complete Rust dependency notices under
`THIRD_PARTY/rust`; JavaScript notices remain alongside the bundled modules. Each platform also
has a `sjel-source-<version>-<platform>.tar.gz` release asset containing the committed Sjel source
and the source of its compiled Rust dependencies. These source assets are separate downloads,
not required to install the prebuilt CLI.

#### Standalone installer for the next preview

Once a release containing `install.sh` is published, the standard installation uses either command:

```sh
curl -LsSf https://github.com/larsboes/Sjel/releases/latest/download/install.sh | sh
# Alternative when curl is absent:
wget -qO- https://github.com/larsboes/Sjel/releases/latest/download/install.sh | sh
```

A version-specific URL selects that release by default:

```sh
curl -LsSf https://github.com/larsboes/Sjel/releases/download/v0.0.3/install.sh | sh
```

The generated POSIX entry point embeds the existing Bash installer and the release version. It
loads the complete function body before starting installation, selects the host archive, checks
its SHA-256 and preserves the existing ownership/update rules. The retained runtime's updater
still discovers newer releases; the bootstrap's version default does not pin future updates.

Both curl and GNU wget work for archive downloads. HTTPS is required across redirects, TLS
certificate checks stay enabled, and download sizes are bounded. Missing prerequisite tools fail
before creating the installation root. The bootstrap does not install dependencies or edit shell
configuration; it prints the PATH command when the command directory is absent from PATH.

Custom directories remain explicit:

```sh
curl -LsSf https://github.com/larsboes/Sjel/releases/latest/download/install.sh |
  sh -s -- --prefix "$HOME/.local/share/sjel" --bin-dir "$HOME/.local/bin"
```

These public URLs are not live while only the `v0.0.2` draft exists. That immutable draft predates
the standalone entry point; its authenticated manual-download path remains below.

#### Existing draft and manual installation

The [CLI distribution workflow](../../.github/workflows/cli-distribution.yml) builds these
archives and rehearses the generated POSIX bootstrap, a compatible update and rollback in isolated
homes. Pushes and pull requests run the three-platform preview when a change affects the
installer payload or its build inputs. Manual dispatch always builds a preview for the exact
revision, including its corresponding source archive. The existing draft filenames are:

| Host | Archive |
| --- | --- |
| macOS ARM64, built on macOS 15 | `sjel-cli-v0.0.2-aarch64-apple-darwin.tar.gz` |
| Ubuntu 24.04-class Linux x86-64 | `sjel-cli-v0.0.2-x86_64-unknown-linux-gnu.tar.gz` |
| Ubuntu 24.04-class Linux ARM64 | `sjel-cli-v0.0.2-aarch64-unknown-linux-gnu.tar.gz` |

GNU/Linux builds target glibc, not Alpine or other musl distributions. Older distributions and
macOS Intel are not supported by these preview assets. Source compilation remains on the build
machine, not the installation machine. The workflow retains artifacts; tagging and draft creation
are separate maintainer actions.

Bootstrap installation uses a POSIX shell, Bash, standard Unix tools including tar and gzip,
and either `sha256sum` or `shasum`. Online downloads also need curl or GNU wget and CA certificates. Harness executables and their extension runtimes are separate
prerequisites; the installer does not install or authenticate them.

`v0.0.2` is a draft in [GitHub Releases](https://github.com/larsboes/Sjel/releases). Downloading it
requires authenticated maintainer access; the public release URLs do not serve draft assets.
The following example uses an authenticated GitHub CLI and selects Linux x86-64. Set `archive`
to the matching filename above for another supported host:

```bash
archive=sjel-cli-v0.0.2-x86_64-unknown-linux-gnu.tar.gz
gh release download v0.0.2 --repo larsboes/Sjel \
  --pattern "$archive" --pattern cli-install.sh --pattern SHA256SUMS \
  --dir sjel-cli-preview
cd sjel-cli-preview
digest=$(awk -v name="$archive" '$2 == name {print $1}' SHA256SUMS)
bash cli-install.sh --archive "$archive" --sha256 "$digest"
export PATH="$HOME/.local/bin:$PATH"
sjel --version
sjel harnesses list
```

Alternatively, download the matching archive, `cli-install.sh` and `SHA256SUMS` from the draft
release page into one directory, then run the commands from `digest=...` onward there. The installer
checks the selected archive against its digest; the combined release checksum file also names
other platforms, which need not be downloaded. Obtain the installer and checksum metadata from
the official authenticated release, not an unrelated mirror.

The PATH change affects the current shell only. Persist `~/.local/bin` in the shell configuration
if needed. After Pi is installed and authenticated, activate its coding resources explicitly:

```bash
sjel pack use pi coding-harness
```

This command changes the Pi profile; installation alone changes no harness settings. Other supported harnesses can be selected through `sjel harnesses use coding-harness
--harness <id>`. Review drift before synchronizing existing deployments.

Direct installation defaults to `${XDG_DATA_HOME:-~/.local/share}/sjel` and `~/.local/bin/sjel`.
`--prefix` and `--bin-dir` select other absolute directories. The installer refuses unrelated
command owners, including an existing source-install or package-manager command link. It retains
version directories and switches a stable `current` pointer, so Pi registrations remain stable.
An update replaces neither credentials nor materialized harness copies. Modified payload files
hold an update; local edits must be preserved separately before proceeding.

```bash
sjel self status
sjel self check
sjel self update
sjel self update --version v0.0.3
sjel self rollback
```

Checks report availability without installing. Online discovery requires an official published
release with `CLI_VERSION`, platform archives and `SHA256SUMS`; preparation alone does not make
that channel available. Local testing also supports `sjel self update --archive FILE --sha256
TRUSTED_DIGEST`. Rollback switches the retained runtime only; it does not undo database changes
or harness copies, delete the damaged release, or restart active agent sessions. Retained versions
consume disk space; no uninstall or retained-version pruning command is provided in this preview.

Download authenticity relies on the official GitHub HTTPS release channel and its SHA-256
metadata, not an independent publisher signature. A digest from an untrusted source is not
sufficient. Package-manager-owned commands are never self-overwritten, and developer checkouts
continue to use their existing update workflow.

<a id="release-preparation-and-later-testing"></a>

## Prepare a release

The [release preparation guide](release-preparation.md) covers maintainer workflows.

- <a id="prepare-files-without-installing"></a>[Prepare artifacts](release-preparation.md#prepare-files-without-installing)
- <a id="signing-publication-and-ios-distribution"></a>[Signing and iOS distribution](release-preparation.md#signing-publication-and-ios-distribution)
- <a id="optional-capabilities-and-the-later-friend-trial"></a>[Optional packages and clean-user trial](release-preparation.md#optional-capabilities-and-the-later-friend-trial)

## Install a shell-only node

This is a source-based operator workflow. A shell-only node serves the status and lifecycle
API; it does not yet provide a household's populated calendar, feed or travel data. Capabilities
are configuration choices within the checkout, not separately downloaded packages. The steps
below describe the current implementation; a clean-machine installation has not been rehearsed
as part of this documentation update.

### 1. Prepare the source and host tools

Use macOS or Linux with Git, Bash, a Rust toolchain and the platform compiler/linker.
[toolchain.toml](../../toolchain.toml) owns the full host-tool requirements
and platform installation hints; check its report after setup rather than assuming these four
tools satisfy every enabled capability or maintenance workflow. Containers and local models
are unnecessary for the shell-only service.

```bash
git clone https://github.com/larsboes/Sjel.git
cd Sjel
tools/install.sh
tools/toolchain-check --workflow build
```

The installer asks for the private overlay location and machine configuration. It offers
capability, harness-integration and persistence choices; skip integrations you do not need.
Missing tools are reported, not automatically installed. A non-interactive run skips capability
selection, so reaching “Setup done” does not establish that a node is running.

For a release checkout, [bootstrap.sh](../../bootstrap.sh) provides `usage` and `development`
clone profiles; see [release installation](release-preparation.md#getting-onto-the-line).

### 2. Select capabilities and provision authentication

```bash
tools/capability.sh enable sjel-status
tools/capability.sh enable store
tools/capability.sh list
```

`sjel-status` is the shell service; `store` declares the shared database's backup contract and
has no process to start. Enable resolves each capability's `requires` transitively and writes
the selected set; it does not build or start services. Add domain capabilities after the shell
works, following each owner's README for credentials, imports and data setup.

The operator runs `tools/setup-inbound-auth.sh` interactively to provision the shared inbound
credential. It explains the storage and asks before writing. Agents must not run it or handle
the value. The script uses the login Keychain plus a runtime file on macOS; on Linux the runtime
file is the credential's only copy. Configuration holds its path, never the value. Provision
this before starting the node; do not disable the gate to get through onboarding.

### 3. Build and start the status service

From the checkout, start the `sjel-status` service:

```bash
tools/service-runner.sh start sjel-status
tools/service-runner.sh status sjel-status
sjel capability url sjel-status
```

The runner builds the service binary when required. A running service keeps the binary it
started with. After a pull or an edit, `tools/service-runner.sh refresh` builds every running
process and restarts only those whose binary changed. `tools/doctor` names a process that is
older than its binary. `sjel-status` serves the status API.
The printed URL reflects the current registry instead of assuming a port.
Client applications connect through the native desktop and mobile interfaces (`apps/gpui`).

### 4. Verify persistence and the next login

```bash
tools/service-runner.sh install-persistence sjel-status
tools/service-runner.sh persistence-status sjel-status
tools/doctor
```

Persistence installation changes the host's user service configuration. See
[service persistence](persistence.md) for launchd, systemd and Linux lingering requirements.
After the next reboot and user login, repeat the status and persistence checks. A rendered unit
alone does not prove recovery after reboot. Confirm the service and its API data load; `/health`
alone does not establish that the interface works.

### 5. Disable a capability without deleting its data

For a capability being retired, remove its installed persistence, stop it, then disable it:

```bash
tools/service-runner.sh remove-persistence <capability>
tools/service-runner.sh stop <capability>
tools/capability.sh disable <capability>
```

Use the lifecycle commands only for a running service with applicable persistence; a data-only
capability has no process. Disable refuses a capability still required by an enabled dependent.
It does not erase its database rows, credentials, source, images or build artifacts. Back up and
review those separately before any deletion. Disabling `sjel-status` also removes the running
shell once it is stopped; keep it enabled while retiring optional capabilities.

## Installation footprint

Measure runtime artifacts separately from the source checkout and build caches:

```bash
du -sk target/release/sjel-status target/release/sjel-cli
sjel storage target --json
```

The first command measures allocated disk blocks for the status server and CLI at their
deployment paths. The storage command resolves the active Cargo target directory, which may
differ from the checkout's service binaries. Do not add a target directory total to the binaries
it already contains.

Measured on 2026-10-17, existing local artifacts occupied 13.77 MiB for `sjel-status` and 13.75 MiB
for `sjel-cli`: **27.52 MiB combined**. This is a core runtime artifact inventory, not an
independently installable package or a minimum disk requirement.
It excludes the source checkout, host tools and libraries, helper binaries, private configuration,
database, logs, backups and any selected domain services, models or container images. Artifact
versions may differ from the current source until rebuilt.

The normal installation retains the source and build tooling for updates and on-demand builds.
There is no validated cache-free production bundle in this measurement. Establishing one needs
a clean-host rehearsal that records initial download, peak build space, retained installation
size, authenticated browser access and restart behavior for the same selected capability set.

## Daily operations workflow

### 1. Toolchain verification

Before a build, backup, restore, or audit, check the tools required for that workflow:

```bash
tools/toolchain-check
tools/toolchain-check --workflow backup
```

### 2. Comprehensive diagnostics

```bash
tools/doctor
```

Doctor reports host configuration, enabled services, persistence, scheduled producers, store
integrity, freshness, backup receipts, and harness deployment. Its checks depend on the active
machine declaration. Diagnose each failure using the named owner; unrelated failures need
separate fixes. See the [doctor guide](doctor.md).

## Operations guides

- [Doctor diagnostics](doctor.md): report modes, check owners, and failure investigation.
- [Service persistence](persistence.md): start and stop behavior, generated host units, and image
  identity in backups.
- [Store backup contract](../../services/store/README.md#commands): live SQLite snapshots and
  isolated restore verification.

## Patch first

This section was **Pins and cooldown** until 2026-09-02, and an `upstreams.toml` entry dated
before then was decided under the hold it described. Those entries still point here; their text is
left as written, because rewriting a decision's reasoning to match a later rule falsifies it.

Take the patch. There is no adoption cooldown on host packages, Cargo, containers, rustup or
repository-wide version policy, and nothing is held at a version anywhere. Q74 removed the
cooldown on 2026-09-02; Q77, the same day, removed what was left — `upstreams.toml`'s 86 `pin`
lines, the Rust toolchain literal, the three container image versions, and
`capabilities/agentbox`'s pinned release and checksums. Q109 (2026-09-22) is the narrow
exception: Bun/npm resolution waits 24 hours in the two UI trees and on this laptop, while
Dependabot applies the same one-day window to those two manifests. The laptop-wide copy is
`~/.bunfig.toml`; `tools/install.sh` offers it opt-in and `tools/doctor` reports drift. It is an
age hold, not a version pin, and every dependency still tracks its upstream's latest after that
window.

The hold asked a release to age seven to fourteen days before adoption, on the argument that the
ecosystem finds a compromised publish in days and reading the dependency tree yourself cannot.
That argument is still true, and it was traded away deliberately. The same window that catches a
poisoned release also holds every ordinary security fix, and this deployment has one operator: a
hold that needs a human to end it is a hold that ends late. What was measured on the day of the
ruling is why the trade is defensible rather than merely chosen — Homebrew formulae sat outdated
with no updater configured at all, `openssl@3`, `gnupg`, `libgcrypt`, `nss` and `ffmpeg` among
them, while the bot that was to enforce the hold had never been installed. The hold was guarding a
door that was already open.

**What replaces it is speed and reporting, not silence.** Four things move on their own, and each
one leaves something a check can read:

- `services/host-patch` runs `tools/host-patch.sh` every 24 hours — `brew update`, `brew
  upgrade --formula`, `brew upgrade --cask --greedy`, `brew cleanup`, one `uv tool upgrade` per
  installed tool, `rustup update`, then `tools/audit`.
- `services/container-refresh` runs `tools/container-refresh.sh` every 24 hours on a host
  that runs containers: it pulls every image a `service.toml` declares and recreates the ones
  whose digest moved.
- `.github/dependabot.yml` opens the repository half daily. Cargo and github-actions carry
  `cooldown: default-days: 0`; the two Bun manifests carry `default-days: 1`, the 24-hour npm
  window restored by Q109. `.github/workflows/dependabot-automerge.yml` still arms
  `gh pr merge --auto --squash` on each pull request — so the ordinary bump merges itself once
  the required checks pass, and no bump waits for somebody to look.
- `rust-toolchain.toml` names the `stable` channel rather than a release, which is the toolchain
  `rustup update` above already installs.

Both scheduled jobs write a receipt the next `tools/doctor` reads out, because a scheduled job's
real failure is that it quietly stops running. `.github/workflows/security.yml`, CodeQL and
GitHub's advisory alerts are what now stand between a bad publish and this machine.

**The named cost.** A compromised publish reaches the npm path after one day instead of after a
seven-day hold; host packages and the other ecosystems still take the patch immediately. The two
Bun UI trees now add Socket's scanner, but it is a network-backed detector rather than a guarantee:
free mode needs no token, a Socket outage is an operational dependency, and the scanner does not
cover non-Bun paths. What stands against the remaining gap is smallness and reading — every entry
in `upstreams.toml` carries a human verdict, every resolving UI tree carries the hold and scanner,
and `bun install` never runs a lifecycle hook (`tools/check-bun-install-policy.sh`), which is the
path ChainDrop took.

**The second cost, from dropping the versions.** A merely BAD release now lands too. A broken
release of a uv tool breaks its caller the day it ships, a bad Home Assistant `:stable` is what
the house runs by the next morning, and CI can go red for a Bun release nobody here chose. That
is the trade, not an oversight: a version literal bought a delay and, on this deployment, delay
was measured to be the more expensive failure. It also never bought reproducibility — a container
tag is rebuilt under the same name, and `upstreams.toml`'s `pin` recorded an audit date while
`host-patch` moved the installed version past it every night.

**Where the version actually lives now.** Not in a manifest. The digest of a running container
(`docker inspect <name> --format '{{.Image}}'`), the label on a built agentbox image,
`Cargo.lock` and `bun.lock` for what is compiled, and `<overlay>/data/*/last.json` for what the
last scheduled run did. Every one of those describes a machine rather than an intention.
`toolchain.toml`'s `min_version` rows are the exception that proves the rule: they are FLOORS —
the oldest release Sjel's use of a tool is known to work on — and never a version to install.

**One binary, one owner.** On macOS `brew` owns `bun`, `uv`, `gitleaks` and `osv-scanner`, so
`host-patch.sh` never calls `bun upgrade` or `uv self update`. A second updater for one file is
how a `~/.local/bin` copy comes to shadow the `brew` one and answer differently — the PRD records
that happening to `yt-dlp`, which returned HTTP 403 on every media URL while `--dump-json` kept
working.

**Container images track a channel.** A `service.toml` `tag` names the publisher's rolling tag —
`:stable` for Home Assistant, `:latest` for Pi-hole, `:alpine` for Vaultwarden — and
`services/container-refresh` pulls every declared image every 24 hours, recreating the
container when the digest moves. `tools/check-service-tomls.sh` refused `latest` until 2026-09-02;
it was defending a reproducibility a tag never gave, since publishers rebuild under the same
literal. The digest is what identifies a running image.

**The Bun the workflows install is `latest`.** `oven-sh/setup-bun` is asked for `bun-version:
latest` in both workflows, so CI runs the runtime a contributor's package manager just gave them.
It was three pinned literals and `tools/check-bun-pin.sh` keeping them equal until 2026-09-02; with
no literal left anywhere — not in a workflow, and since Q77 not in `upstreams.toml` either —
there is nothing to diverge, and the class that gate caught is impossible rather than watched. The
cost is stated rather than hidden: a bad Bun release can turn CI red for a reason
unrelated to the code, which is the same trade `security.yml` makes for its scanners.

**`capabilities/agentbox` builds from the latest release.** `agentbox build` and `agentbox
host-install` resolve GitHub's `/releases/latest` and verify the tarball against THAT release's
own published `SHA256SUMS` — so the sha256 refusal survives, and the version and checksum
literals it used to compare against are gone. The self-update shim went with them: it refused
`pi update` to protect a pin, and with no pin it would refuse the outcome this section asks for.
The cooldown half, and the separate `gate` verb that was left holding only it, are gone. The
printed advisory reminder stays, because nothing automated reads advisories for a release tarball.
