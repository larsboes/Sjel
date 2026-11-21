# Agent sessions and execution boundaries

The [Agent capability](../../crates/agent/README.md) owns routes, sessions, streams,
approvals and terminals. The [agent plan](../plans/agent.md) links unfinished work.

## Tiers

A session starts with one immutable tier. A higher tier requires a new session.
Clients may narrow plugins, never widen them.

| Tier | Purpose | Scope |
|---|---|---|
| sjel | Capability questions and proposal actions | No shell or file writes; read-only SQL and admitted capability routes |
| project | Coding in one checkout | Files and commands within the selected session root |
| account | Work across the account | Home-directory scope; privilege escalation remains blocked |

Project/account roots must remain under the account home. A phone may start either tier, but
Ask and Block decisions still require their distinct deliberate approval actions.
The classifier is not a sandbox: account delivery requires an OS boundary or the existing
agentbox boundary.

## Policy

The shared [case table](../../Packs/security/extensions/command-policy/cases.toml) owns the
Pi/Rust baseline; the Rust classifier is `crates/sjel-agent/src/ext/policy.rs`.
The [security Pack](../../Packs/security/README.md#pi-session-permissions) owns Pi-specific
project scope, approval controls and limits.

Classification splits shell compounds, unwraps command wrappers and recursively classifies
shell strings. It checks substitutions, redirects and resolved paths; the strictest part wins.
Opaque program names are at least Ask. Unknown ordinary programs retain the baseline Allow
behavior. The selected class and rule ID enter the call log.

Allow runs without a card. Ask may receive an exact-tool/argument session grant. Block requires
a separate one-call override, never a session grant. The shared table covers destruction,
history changes, privilege, secrets, publishing and download-and-run, alongside routine reads,
local commits and builds. It is the authority for exact command spellings.

Policy evaluation measures approvals against calls and checks secrets refusal and task-minimal
cloud fields. No prompt-rate threshold exists before measurement. A small-model A6 classifier
requires evidence of fewer prompts without a missed Block.

## Terminals

Terminals are full account shells, not policy-classified agent tools. Every terminal route is
closed to agent tokens and stays behind loopback plus the signed paired-device listener.
The phone authenticates once per foreground session and clears access on background entry.
Start/end audit rows contain metadata and exit status, never input or output.

PTY ownership stays in Agent while it shares session/stream/device plumbing. Both clients use
the same remote session; an Ely local spawn is not an acceptable replacement. If the native
terminal fails, the fallback is tailnet-only SSH, tmux and a phone client with a Secure Enclave
key, not an embedded second SSH stack.

## Proposal-only unattended work

The existing capability proposal queue owns data-task proposals. The sjel tier is the default
runner. Other harnesses require verified file/shell disablement and governed sjel-mcp access.
An Ask write fails the step immediately as blocked; a headless run cannot approve itself.
A supervisor must enforce run limits before launch. No final owner records, policy changes
or evaluator changes become autonomous permissions.

Capability records remain authoritative; graph/vector indexes are rebuildable projections.
Run databases hold isolated state/cursors/caches. Results carry source, revision, observation
time, class and scope, with authorization at every expansion. C3 never enters an index.
Same-file context remains preferred over unproven cross-document graph expansion.
