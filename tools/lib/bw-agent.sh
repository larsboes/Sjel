# Resolve the Bitwarden desktop app's SSH agent socket into SSH_AUTH_SOCK, if
# present. Single source for WHERE that socket lives — sourced by both the shell
# config (tools/shell/init.zsh) and any tool that needs the vault agent
# (tools/backup.sh). Guarded: a no-op when the app isn't running / not installed,
# so it never points SSH_AUTH_SOCK at a dead socket. POSIX-ish on purpose so
# zsh and bash can both source it. bash 3.2-safe.
#
# Candidate order covers each platform's Bitwarden desktop packaging: the macOS
# app sandbox container, the Linux snap and flatpak sandboxes, then the plain
# ~/.bitwarden-ssh-agent.sock the .deb/.rpm and AppImage builds use. On WSL there
# is usually no Bitwarden desktop at all, so this stays a clean no-op there.
for _bw_sock in \
  "$HOME/Library/Containers/com.bitwarden.desktop/Data/.bitwarden-ssh-agent.sock" \
  "$HOME/snap/bitwarden/current/.bitwarden-ssh-agent.sock" \
  "$HOME/.var/app/com.bitwarden.desktop/data/.bitwarden-ssh-agent.sock" \
  "$HOME/.bitwarden-ssh-agent.sock"; do
  if [ -S "$_bw_sock" ]; then export SSH_AUTH_SOCK="$_bw_sock"; break; fi
done
unset _bw_sock
