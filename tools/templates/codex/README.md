# Codex command access

`config.base.toml` is an opt-in permissions baseline, not an installer. Replace `@HOME@`
with the absolute home directory, merge its root keys into the user config, and add its
`permissions.sjel-auto` and `shell_environment_policy` tables. Preserve unrelated operator keys. Back up the user config
before changing it. Do not combine `default_permissions` with `sandbox_mode` or
`sandbox_workspace_write`. Copy `security.rules` to a separate user rules file, leaving
existing command approvals alone. Restart Codex to load the result.

Ordinary commands receive filesystem write access and direct network access without
sandbox escalation prompts. OS permissions still apply. Secret-file paths are denied by
the filesystem sandbox, including home-directory credentials and workspace dotenv/private
key files. Shell subprocesses also exclude variable names containing KEY, SECRET, TOKEN,
PASSWORD, PASSWD, or CREDENTIAL, and the Bitwarden session variable. The deliberate
tradeoff is broad access to everything else, not isolation to
the current project. Secret denials also prevent sourcing those files; this is not Pi's
`vault_exec` tool or its output sanitizer. Unrecognized secret variable names, shell
startup files that reintroduce credentials, keychains, tool output, MCP servers, and apps
are not protected by these filesystem rules and environment filters.

The granular approval policy leaves command-rule prompts interactive and prevents full
sandbox escalation from bypassing secret denials. MCP elicitation and skill approvals
remain interactive; disabling those categories rejects requests rather than accepting
them. Do not set `approval_policy = "never"` or use the bypass flag: neither provides the
requested deletion approval workflow.

`security.rules` asks about every direct `rm`, including non-recursive invocations and
the listed absolute paths and simple wrappers. The broader match avoids an incomplete
list of recursive/force flag permutations. It does not recognize arbitrary wrapper
options, variable-expanded executables, scripts, `find -delete`, language-level deletion,
or remote deletions. These are command-prefix rules, not a general deletion firewall.
Do not claim that they prevent all harmful file removal. The agent must still request
confirmation for destructive operations it recognizes outside these prefixes.

Validate rules without running the target command:

```bash
codex execpolicy check --rules tools/templates/codex/security.rules -- rm -rf build
codex execpolicy check --rules tools/templates/codex/security.rules -- cargo test
codex doctor --summary
```

Test filesystem denials only with synthetic files, never real credentials. Verify the
live approval flow in a fresh interactive session before relying on it. A profile or
app-selected permission preset can override user defaults, and user-owned configuration
is not a tamper-proof managed security policy.

References: [permission profiles and approval policy](https://learn.chatgpt.com/docs/config-file/config-reference),
[command rules](https://learn.chatgpt.com/docs/agent-configuration/rules).
