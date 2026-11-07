---
name: vendor-audit
description: Vets third-party software before it is installed or run on this machine — malicious code, the dependency graph, every network destination, the execution and permission model, and what it leaves on disk. Returns a verdict ranked by severity with each load-bearing claim re-read in the source, and records it where Sjel keeps dependency verdicts. Use when evaluating a repository, binary, CLI tool, app or agent harness for adoption, when asked to audit a dependency or check something for malicious code, supply-chain risk, telemetry or an installer, or before running an unknown install script. Do not use for auditing code this repository owns (that is tools/audit and tools/intake), for scanning a host for vulnerabilities, or for reviewing a diff for correctness.
allowed-tools: Bash, Read, Grep, Glob, WebFetch
---

# Vendor audit

Decide one question: **what code ends up executed on this machine, and can anyone but the author
change it without the maintainers noticing.**

The audit is four lenses plus one check. Each lens answers a question the others cannot.

| Lens | Question | Read |
|---|---|---|
| Supply chain | What is built, and who can alter it? | `references/supply-chain.md` |
| Egress | What leaves the machine, and to whom? | `references/egress.md` |
| Execution | What can run, and what stops it? | `references/execution.md` |
| Data at rest | What does it leave behind? | `references/data-at-rest.md` |

A complete pass on a real project, with the numbers it produced: `references/worked-example.md`.

## 1. Get the source, not the pitch

Clone it. A README audit is not an audit, and the claims in a README are the thing under test.

```bash
git clone --depth 1 <url> /tmp/audit-<name> && cd /tmp/audit-<name>
```

A shallow clone hides history: commit patterns, force-pushes, whether a lockfile drifted. Record
that limit in the report rather than working around it silently. Deepen it (`--depth 200`) only
when the verdict turns on history.

## 2. Run the four lenses

Fan out one reader per lens when the codebase is large enough that one pass would truncate. Keep
the lenses separate: a reader who is hunting for telemetry stops noticing `build.rs`.

A finding needs three things or it is not a finding: **an absolute path, a line number, and the
exact text quoted.** A summary that says "shell execution looks risky" is worth nothing; the line
that pipes a remote script to `sh` is worth everything.

## 3. Verify the load-bearing claims by reading them

An audit report that has not re-read its own headline findings is a rumour. After the lenses
return, open the five to ten claims the verdict depends on and read the lines. Expect to find
that some are wrong — a summary describes intent, the file is the outcome.

This step is not optional and it is not ceremonial. It is the difference between "an agent said
this is unsafe" and "line 1988 of `cli.rs` falls through to full access".

## 4. Answer the questions a person actually asks

Rank findings by whether they change the install decision, not by how interesting they are:

1. **Is there code that only serves the author?** Obfuscated blobs, encoded strings next to network
   calls, a fetch of code that is then executed, a base64 that decodes to a URL, logic that
   special-cases a hostname or a username. Absence of these is the finding that matters — say so
   plainly, in one line, rather than letting silence read as incomplete.
2. **What is the default, not what is available?** A careful approval broker behind a flag nobody
   passes is a project with no approval. The default is the product.
3. **Where does trust concentrate?** One account owning source, lockfile and CI means the signing
   chain only proves *that job* produced the artifact, not that the job is honest.
4. **What breaks if the author turns hostile, and what breaks if a dependency does?** Separate them.
5. **Can it be run with less?** Offline mode, a self-hosted endpoint, extensions disabled, an
   unprivileged account. An audit that ends in "do not run it" is weaker than one that ends in
   "run it this way".

## 5. Say what the audit cannot tell you

Every report ends with what was not verified, in its own list. Release assets only present on the
remote, branch protection, 2FA, runtime behaviour, anything requiring execution. A missing check
that reads as a passed check is the failure mode this skill exists to prevent.

Never execute the thing being audited to find out. Not its installer, not its build, not its tests
in a shared checkout. Building from source to read it is fine when the build is the thing being
inspected, and then it happens in a scratch clone.

## 6. Record the verdict

Two places, both existing — do not invent a third:

- **`upstreams.toml`** owns the verdict: `adopt` / `evaluate` / `watch` / `reject`, with `why` and,
  for `evaluate` or `watch`, the `revisit` condition that ends it. Two sentences. The evidence
  behind them stays in whatever wrote the findings.
- **`docs/plans/containment.md` §4** owns how a harness runs: the ring, the provider, the wrapper.
  An adopt verdict on something that executes is not finished until its row is there.

A verdict names the conditions it rests on, so a later session can tell whether it still holds.
"Adopt, provided it runs contained and with extensions off" is a verdict. "Seems fine" is not.

## Rules

- Quote, never characterise. "No telemetry" is a claim; "zero hits for `sentry|posthog|segment`
  across `crates/` and `extensions/`" is evidence.
- Report the honest strengths. A project with cosign-verified CI and dated advisory exceptions is
  a different risk from one without, and a reader deciding to install it needs both halves.
- A count beats an adjective. "223 `unsafe` in one crate" beats "some unsafe code".
- If a previous audit exists, audit the diff. A new version reopens only the lens whose surface
  changed.
