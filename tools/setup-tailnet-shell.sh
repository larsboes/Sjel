#!/usr/bin/env bash
# Human-run setup. Do not invoke from an agent session: this changes Tailscale config and logs a
# node in.
#
# On macOS the Tailscale app cannot serve the shell: its network extension is sandboxed and gets
# `connect: operation not permitted` on every user-owned socket path tried (the overlay and
# /Library/Tailscale/, 2026-10-20). So macOS runs a second, user-level tailscaled in userspace
# networking mode, named `sjel`, that serves only the shell. The app stays untouched, and the
# daemon runs as the user; only the one Serve call below uses sudo. The daemon is Homebrew's
# `tailscale` formula, unlinked so its CLI does not shadow
# the app's (`brew install tailscale && brew unlink tailscale`).
set -euo pipefail
TOOLS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$TOOLS_DIR/lib/paths.sh"

SOCKET="$SJEL_OVERLAY_ROOT/secrets/sjel-status.sock"
if [ ! -S "$SOCKET" ]; then
  echo "setup-tailnet-shell: protected socket is not live: $SOCKET" >&2
  echo "Start sjel-status after provisioning inbound auth, then retry." >&2
  exit 1
fi

TS=(tailscale)
if [ "$(uname -s)" = "Darwin" ]; then
  BIN="$(brew --prefix tailscale 2>/dev/null)/bin"
  [ -x "$BIN/tailscaled" ] || {
    echo "setup-tailnet-shell: run 'brew install tailscale && brew unlink tailscale' first" >&2
    exit 1
  }
  # The node key is a credential, so the node's state lives beside the shell's socket.
  STATE="$SJEL_OVERLAY_ROOT/secrets/tailscale-sjel"
  DAEMON_SOCKET="$STATE/tailscaled.sock"
  LABEL="com.sjel.tailscaled"
  PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
  mkdir -p "$STATE" "$HOME/Library/LaunchAgents"
  chmod 700 "$STATE"
  # --port=0 picks a free UDP port, so this node never competes with the app's.
  cat > "$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>$LABEL</string>
<key>ProgramArguments</key><array>
<string>$BIN/tailscaled</string>
<string>--tun=userspace-networking</string>
<string>--statedir=$STATE</string>
<string>--socket=$DAEMON_SOCKET</string>
<string>--port=0</string>
</array>
<key>RunAtLoad</key><true/>
<key>KeepAlive</key><true/>
<key>StandardErrorPath</key><string>$STATE/tailscaled.log</string>
</dict></plist>
EOF
  launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
  launchctl bootstrap "gui/$(id -u)" "$PLIST"
  for _ in 1 2 3 4 5 6 7 8 9 10; do [ -S "$DAEMON_SOCKET" ] && break; sleep 1; done
  TS=("$BIN/tailscale" "--socket=$DAEMON_SOCKET")
  echo "Logging in the 'sjel' node. Open the URL it prints, if it prints one."
  "${TS[@]}" up --hostname=sjel
  # With Tailnet Lock on, a signing node must sign this one. `lock status` prints the command.
  "${TS[@]}" lock status | grep -i -A3 "locked out" || true
fi
# A Unix-socket target needs root for this one call (tailscale ipn/ipnserver/actor.go,
# connIsLocalAdmin). The config persists in the node's state; the daemon stays the user's.
SERVE=("${TS[@]}")
[ "$(uname -s)" = "Darwin" ] && SERVE=(sudo "${TS[@]}")

command -v "${TS[0]}" >/dev/null 2>&1 || {
  echo "setup-tailnet-shell: tailscale CLI is not installed" >&2
  exit 1
}
echo "Current Tailscale Serve configuration:"
"${TS[@]}" serve status
echo
echo "This will point the default HTTPS Serve handler at the protected sjel-status Unix socket."
echo "The shell uses the socket-only operator gate; TCP capability routes require the shared token."
read -r -p "Apply this Tailscale Serve change? [y/N]: " CONFIRM
case "$CONFIRM" in [Yy]*) ;; *) echo "cancelled; no Tailscale config changed"; exit 0 ;; esac

"${SERVE[@]}" serve --bg "unix:$SOCKET"
echo
echo "Updated Tailscale Serve configuration:"
"${TS[@]}" serve status
