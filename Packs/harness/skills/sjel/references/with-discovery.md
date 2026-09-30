# Discover Sjel services

Always discover before operating.

1. Run `tools/axon-context with [capability]`.
2. For the current service state, run `sjel capability list` or `sjel capability health`.
3. For one HTTP base URL, run `sjel capability url <capability>`.
4. Read only the returned capability README and manifest before composing an unfamiliar call.

The registry owns service identity, ports, health paths, dependencies, and proxy behavior. Do not
keep an endpoint or port table in this skill. A registered HTTP surface is not proof that its
process is running; probe it when the task depends on availability.

If a capability has no HTTP surface, use the native CLI or protocol documented in its README.
If discovery fails, follow `references/shared-failure-policy.md`.
