# Lens 3: execution

What can run, and what stops it. This lens decides the verdict more often than the other three,
because most programs are not malicious — they are permissive by default.

## The permission model, and its default

Find the code that decides whether an action is allowed, asked about, or denied. Then find the
default, which is the line that matters.

```bash
grep -rn 'unwrap_or(.*Policy\|unwrap_or_default()\|Default for .*Policy' --include=*.rs .
grep -rn 'allow_all\|auto_approve\|skip_approval\|yolo\|unsafe_mode' --include=*.rs .
```

Ask of the machinery:

- What are the modes? Is there a genuinely read-only one?
- Can a config file, an env var or a flag select a permissive mode non-interactively? A flag that
  exists for CI is also a flag that exists for an injected instruction.
- Can project-local config *loosen* the policy, or only tighten it? Tighten-only is a deliberate
  design and worth naming as a strength.
- Does the agent's own generated text ever count as consent? It must not: authorization has to
  come from a human action or a policy, never from the model.
- What happens when no human is attached — does approval deny, or proceed?

A careful broker that ships disabled is a project with **no** permission model. Say that in one
sentence before describing how good the broker is, or the reader concludes the opposite.

## Shell execution

```bash
grep -rn 'arg("-c")\|arg("-lc")\|/bin/sh\|bash -c\|shell=True' --include=*.rs --include=*.py --include=*.ts .
grep -rin 'denylist\|blocklist\|forbidden_commands\|rm -rf' --include=*.rs . | head
```

`sh -c "<one string>"` loses argument separation, so every shell metacharacter in a model-authored
command is live. That may be the intended tool — report it as the design it is, and report whether
anything else constrains it.

A blocklist of dangerous commands is not a boundary and should not be reported as one. If there is
no blocklist at all, the containment is the approval prompt — so the prompt's default becomes the
whole finding.

## Filesystem reach

Which paths can the built-in file tools touch: the project, the home directory, the whole disk?
Then check whether a flag that confines them is applied on the default path or only on the safe
path. Confinement that appears only when the user opts into safety confines nothing.

Shell tools and extensions usually ignore the file tool's path scope entirely. Check separately
rather than assuming one setting covers both.

## Extensions, plugins, adapters

For each extension mechanism, answer: what isolation does an extension actually have?

```bash
grep -rn 'seccomp\|sandbox-exec\|landlock\|namespace\|bubblewrap\|container' --include=*.rs --include=*.py .
grep -rn 'implicit_trust\|trust.*default\|auto.?enable' --include=*.rs .
grep -rn 'signature\|minisign\|ed25519\|cosign' --include=*.rs . | head
```

Three distinct claims must not be conflated:

1. **Separate process** — protects against crashes, not against intent.
2. **Capability declarations in a manifest** — consent metadata. It tells the user what it may do;
   it does not stop it doing more. Quote the project's own words if it says so; that is honesty,
   not a finding, but the reader must not mistake it for isolation.
3. **OS enforcement** — a sandbox profile, seccomp, a container, an unprivileged user. Only this
   is a boundary.

Then: is an extension verified on install, and against what? A checksum downloaded from the same
release as the archive catches corruption, not compromise. Local installs with no signature are
trust-on-first-use.

An extension that can **override built-in tools** (replacing `bash`, `read`, `write`) or that
loads third-party code written for a different harness deserves its own line at the highest
severity the lens has, because enabling it is a one-click transfer of the whole account.

## Operating-system entitlements

```bash
grep -rn 'CGEventPost\|AXUIElement\|CGPreflightScreenCapture\|Accessibility\|AVCaptureDevice\|requestAccess' .
```

Accessibility, Screen Recording, microphone and camera are granted once per app by the OS and
then held. Ask the question that decides it: **is each action confirmed per use, or does the
single TCC grant authorize everything after it?** A program with one TCC grant and no per-action
confirmation can drive any application the user can.

Also check whether a capability the README advertises actually exists in the source. A marketed
feature not present in the code is a finding about the README, and it changes how much of the rest
of it to believe.
