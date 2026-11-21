# Signed Sjel background app

Sjel's macOS jobs whose service manifest declares `full_disk_access = "true"` run through
a separate signed app. System Settings displays it as **Sjel** with the Sjel icon. Neither
Sjel Desktop nor the menu-bar app needs its own Full Disk Access grant. The background app
does not have to be open for scheduled jobs to run.

The background app is installed at `~/Library/Application Support/Sjel/helpers/Sjel.app`.
It is distinct from the menu-bar app at `~/Applications/Sjel.app`. A sibling receipt identifies
the build inputs and installed executable. The installer reuses the menu-bar app's icon.

## Set up or migrate

1. Run `sjel fda-launcher install` to build, sign, and install the background app.
2. Open **Settings → macOS permissions** in Sjel Desktop or **⋯ → macOS Permissions…** from
   the menu-bar app. Drag **Sjel.app** into **Full Disk Access** and switch on **Sjel**.
   If the list rejects the drop, use **Show in Finder** and add the revealed bundle using **+**.
3. From the logged-in desktop session, run `sjel fda-launcher migrate`. It verifies access
   from a disposable launchd job before switching the declared jobs. Only after it succeeds,
   remove the previous `sjel-fda-launcher` entry in System Settings.

| Command | Effect |
|---|---|
| `sjel fda-launcher status` | Check the bundle signature, build inputs, and receipt. Does not test access. |
| `sjel fda-launcher install` | Install the signed bundle. A current installation is untouched. Does not change routing or grants. |
| `sjel fda-launcher verify` | Probe access from launchd. Reports status without listing protected files. |
| `sjel fda-launcher migrate` | Install, verify access, switch jobs, and record the selected helper. |
| `sjel fda-launcher path` | Print the executable that persistence currently uses. |
| `sjel fda-launcher app-path` | Print the bundle to add in System Settings. |

Install requires a stable code-signing identity and refuses unowned, symlinked, or modified
installations. Source changes during a build abort publication. The installer signs and checks
the candidate before replacing the bundle.

The live probe runs outside Terminal's responsibility chain. An access denial, a missing
protected directory, and failure to start the launchd probe are distinct results.
An unsuccessful migration restores changed jobs, preserves the previous helper, and leaves
future job installs on the previous route. After migration, the prior flat executable
remains available for recovery; the obsolete overlay executable is removed only after
installed and loaded jobs stop using it. No command edits macOS privacy settings.

## Checks

`tools/fda-launcher.test.sh` tests installation, signing, migration gating and rollback in an
isolated home. Live acceptance must confirm the icon and name in System Settings and run the
launchd probe on macOS.
