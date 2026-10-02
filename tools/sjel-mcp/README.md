# tools/sjel-mcp

Registers Sjel's MCP server with the agent harnesses on this machine, and verifies each
registration by speaking MCP to the server it just registered.

```bash
tools/sjel-mcp/sjel-mcp register              # pi and Claude Code
tools/sjel-mcp/sjel-mcp register claude       # one of them
tools/sjel-mcp/sjel-mcp unregister pi
```

Two harnesses, deliberately. This is a table of *measured* MCP registration paths, not a
harness registry — `tools/lib/harness-registry.ts` remains the authority on which harnesses
exist and which are installed. Codex supports MCP and is absent here on purpose: `~/.codex`
exists on this machine while no `codex` binary does, so a writer for it could not be verified
even once, and that registry states the rule (from the day three Packs were deployed for a
Codex that was not installed) that a row moves in only when the format has been verified.

| Harness | How it is registered |
| --- | --- |
| pi | Written into `~/.pi/agent/mcp.json` directly |
| Claude Code | By driving `claude mcp add sjel --scope user`, then verified |

**pi is written directly rather than driven through its CLI, and the measurement is the
reason.** `pi mcp add` rejects `--timeout` ("Unknown option"), and that per-request timeout is
load-bearing: the server polls for up to 120 s while the owner decides an ask-mode write, and
pi's default is 60 s. `tools/agent-integrations.sh` drives an upstream's own installer where it
can; here it cannot express a field that matters.

## The server itself is still TypeScript

`tools/sjel-mcp.ts` is the stdio server — the JSON-RPC implementation, the gate-backed tool
list, the approval polling. It is unchanged and still runs as `sjel mcp`. This crate is the
registration and verification half, which is new code and therefore Rust
(`on-dependencies-and-build.md`). The server migrates into this crate when it is next touched.

## Verification is the point

Registering is cheap; the value is the handshake. After registering, the tool starts the server
and speaks real MCP to it — `initialize`, then `tools/list` — and prints the tool count it
answered with. That is what would have caught the original failure, where the server was built,
documented in ISA, and registered in no harness at all for a day.
