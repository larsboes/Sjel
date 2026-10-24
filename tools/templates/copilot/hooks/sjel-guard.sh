#!/bin/bash
# sjel-guard.sh — Sjel's preToolUse hook for GitHub Copilot CLI.
#
# Copilot keeps no tool-deny setting: `copilot help config` offers permissions, sandbox and
# allow-lists, and `--deny-tool` is a session flag that dies with the session. Earlier Sjel solved
# that with a launcher (tools/copilot) passing three --deny-tool flags, which meant `copilot` typed
# by hand ran with none of them.
#
# A command preToolUse hook is the persistent form: Copilot loads ~/.copilot/hooks/*.json on every
# session, and a command hook is fail-closed — a crash or non-zero exit denies the tool call.
#
# Contract: read Copilot's hook payload on stdin, write at most one decision object on stdout.
#   {"permissionDecision": "deny", "permissionDecisionReason": "..."}   deny
#   (nothing, exit 0)                                                   no opinion, normal flow
#
# What it matches, and what it deliberately does not:
#   matched    rm, git commit, git push — as the first word of a shell segment, with any leading
#              env assignments and wrappers (sudo, command, env, nice, nohup, time) skipped, and
#              /bin/rm or /usr/bin/git recognised by their basename.
#   not matched  a blocked command reached through another program: `xargs rm`, `find -delete`,
#              `bash -c 'git push'`, or a git alias. This is a guard against the agent reaching
#              for the obvious command, not a sandbox. The sandbox is sandbox.* in
#              settings.json; the non-bypassable layer is a root-owned hook in
#              /etc/github-copilot/policy.d/.
#
# bash 3.2-safe, and no jq: a hook runs on every tool call, and this must not depend on what the
# sandbox happens to put on PATH (CONTRIBUTING.md#portable-shell).

set -u

# A terminal stdin means somebody ran this by hand to read the text. Print the rules, not a hang.
if [ -t 0 ]; then
  echo "sjel-guard: blocks rm, git commit and git push in Copilot sessions" >&2
  echo "usage: echo '{\"toolName\":\"bash\",\"toolArgs\":{\"command\":\"rm -rf x\"}}' | $0" >&2
  exit 0
fi

payload=$(cat)

# toolName is camelCase for a camelCase hook event and tool_name for the PascalCase (Claude-format)
# one. Both are read, because the guard is installed under the camelCase name today but a
# VS Code-format config copied next to it must not silently disarm it.
tool=$(printf '%s' "$payload" \
  | sed -nE 's/.*"tool(Name|_name)"[[:space:]]*:[[:space:]]*"([^"]*)".*/\2/p' \
  | head -1)

case "$tool" in
  bash | powershell | shell | Bash | PowerShell | BashTool) ;;
  *) exit 0 ;;
esac

# Everything after the command key, not matching-to-quote: an escaped quote inside the command
# would otherwise end the match early and hide `echo "x" && git push` from the scan.
rest=$(printf '%s' "$payload" | sed -nE 's/.*"command"[[:space:]]*:[[:space:]]*//p')
[ -n "$rest" ] || exit 0

# Drop the opening quote, turn escaped quotes into single quotes, then end at the first quote
# that is left. Nothing is ever evaluated, so swapping the quoting character cannot inject
# anything — and an escaped quote no longer hides the rest of the command from the scan.
cmd=$(printf '%s' "$rest" | sed -e 's/^[[:space:]]*"//' -e 's/\\"/'"'"'/g' -e 's/".*//')

blocked=$(printf '%s' "$cmd" | awk '
{
  n = split($0, segs, /[;&|()`]+/)
  for (i = 1; i <= n; i++) {
    m = split(segs[i], w, /[ \t]+/)
    j = 1
    while (j <= m) {
      t = w[j]
      if (t == "") { j++; continue }                                 # a segment that starts after a space splits to one
      if (t ~ /^[A-Za-z_][A-Za-z0-9_]*=/) { j++; continue }          # FOO=bar rm -rf x
      if (t == "sudo" || t == "command" || t == "env" || t == "nice" || t == "nohup" || t == "time") { j++; continue }
      # A wrapper flag that takes a value: `sudo -u root rm`, `nice -n 10 rm`, `env -i rm`.
      if (t ~ /^-(u|g|p|C|U|T|D|n|o|f|r|t|h|S|i|c)$/) { j += 2; continue }
      if (t ~ /^-/) { j++; continue }                                 # sudo -E rm, --non-interactive
      break
    }
    if (j > m) continue
    head = w[j]
    if (head == "") continue
    sub(/^.*\//, "", head)                                            # /bin/rm -> rm
    if (head == "rm") { print "rm"; exit }
    if (head == "git") {
      k = j + 1
      while (k <= m) {
        t = w[k]
        if (t == "-C" || t == "-c" || t == "--git-dir" || t == "--work-tree" || t == "--namespace") { k += 2; continue }
        if (t ~ /^-/) { k++; continue }
        if (t == "commit" || t == "push") { print "git " t; exit }
        break                                                          # git status, git diff: allowed
      }
    }
  }
}')

[ -n "$blocked" ] || exit 0

printf '{"permissionDecision":"deny","permissionDecisionReason":"Sjel guard: `%s` is blocked in Copilot sessions. The blocks live in ~/.copilot/hooks/sjel-guard.sh, owned by `sjel copilot apply`; the owner can change the list there."}\n' \
  "$blocked"
exit 0
