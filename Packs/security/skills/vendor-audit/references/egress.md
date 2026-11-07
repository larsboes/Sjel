# Lens 2: egress

What leaves the machine, to whom, and whether it can be stopped.

## Enumerate destinations, not intentions

```bash
grep -rEon 'https?://[a-zA-Z0-9./_-]+' --include=*.rs --include=*.ts --include=*.py \
  --include=*.go --include=*.sh . | grep -v -i 'test\|fixture\|example' | sort -u
```

Build a table. Every row gets four columns, and a row with no answer is a row to finish:

| Destination | Purpose | When it is hit | Local data carried |

"When it is hit" is the column people skip and the one that decides the risk. A host contacted on
login once is not the same as a host contacted on every launch in the background.

## Telemetry and crash reporting

```bash
grep -rin 'sentry\|posthog\|amplitude\|segment\.io\|datadog\|mixpanel\|bugsnag' .
grep -rin 'telemetry\|analytics\|consent\|opt.?out\|machine.?id\|instance.?id' --include=*.rs .
```

Absence is a finding and should be stated in one line with the terms searched. Presence is worse
than it looks when it is opt-**out**, silent, or on by default with no config switch.

Distinguish a name from a behaviour: a file called `telemetry.rs` whose own header says it writes
a local log and is installed only by an explicit caller is not telemetry leaving the machine. Read
it rather than reporting the filename.

## The self-updater

The updater is the only component that can turn a one-time install into a permanent remote-code
channel. Ask in order:

1. What URL does it fetch, and is that URL a constant or configurable by env var or config file?
   A configurable base URL is a feature for a user and an attack for anyone who can set env.
2. Does it verify a signature over the thing it executes, or a checksum, or nothing?
3. Does it execute a *script* fetched at update time, even when the binary itself is signed? If
   so, the signature protects the binary and the script is the real payload — trust collapses to
   the hosting account and its TLS.
4. Is the check on by default at every launch, and what silences it?

An updater that pipes a fetched script to `sh` is the single most common serious finding in this
lens. Report it as the RCE channel it is, and give the mitigation: install a pinned artifact and
never invoke the updater.

## Proxy and allow-list compatibility

```bash
grep -rn 'no_proxy\|NO_PROXY\|disable_proxy\|danger_accept_invalid_cert' --include=*.rs --include=*.py .
```

This decides whether the thing can be contained, so it decides the containment design. A
background fetch that deliberately ignores the proxy environment variables cannot be constrained
by an egress proxy. Whether that breaks containment depends on the boundary underneath:

- **Proxy-only containment** — the program is otherwise free to open sockets. The fetch escapes,
  and a host firewall or DNS rule is required.
- **OS-level containment** — a `(deny default)` seatbelt profile, seccomp, or a network namespace.
  Non-loopback sockets are refused before the program's proxy settings are consulted, so
  `.no_proxy()` is irrelevant. Read the profile rather than assuming it.

Say which mechanism the run depends on, because "needs a firewall rule" and "already denied" are
different amounts of work and only one of them is a finding.

## Data content

What is in the request body beyond the protocol? Search for collection of facts that are not
needed for the call:

```bash
grep -rn 'hostname(\|whoami\|user\.name\|machine.?id\|remote\.origin\|user\.email\|env::vars' --include=*.rs . \
  | grep -v 'test'
```

Hostname, username, machine ID, git remote, absolute paths outside the project, env vars. Any hit
is a finding. No hit is a sentence in the report.

## Compare claims to code

Read the project's `SECURITY.md` last, then ask whether each claim is true. A project that states
its own limits ("this is not a sandbox", "offline disables discovery, not inference") is the
easiest kind to audit, because the claims are checkable. A project that claims isolation it does
not implement is a finding in this lens, whatever the code does.
