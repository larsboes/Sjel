# Lens 4: data at rest

What the program leaves on the machine, and who else can read it.

## Where state lives

```bash
grep -rn 'home_dir()\|dirs::\|data_dir\|config_dir\|/\.config\|\.local/state' --include=*.rs --include=*.py . | head -20
```

Then list the directory on a machine where it has run, if one exists. Source tells the design;
the directory tells what it actually wrote.

## Session and history files

A coding agent writes the conversation to disk. The conversation contains the *contents* of
whatever it read, so a session file is a transcript of files, command output and `env`.

```bash
grep -rn 'sessions\|transcript\|history\|\.jsonl' --include=*.rs . | head -20
grep -rn '0o600\|0o700\|set_permissions\|write_private' --include=*.rs . | head
```

Three things to report:

- **Mode.** `0600` in a user-owned directory is correct. Anything world-readable is a finding.
- **Contents.** Prompts and tool results verbatim means a `bash: env` result puts every exported
  key into a plaintext file that is not the keychain and is not in the secret scanner's scope.
- **Retention.** Is there rotation, an expiry, a delete command? Unbounded append-only growth of
  transcripts is both a privacy and a disk problem.

## Credentials

```bash
grep -rn 'keychain\|keyring\|Security.framework\|Credential Manager\|libsecret' --include=*.rs --include=*.py --include=*.ts .
grep -rn 'credentials\|token\|api_key' --include=*.rs . | grep -i 'path\|dir\|write' | head
```

The question is one: **is the token in the OS secret store or in a file?**

A plaintext file at `0600` is readable by any process running as the same user — which is every
agent the user has ever started. That is a materially weaker position than the macOS keychain and
it is worth stating as such, even when the file mode is correct and the values are redacted in
`Debug` output and error strings. Redaction in logs is a real strength; it does not move the
token out of the file.

Also check: is the credential ever logged, included in an error message, echoed into a session
file, or passed on a command line (visible in `ps`)?

## Logs

```bash
grep -rn 'log::\|tracing::\|eprintln\|LogLevel' --include=*.rs . | grep -i 'token\|key\|secret\|credential'
```

A log line that prints a request with headers writes the bearer token to disk. Check the log level
at which the redaction happens: redaction that only applies at `debug` still leaks at `trace`.

## Summary table for the report

| Surface | Path | Mode | Contains |
|---|---|---|---|

One row per artifact. This lens produces the shortest section and the one a reader acts on most
directly — "keep this directory off a backup-synced volume" is an instruction, not an opinion.
