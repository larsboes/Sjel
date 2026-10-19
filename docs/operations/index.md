# Operations & host maintenance

Start with Sjel's discovery surface to find the command and owning capability:

```bash
sjel help
sjel search <task>
```

Use the active overlay and runtime output for machine identity, enabled capabilities, ports, and
health. The [generated architecture](../../ARCHITECTURE.md) describes public declarations.

## Release preparation and later testing

The target delivery is a notarized Mac DMG, a native iOS app distributed through TestFlight,
and a prebuilt headless node with optional capability packages. Source installation remains
the developer path. These release tools prepare files; they do not establish that a new user
can complete setup. Track the remaining acceptance work in [the ISA](../../ISA.md#distribution).

### Prepare files without installing

The [manual distribution workflow](../../.github/workflows/distribution.yml) builds Mac, Linux
or iOS artifacts on an isolated runner and retains them for 14 days. It has no release-write
permission, signing credentials, installation step or TestFlight upload. Select the artifact,
host and version in GitHub Actions after the changes are committed. The official artifact
action preserves the archive as a download; `upstreams.toml` records its adoption.

Maintainers can prepare the same files locally. The versions and output directories below are
examples; choose the release version deliberately and use a new output directory each time.
Cold builds may take 15–60 minutes or longer. These commands build, but do not install:

```bash
bun tools/distribution/prepare.ts macos --version v0.1.0 --out /tmp/sjel-macos-prep --build
bun tools/distribution/prepare.ts node --version v0.1.0 --out /tmp/sjel-node-prep --build
bun tools/distribution/prepare.ts capability --capability calendar --version v0.1.0 --out /tmp/sjel-calendar-prep --build
```

Mac preparation needs macOS, Xcode Command Line Tools and the GPUI build dependencies. Node
and capability archives use the host architecture; they are not cross-platform binaries. Build
output is isolated from deployed services and the shared development cache. Without `--build`,
`--target-dir` selects already-built binaries instead. Packaging does not execute those binaries.

The core payload includes the shell, device and entity services, the store declaration, CLI
helpers, web assets and public runtime contracts. It omits source crates, Cargo caches,
`node_modules`, personal configuration, data and model weights. Runtime wrappers use the bundled
CLI rather than compiling it. Only tracked public reference trees and explicit build outputs
are copied; overlay files and developer configuration are not packaging inputs.

Each output includes archive checksums, `release.json` and `READINESS.txt`. The catalogue states
`friend_ready: false`. The DMG contains that note too. Its native app still needs first-run
create/connect-node integration and writable runtime placement before it can serve as a
fresh-user installer. Packaging an app is not evidence that these flows exist.

### Signing, publication and iOS distribution

For a later Mac release, provide a **Developer ID Application** identity and an existing
notarytool Keychain profile using `--identity` and `--notary-profile`. The packager checks binary
library paths, signs executable payloads and the app, submits the DMG, and staples and validates
the notarization ticket. Those flags perform signing and a network submission; leave them out
for an unsigned preparation build. Credentials stay in the owner's Keychain. Follow
[Apple's notarization workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow).

`tools/release` continues to own version tags. After a tag exists, the separate command
`bun tools/distribution/publish-draft.ts --dir OUTPUT --include OTHER_OUTPUT --confirm-draft`
combines catalogues, verifies file sizes and hashes, and creates a remote **draft** release.
`--include` is optional and repeatable. Every input must have been rebuilt from the same clean
checkout and match the release tag; prebuilt-only or dirty-worktree inventories cannot be
published. The command never publishes a public release. It uses the maintainer's existing
`gh` authentication; preparation does not require it. Do not replace an existing release's
assets with unreviewed preparation files.

Native iOS archive preparation is separate from the Tauri phone installer:

```bash
tools/distribution/ios-archive --version 0.1.0 --build 1 --out /tmp/sjel-ios-prep
```

This builds `aarch64-apple-ios` and a generic physical-device archive, using a separate generated
Xcode project. It never changes the Simulator project or installs on a phone. The default
archive is unsigned. The owner can later supply `--team TEAM_ID` to archive with configured
signing, and `--export` to produce an App Store Connect export. Xcode accounts, certificates
and provisioning must already be available; the command does not create credentials or grant
account access. No upload occurs. Use Xcode Organizer to distribute the signed archive after
reviewing it.

Before TestFlight, configure the App Store Connect app for `dev.sjel.ios`, a unique build number,
supported OS version, privacy disclosures and required-reason API manifests, encryption/export
answers, and review access through synthetic data and an accessible test node. Do not give
reviewers household credentials or private data. Validate these against the final binary;
preparation does not invent declarations. TestFlight builds expire after 90 days and external
testing may need review; see [Apple's TestFlight documentation](https://developer.apple.com/help/app-store-connect/test-a-beta-version/testflight-overview/).
`tools/phone-install` remains the Tauri development-phone path; the native Simulator path is
`apps/gpui-prototype/ios/run-sim`. Neither command is a TestFlight publisher.

### Optional capabilities and the later friend trial

The initial package is Calendar. Its archive contains `package.json`, a prebuilt executable
and a build-free `service.toml`. The [package schema](../../schemas/capability-package.schema.json)
defines the file contract. The CLI verifies the whole archive's SHA-256, host platform, bounded
file sizes, individual file hashes, paths and service command before accepting it. Archive
symlinks and extra entries are never installed: declared bytes become regular files.

Verification is separate from installation:

```bash
sjel distribution verify ARCHIVE --sha256 TRUSTED_RELEASE_DIGEST
```

Get the digest from the owner's authenticated release channel; a checksum supplied beside an
untrusted file does not establish authenticity. `install-package` is an explicit later action
and accepts only a writable packaged runtime of exactly the same version and platform. It refuses
source checkouts, app bundles, missing prerequisites and existing capability directories. It
does not enable a capability, start a service, change persistence or delete data. Upgrades,
rollback and a download/install control in the app remain acceptance work, not shipped behavior.

The owner rehearses on a separate clean Mac user or clean machine before involving a friend.
Allow 30–60 minutes after the release files and first-run UI are ready. Record failures in the
owning ISA and keep any screenshots or logs containing personal values private.

1. Download the notarized DMG through the release channel, install the app, and launch without
   Git, Cargo, Bun or Xcode. Record download, installed and peak setup size separately.
2. Create a local node or pair with a separate node through the app. Confirm credentials and
   data stay private and no terminal or source checkout is required.
3. Add Calendar, create one synthetic event, restart the app and machine, and confirm the event
   and expected services return. Disable Calendar and confirm data is retained.
4. Rehearse one compatible update and recovery to the previous runtime. Confirm database
   changes have a tested backup and do not depend on replacing a running binary in place.
5. Only then send the friend the release link and ask them to repeat the user flow without
   coaching. Test the signed native iOS app on a physical phone before inviting TestFlight users.

## Install a shell-only node

This is a source-based operator workflow. A shell-only node serves the dashboard and lifecycle
API; it does not yet provide a household's populated calendar, feed or travel data. Capabilities
are configuration choices within the checkout, not separately downloaded packages. The steps
below describe the current implementation; a clean-machine installation has not been rehearsed
as part of this documentation update.

### 1. Prepare the source and host tools

Use macOS or Linux with Git, Bash, a Rust toolchain and the platform compiler/linker. The web
build also needs Bun. [toolchain.toml](../../toolchain.toml) owns the full host-tool requirements
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
clone profiles; see [release installation](../../CONTRIBUTING.md#getting-onto-the-line).

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

### 3. Build and start the shell

From the checkout, build the static dashboard:

```bash
cd dashboard
bun install --frozen-lockfile --ignore-scripts
bun run build
cd ..
tools/service-runner.sh start sjel-status
tools/service-runner.sh status sjel-status
sjel capability url sjel-status
```

The runner builds the service binary when required. `sjel-status` serves `dashboard/dist`, so
the Vite development server is unnecessary during ordinary use. A missing web bundle leaves
the API available but pages unavailable; a failed build must be resolved before continuing.
The printed URL reflects the current registry instead of assuming a port.

For authenticated browser access on macOS, the operator installs the
[Mac companion](../../apps/mac/README.md#install) with `apps/mac/install`, then chooses
**Open Dashboard** in its menu. This requires the Swift build tools and a valid code-signing
identity. The companion exchanges its Keychain credential for a browser session without putting
the credential in the URL. Opening the bare URL does not sign the browser in.

Linux can run the node, but the Mac companion is unavailable there. An authenticated browser
entry path must be configured and verified separately using the
[listener and access contracts](../architecture/security.md). This workflow does not establish
a turnkey Linux browser login. Phone pairing and the native clients have their own setup.

### 4. Verify persistence and the next login

```bash
tools/service-runner.sh install-persistence sjel-status
tools/service-runner.sh persistence-status sjel-status
tools/doctor
```

Persistence installation changes the host's user service configuration. See
[service persistence](persistence.md) for launchd, systemd and Linux lingering requirements.
After the next reboot and user login, repeat the status and persistence checks and open the
authenticated dashboard. A rendered unit alone does not prove recovery after reboot. Confirm
the page and its API data load; `/health` alone does not establish that the interface works.

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
du -sk target/release/sjel-status target/release/sjel-cli dashboard/dist
sjel storage target --json
```

The first command measures allocated disk blocks for the shell server, CLI and static web
bundle at their deployment paths. The storage command resolves the active Cargo target
directory, which may differ from the checkout's service binaries. Do not add a target directory
total to the binaries it already contains. `dashboard/node_modules` is build input; serving the
static dashboard does not require a Vite process.

Measured on 2026-10-17, existing local artifacts occupied 13.77 MiB for `sjel-status`, 13.75 MiB
for `sjel-cli`, and 12.45 MiB for `dashboard/dist`: **39.96 MiB combined**. This is a shell
artifact inventory, not an independently installable package or a minimum disk requirement.
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
- [Store backup contract](../../capabilities/store/README.md#commands): live SQLite snapshots and
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

- `capabilities/host-patch` runs `tools/host-patch.sh` every 24 hours — `brew update`, `brew
  upgrade --formula`, `brew upgrade --cask --greedy`, `brew cleanup`, one `uv tool upgrade` per
  installed tool, `rustup update`, then `tools/audit`.
- `capabilities/container-refresh` runs `tools/container-refresh.sh` every 24 hours on a host
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
`graphify` breaks `tools/graphify.sh` the day it ships, a bad Home Assistant `:stable` is what
the house runs by the next morning, and CI can go red for a Bun release nobody here chose. That
is the trade, not an oversight: a version literal bought a delay and, on this deployment, delay
was measured to be the more expensive failure. It also never bought reproducibility — a container
tag is rebuilt under the same name, and `upstreams.toml`'s `pin` recorded an audit date while
`host-patch` moved the installed version past it every night.

**Where the version actually lives now.** Not in a manifest. The digest of a running container
(`docker inspect <name> --format '{{.Image}}'`, ISA.md C4), the label on a built agentbox image,
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
`capabilities/container-refresh` pulls every declared image every 24 hours, recreating the
container when the digest moves. `tools/check-service-tomls.sh` refused `latest` until 2026-09-02;
it was defending a reproducibility a tag never gave, since publishers rebuild under the same
literal. The digest is what identifies a running image (ISA.md C4).

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
