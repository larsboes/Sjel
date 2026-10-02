# Operate Sjel

Resolve the capability first with `tools/axon-context with <capability>`. Read its current
contract before using an unfamiliar route.

## Common operations

```bash
sjel capability ingest <url>
sjel capability feed [days]
sjel capability call <capability> get <path> [curl-args...]
sjel capability call <capability> post <path> '<json>' [curl-args...]
```

Use `sjel capability url <capability>` plus `curl` when the generic wrapper does not express the
contract. Prefer read-only requests for orientation.

Before a write:

1. Confirm the target capability owns the data.
2. Check validation, provenance, idempotency, and retry behavior in the contract.
3. Show or verify the exact payload when the change is consequential.
4. Re-read the created or changed record when the API supports it.

Never route around a capability API by editing its database or private files directly. Read
`references/shared-data-boundaries.md` for personal, vault, or cross-capability data.

## Through MCP tools

When the session exposes Sjel's MCP server, prefer it to the shell for capability calls: it
carries the agent token itself, returns the same contracts, and needs no `sjel capability call`
wrapping. The tool list is built from each capability's `GET /routes`, so discover it rather
than memorize it. In Pi the server is `sjel` and a tool is `mcp__sjel__<capability>__<method>_<path>`.

```js
const ns = await describeNamespace("mcp__sjel");   // codemode
ns.tools.forEach((t) => text(t.name));
```

Answers are pseudonymized (F9): one identity is one token such as `<SENDER_k3x9qa>`, every `c3`
object is removed, and a token passed back unchanged acts on that value in the same capability.
A write under `ask` mode waits up to 120 s for Allow in the menu-bar app; the server polls while
it is undecided and tells you when the wait expired.

**Never return a raw capability payload to the model.** `GET /triage` answered 384 rows in
589 KB (about 147k tokens) on 2026-10-02 and ignored a `limit` query parameter, so a direct call
is unusable. Read in a script, reduce, and return only what the answer needs:

```js
const r = await tools.mcp__sjel__comms__get_triage({});
const rows = JSON.parse(r.content[0].text);
const byClass = {};
for (const row of rows) byClass[row.data_class] = (byClass[row.data_class] ?? 0) + 1;
text({ total: rows.length, byClass });             // the model sees this, not 384 rows
```

Writes take the same path, with the body its contract declares:

```js
const r = await tools.mcp__sjel__comms__post_triage_id_gmail({
  id: "<id from a read>",
  body: { action: "archive" },                    // archive | trash | restore
});
text(r.content[0].text);
```

Nothing here is required to operate Sjel: without an MCP server, use the shell commands above
and the same contracts. The connection is owned and reversible — `sjel mcp register <harness>`
writes it and verifies it, `sjel mcp unregister <harness>` removes it, and a capability can be
narrowed to `read-only` or `off` on the Systems page without removing the server.

In Pi, `--tools` is an allowlist over every source, MCP included. Naming only `codemode` there
leaves `ALL_TOOLS` empty and the server's tools unreachable, which reads as a broken connection.
Either omit `--tools`, or name the MCP tools beside it.
