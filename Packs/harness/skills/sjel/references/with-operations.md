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
