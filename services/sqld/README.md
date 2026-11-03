# sqld — the libSQL primary

The database server other devices hold an [embedded replica](../../libs/sjel-turso/README.md) of.
This machine is the primary; a phone is the first replica, and a home server takes the primary role
later without changing anything here except one setting.

There is no code in this directory. `sqld` is Turso's published release, adopted as-is and pinned
in [`tools/install-sqld.sh`](../../tools/install-sqld.sh); this manifest is what makes it a service
this machine owns rather than something started by hand.

## Why a server at all

An embedded replica reads locally and forwards writes to a primary. With one machine and no primary,
that is a network round trip to nowhere — so the primary is the part that had to exist first. It is
also what makes the phone possible: the phone replicates from here, and its reads keep working when
this machine is asleep.

## Install

```console
$ tools/install-sqld.sh          # pinned 0.24.32, digest verified, into ~/.local/bin
$ tools/install-sqld.sh --check  # what is installed, and whether it is the pinned one
```

One version, one digest, one platform per line, each measured. The digest is a literal in the
script on purpose: a checksum fetched from beside the archive it describes proves only that the
transfer was intact. Bumping either is a two-step decision — read the release page, then watch the
install fail if you mistyped.

`sqld` 0.24.32 is from 2025-02-14 and upstream has been quiet since. It syncs correctly with the
`libsql 0.10.0` crate this repository links, which is the compatibility fact that matters and the
reason this does not chase a newer tag.

## Configure

Two machine facts, both in the deployment record rather than here, because this file is tracked:

| setting | meaning |
| --- | --- |
| `SQLD_DB_PATH` | the primary's data directory, under the overlay |
| `SQLD_AUTH_JWT_KEY_FILE` | the file holding the **public** half of the signing key |

`sqld` reads both from its environment, and `tools/service-runner.sh` passes the overlay's
deployment settings into every process it starts. The private half of that key pair lives in the
macOS Keychain; see [`libs/libsql-auth`](../../libs/libsql-auth/README.md).

`sjel setup apply turso --host-here --confirm` does the whole job in one command: it installs the
binary if it is missing, mints a credential from the Keychain key, writes the public half and the
two settings, enables this capability, starts it, and verifies against it.

## Run

```console
$ tools/service-runner.sh start sqld
$ tools/service-runner.sh status sqld
$ tools/service-runner.sh stop sqld
```

Port `8089` is the HTTP/Hrana port a client replicates from, bound to loopback. Port `8105` is
gRPC, for the day this machine is a *replica* of a home server rather than the primary. Neither is
reachable from another device yet: exposing `8089` to the tailnet is a separate decision, and
sqld's JWT check is the control that makes it defensible.

## Backup

The database is `<SQLD_DB_PATH>/dbs/default/data`. The manifest declares it as
`backup_sqlite_online`, so `tools/backup.sh sqld` copies it open with `sqlite3 .backup` instead of
holding the primary down — which for a primary would mean every follower's writes failing.

## Upstream

[`upstreams.toml`](../../upstreams.toml) `[libsql-server]`: adoption verdict, license, and why this
release rather than another. The client side of the same project is `[libsql]`.
