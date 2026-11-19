---
id: tool-rex-trial-config
title: "Decide the home of the Rex trial config"
state: backlog
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
---

# Decide the home of the Rex trial config

## Outcome

When the Rex trial ends (`upstreams.toml`, `[rex]`), its machine-local config has an owner in Sjel
or is removed with Rex.

## Acceptance

- Today `~/.config/rex/init.lua` holds five actions (project, diff, review, scratch,
  resume_agents), and `~/.config/rex/bin/` holds `rex-agent-record` and `rex-review`. None of
  these files are tracked.
- Two `SessionStart` hooks call `rex-agent-record`: one in `~/.claude/settings.json` and one in
  `~/.codex/hooks.json`. Records go to `~/.local/state/rex-agents/`.
- If Rex stays, move the config and scripts into a Pack or `tools/` with deployment and tests,
  and make the hooks deployed state.
- If Rex is rejected, remove the config, the scripts, both hooks, the state directory and
  `Packs/harness/extensions/rex-status.ts`.
