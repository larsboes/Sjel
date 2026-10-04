# tools/sjel-mcp

Sjel's capabilities as an MCP server, and the registration of that server in the agent harnesses
on this machine. Both halves are this crate.

```bash
sjel mcp                                      # the stdio MCP server itself
sjel mcp register [claude|pi]                 # write the entry, then verify it
sjel mcp unregister [claude|pi]
```

There is no `tools/sjel-mcp.ts` any more: the server was ported into `src/server.rs` on
2026-10-04 (see below).

Two harnesses, deliberately. This is a table of *measured* MCP registration paths, not a
harness registry — `tools/sjel-cli/src/harnesses/registry.rs` remains the authority on which harnesses
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

## The server is in this crate too

`src/server.rs` is the stdio server — the JSON-RPC loop, the gate-backed tool list, the approval
polling — and `src/tools.rs` is the pure half of it: which tools a capability offers under the
owner's mode, the MCP-legal name for one, and the URL one call becomes. Both were
`tools/sjel-mcp.ts` until 2026-10-04, which is the move this file's earlier version recorded as
pending ("the server migrates into this crate when it is next touched"). It moved for two
reasons: the server is a long-lived process an agent talks to, so bun was in the runtime and not
just the build, and the registration half in this crate already had to spawn and speak to it.

The port was compared against the TypeScript on this machine before the file was deleted: the
same request stream (`initialize`, `tools/list`, `ping`, a notification, an unknown method, a
malformed line, two `tools/call` — one live read and one unknown name) through both, with the
answers equal after normalizing the pseudonym tokens. Three differences are deliberate:

- The tool list is ordered by capability name. The TypeScript took `readdir` order over
  `data/agent-gates/`, which is the filesystem's and differs between machines; the set is what
  matters and the order is now the same everywhere.
- JSON object keys are sorted, because `serde_json`'s default map does that. The TypeScript wrote
  them in insertion order. An MCP client parses the JSON either way.
- A `202` with no `approval` id answers with its body rather than failing. The gate always sets
  one, so this is a capability answering `202` for some other reason; in the TypeScript that path
  read the body twice and threw, which is not worth reproducing.

The pseudonym tokens in two live answers differed between the two runs, which is the session id
doing its job: `X-Sjel-Agent-Session` is one random value per process, so the same value comes
back as the same token within a conversation and as a different one across processes.

## Verification is the point

Registering is cheap; the value is the handshake. After registering, the tool starts the server
and speaks real MCP to it — `initialize`, then `tools/list` — and prints the tool count it
answered with. That is what would have caught the original failure, where the server was built,
documented in ISA, and registered in no harness at all for a day.
