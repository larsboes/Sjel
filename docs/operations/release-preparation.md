# Release preparation

CLI-only preview archives are prepared independently of desktop and node delivery. The target
app delivery is a notarized Mac DMG, a native iOS app distributed through TestFlight, and a
prebuilt headless node with optional capability packages. Source installation remains
the developer path. Clean-user acceptance is tracked separately for the
[CLI preview](../tasks/cli-preview-acceptance.md) and [Mac distribution](../tasks/distribution-clean-mac.md).


## The release line

The public spine versions with SemVer, not CalVer. A tag is `vMAJOR.MINOR.PATCH` with all three
components present — `tools/release` refuses any other shape — and `tools/lib/version.sh` orders
tags with `sort -V`.

- **major** — a public contract or architectural shape changes such that an existing overlay needs
  edits to keep working.
- **minor** — a capability or operator-visible feature lands.
- **patch** — fixes only.

Judge a change against those three before opening a pull request. A contract change that ships as a
patch is what breaks an overlay on an update it was told was safe.

Deployment overlays stay untagged. An overlay is deployment state rather than a public release
line, and its commit identity stays independent of the Sjel version it runs against.

## Cutting and consuming a release

Create tags with `tools/release`. The [work-tracking contract](../architecture/work-tracking.md#releases)
defines release records, exact-source acceptance and publication receipts. The tag command also
checks for a clean checkout on `main`, a current remote base and full CI at the selected commit.
Technical notes use the same manifest delta as the consumer update preview.

A checkout moves along the line with `tools/update.sh`: fetch, fast-forward, re-run `tools/doctor`.
It never resets, rebases or force-pushes; a checkout that is both ahead and behind is left alone
with instructions rather than silently repaired.

## Getting onto the line

`bootstrap.sh` clones a release and hands off to `tools/install.sh`. The `usage` profile uses
`--depth 1 --branch <tag>`; `development` keeps full history. Fetch the bootstrap by a chosen
release tag rather than executing a moving branch. `tools/bootstrap.test.sh` exercises both
clone shapes, installer handoff, refusals and truncated downloads against a local test remote.

A usage checkout can become a development checkout without reinstalling. Restore the branch
fetch specification before deepening the tag-only clone:

~~~sh
git config remote.origin.fetch '+refs/heads/*:refs/remotes/origin/*'
git fetch --unshallow origin
~~~

Both profiles retain the source tree: tools resolve `SJEL_ROOT` from it. The bootstrap chooses
the clone shape; it does not provide prebuilt services or complete authentication and startup.

**A release tarball instead of a clone was rejected.** `tools/update.sh` updates by fetch and
fast-forward. A tarball would require a second update mechanism, including removal of files
deleted upstream. Historical clone measurements do not establish the current installation size;
use the [footprint method](index.md#installation-footprint).

Source checkouts retain Git for release identity, updates and repository metadata. The consumer
distribution target is now prebuilt apps and a minimal node, with optional capability packages;
it must not require Git or build tooling on the user's machine. `distribution.json` and the
release catalogue carry the packaged runtime's identity. Check the [CLI preview](../tasks/cli-preview-acceptance.md)
and [Mac distribution](../tasks/distribution-clean-mac.md) tasks for clean-user acceptance.

## Prepare the CLI preview

Maintainer preparation uses the distribution command:

```bash
bun tools/distribution/prepare.ts cli --version v0.0.2 --out /tmp/sjel-cli-prep --build
```

`--prepare-dependencies --target-dir DIR` can prepare production extension dependencies around
existing binaries without compiling Rust. Such output retains `source.rebuilt: false` and is not
eligible for draft publication. Dependencies use frozen lockfiles with lifecycle scripts disabled.
Without either flag, dependency-backed Pi packages are omitted and held; that mode is a packaging
fixture, not a complete coding runtime. Bash owns the bootstrap because it must run before any
Sjel binary or developer runtime is installed; policy remains in versioned helpers.

## Prepare files without installing

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
create/connect-node integration before it can serve as a fresh-user installer. Packaging an app
is not evidence that these flows exist.

Capability preparation reads its user-facing metadata from `capabilities/<name>/feature.toml` and
records the package name, exact version and platform, prerequisites, recommendation topics, data
access, asset name, byte count and SHA-256 in `release.json`'s `capabilities` array. `publish-draft.ts`
merges these entries and verifies each one against its release asset. This metadata is not an
installation approval: a client must require `friend_ready: true`, match the running runtime's
version and platform, and pass the digest to `sjel distribution install-package`. The published
catalogue shape is defined by [`release-catalogue.schema.json`](../../schemas/release-catalogue.schema.json).

The app's runtime is read-only inside the bundle. `sjel distribution place` copies it to
`~/Library/Application Support/Sjel/runtime/<version>` and creates an empty overlay at
`~/Library/Application Support/Sjel/overlay`. It never overwrites an existing file, and it starts
no service and creates no credential. The launcher uses the placed copy of the same release when
one exists. Copies of other releases stay in place; nothing removes them yet.

## Signing, publication and iOS distribution

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

Native iOS archive and TestFlight upload:

```bash
sjel testflight                       # archive, upload, wait for processing, set What to Test
sjel testflight --notes "Pair with the Mac …"
sjel testflight status                # newest build and its processing state
tools/distribution/ios-archive --version 0.1.0 --build 1 --out /tmp/sjel-ios-prep   # unsigned, local only
```

`sjel testflight` (`tools/sjel-cli/src/testflight.rs`) reads the bundle ID, team and version from
`apps/gpui/ios/host/project.yml` and signs its own App Store Connect token from the key in
`~/.appstoreconnect` (one `AuthKey_*.p8` with the App Manager role, and an `issuer_id`). It never
prints that token. The build number is `git rev-list --count HEAD`, or one past the newest upload
if that is higher, because Apple refuses a number it has already seen. The build itself is
`tools/distribution/ios-archive`. It builds `aarch64-apple-ios` and a device archive in a separate
generated Xcode project, and it never touches the Simulator project or a phone.

Two things the key cannot do, measured 2026-10-22:

1. Create the app record. Apple's API refuses `POST /v1/apps` for every role, so
   `sjel testflight app` registers the bundle ID and prints the one step for the App Store
   Connect UI. The record is named `Sjel Life`, because the store name `Sjel` is taken. The home
   screen still shows `Sjel`.
2. Sign with a cloud-managed certificate, which needs an Admin key. The export uses the Apple
   Distribution identity in the login keychain instead, with the profile `dev.sjel.ios App Store`
   that `sjel testflight profile` makes and installs.

Every build goes to the internal group `Team`, which sees all builds and needs no Beta App
Review. Its testers are added once in App Store Connect (TestFlight → Team → Testers). External
testing needs Beta App Review per build, and builds expire after 90 days; see
[Apple's TestFlight documentation](https://developer.apple.com/help/app-store-connect/test-a-beta-version/testflight-overview/).
The Simulator path is `apps/gpui/ios/run-sim`. It does not install from TestFlight.

## Optional capabilities and the later friend trial

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
does not enable a capability, start a service, change persistence or delete data. GPUI Settings can
browse the official latest-release catalog, inspect package requirements and data access, and
request an install after a separate confirmation. Installation stays unavailable until the catalog
marks a matching-runtime release `friend_ready: true`; a clean-runtime rehearsal is still required
before a public release. Updates and rollback remain acceptance work. Settings can also list
installed package receipts and request a data-preserving uninstall. Uninstall refuses enabled
dependents, payload files that differ from the receipt, and unowned files in the package directory;
the user must review and move such files before removal. It stops the service, removes persistence,
disables the capability, removes receipt-owned payload files and retains capability data outside the
package directory.

The owner rehearses on a separate clean Mac user or clean machine before involving a friend.
Allow 30–60 minutes after the release files and first-run UI are ready. Record failures with the
owning area plan and keep any screenshots or logs containing personal values private.

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
