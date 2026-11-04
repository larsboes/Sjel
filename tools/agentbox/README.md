# agentbox

A coding agent in a disposable container, talking to a model server on the host. One project directory in, nothing left behind on exit.

## Surface

The `agentbox` CLI (`tools/agentbox/agentbox`) manages image builds, container lifecycle, and host installations:

- `agentbox build` — fetch the latest agent release archive, verify sha256 checksums, and build the container image.
- `agentbox run [--online] [--project <path>]` — launch disposable container running the coding agent.
- `agentbox shell` — start an interactive shell in a disposable container without the agent.
- `agentbox doctor` — probe container-to-host connectivity and model endpoint reachability.
- `agentbox host-install` — verify and install the standalone agent binary on the host.

## Why this shape: Disposable container isolation over host agent execution

Running a coding agent directly on a developer workstation grants the agent process unconstrained access to SSH keys, cloud tokens, shell history, and adjacent repositories.

A disposable container (`--rm`) created with non-root UID/GID mapping ensures that changes are confined strictly to the mounted project directory (`/workspace`). Container teardown removes writeable layers immediately on exit.

The agent's configuration is mounted into `/home/agent/config` from an ephemeral directory. The host's personal configuration is never mounted into the container.

## Why this shape: Host-bound native inference with containerized tool execution

Inference engines for local models (such as MLX on Apple Silicon) rely on Metal and Neural Engine acceleration, which cannot run inside a Linux container VM.

Placing the model server natively on the host while confining the agent and its tools to the container preserves hardware acceleration while isolating code execution.

The model endpoint is accessed across the container bridge network (`host.docker.internal`). Endpoints bound to external interfaces enforce API authentication keys stored in overlay configuration.

## Why this shape: Explicit network modes over heuristic firewalling

Agent execution provides two network states:

- Default (`--network none`): Loopback only, no external routes, and no DNS resolution. Untrusted checkouts or local tests execute in isolation.
- Online (`--online`): Bridge network access reaching `host.docker.internal` for model calls and external internet endpoints for package operations.

Intermediate internal networks without host routing or complex in-container firewall rules are rejected in favor of an explicit invocation flag.

## Why this shape: Release archive digests over package managers

Agent binaries are downloaded as standalone release tarballs verified against published cryptographic checksums.

Building the container image (`tools/agentbox/Containerfile`) directly from release archives avoids installing Node, npm, or runtime package managers, satisfying repository language tooling doctrine.

## Verification

```bash
tools/agentbox.test.sh
tools/agentbox/agentbox doctor
tools/agentbox/agentbox run --online --project . -- -p "status"
```

## Considered and declined

- **Host-resident agent with VM tools (Gondolin)**: Declined. Retaining the agent, extensions, and credentials on the host while routing only tool execution into a VM compromises the primary isolation boundary.
- **Complex sandbox gateways (NVIDIA OpenShell)**: Declined. Multi-service policy gateways introduce operational overhead and background daemons compared to standard container execution.
- **npm-installed container agent (`node:22`)**: Declined. Standalone release binaries eliminate Node runtime overhead and third-party package manager dependencies.
- **Internal networks with relay containers**: Declined. Running auxiliary forwarding containers introduces orphaned process risks and unmanaged network bridges.
- **Podman**: Declined. Incompatible network resolution semantics (`host.containers.internal`) and unverified isolation behavior on macOS hosts.
