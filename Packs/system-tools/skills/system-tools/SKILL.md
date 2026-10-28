---
name: system-tools
description: "Guides discovery and safe use of system-level CLI tools for package management, processes, networking, and administration. Use when an operating-system command is needed or its options are unfamiliar. Do not use for project-specific workflows; follow that project's own documentation."
effort: low
---

# System Tools

System-level CLI tooling for shell operations and administration.

## Discovery Pattern

For any tool, use these in order:

1. **`--help` or `-h`** — Quick flag reference
2. **`man <command>`** — Full manual (if available)
3. **`tldr <command>`** — Common examples (install via `npm install -g tldr`)
4. **`<command> <subcommand> --help`** — Specific subcommand help

## Core System Tools

### Package Management

| Tool | Purpose | Discover |
|------|---------|----------|
| `apt` / `apt-get` | Debian/Ubuntu packages | `apt --help` |
| `dpkg` | Package inspection | `dpkg --help` |
| `snap` | Snap packages | `snap --help` |
| `pip` / `pip3` | Python packages | `pip --help` |
| `npm` | Node packages | `npm --help` |

### Process & System

| Tool | Purpose | Discover |
|------|---------|----------|
| `ps` | Process status | `ps --help` or `man ps` |
| `top` / `htop` | Process monitor | `top` (interactive), `htop --help` |
| `kill` / `killall` | Signal processes | `kill -l`, `killall --help` |
| `systemctl` | systemd services | `systemctl --help` |
| `journalctl` | System logs | `journalctl --help` |
| `df` / `du` | Disk usage | `df --help`, `du --help` |
| `free` | Memory usage | `free --help` |
| `uptime` | System uptime | `uptime --help` |
| `uname` | System info | `uname --help` |

### Networking

| Tool | Purpose | Discover |
|------|---------|----------|
| `curl` | HTTP client | `curl --help` |
| `wget` | File download | `wget --help` |
| `ping` | Connectivity test | `ping --help` |
| `netstat` / `ss` | Socket stats | `ss --help` |
| `nc` / `netcat` | Network swiss army knife | `nc -h`, `man nc` |
| `dig` / `nslookup` | DNS lookup | `dig -h`, `nslookup` |
| `ip` | Network config | `ip --help` |
| `iptables` / `nft` | Firewall rules | `iptables --help`, `nft --help` |
| `ssh` | Remote shell | `ssh`, `man ssh` |
| `scp` / `rsync` | File transfer | `scp`, `rsync --help` |

### File & Text

| Tool | Purpose | Discover |
|------|---------|----------|
| `find` | File search | `find --help` |
| `locate` / `mlocate` | Fast file search | `locate --help` |
| `grep` / `rg` (ripgrep) | Text search | `grep --help`, `rg --help` |
| `sed` | Stream editor | `man sed` |
| `awk` | Text processing | `man awk` |
| `jq` | JSON processing | `jq --help` |
| `yq` | YAML processing | `yq --help` |
| `tar` / `zip` | Archiving | `tar --help`, `zip --help` |
| `fd` | User-friendly find | `fd --help` |
| `fzf` | Fuzzy finder | `fzf --help` |

### Development

| Tool | Purpose | Discover |
|------|---------|----------|
| `git` | Version control | `git --help`, `git <cmd> --help` |
| `docker` / `podman` | Containers | `docker --help`, `podman --help` |
| `make` | Build automation | `make --help` |
| `gcc` / `clang` | C compiler | `gcc --help` |
| `python` / `python3` | Python interpreter | `python3 --help` |
| `node` | Node.js runtime | `node --help` |
| `cargo` | Rust package manager | `cargo --help` |

### Monitoring & Debugging

| Tool | Purpose | Discover |
|------|---------|----------|
| `strace` | System call trace | `man strace` |
| `ltrace` | Library call trace | `man ltrace` |
| `lsof` | Open files | `lsof -h` |
| `tcpdump` | Packet capture | `man tcpdump` |
| `vmstat` / `iostat` | System stats | `vmstat --help`, `iostat --help` |
| `dmesg` | Kernel messages | `dmesg --help` |

## Check Local Availability

Check whether a command exists before using it. Prefer `command -v` for scripts on `PATH`; use
`--version` only when the command supports it. Do not assume a custom script path or disclose
credential values while checking authentication.

```bash
command -v docker podman gcloud aws az
```

## Errors and safety

If a command is unavailable or its help is missing, state that and do not guess its flags from a
similar tool. Before a destructive or system-wide change, identify the target and rollback path;
confirm before an irreversible action.

## Usage Pattern

When a task needs a CLI tool:

1. **Identify the domain** (package install, network, process, etc.)
2. **Check if tool exists** (`which <tool>` or `command -v <tool>`)
3. **Discover capabilities** (`<tool> --help | head -50`)
4. **Use progressively** — start simple, add flags as needed

## Examples

```bash
# Find how to install something
apt --help | grep install

# Check curl request options
curl --help | grep -E "(header|data|request)"

# See all git subcommands
git --help | grep -A 100 "These are common Git commands"

# Learn about tar extraction
tar --help | grep extract
```
