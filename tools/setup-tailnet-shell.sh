#!/usr/bin/env bash
# Human-run setup. Do not invoke from an agent session: this changes Tailscale Serve config.
set -euo pipefail
TOOLS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$TOOLS_DIR/lib/paths.sh"

command -v tailscale >/dev/null 2>&1 || {
  echo "setup-tailnet-shell: tailscale CLI is not installed" >&2
  exit 1
}
SOCKET="$SJEL_OVERLAY_ROOT/secrets/sjel-status.sock"
if [ ! -S "$SOCKET" ]; then
  echo "setup-tailnet-shell: protected socket is not live: $SOCKET" >&2
  echo "Start sjel-status after provisioning inbound auth, then retry." >&2
  exit 1
fi

echo "Current Tailscale Serve configuration:"
tailscale serve status
echo
echo "This will point the default HTTPS Serve handler at the protected sjel-status Unix socket."
echo "The shell uses the socket-only operator gate; TCP capability routes require the shared token."
read -r -p "Apply this Tailscale Serve change? [y/N]: " CONFIRM
case "$CONFIRM" in [Yy]*) ;; *) echo "cancelled; no Tailscale config changed"; exit 0 ;; esac

tailscale serve --bg "unix:$SOCKET"
echo
echo "Updated Tailscale Serve configuration:"
tailscale serve status
