# Security & access control

Use the shared server gate when exposing a capability. Authentication and listener policy are
implemented in [`sjel-server`](../../libs/sjel-server/README.md); the
[devices capability](../../capabilities/devices/README.md) owns pairing.

## Security invariants

<a id="1-loopback-binding-only"></a>

### 1. Listener policy

Loopback is the default capability listener. Broader reach requires the explicit authenticated
listener policies in [`sjel-server`](../../libs/sjel-server/src/lib.rs); the separate
[LAN listener](../../libs/sjel-server/src/lan.rs) requires paired-device credentials.
Doctor's server-bind check enforces use of the shared server boundary in capability source.
Container network declarations have their own rules in the
[service schema](../../schemas/service.toml.example), including host networking where required.

### 2. Tailnet identity ingress gate

A configured `SJEL_TAILNET_OPERATOR` identifies the operator, but an identity header alone does
not prove that Tailscale supplied it. The trusted Tailnet proxy policy is reserved for a Unix
socket accessible to Tailscale Serve. Direct loopback callers must satisfy the selected credential
policy. See [`tailnet.rs`](../../libs/sjel-server/src/tailnet.rs) and
[`auth.rs`](../../libs/sjel-server/src/auth.rs).

The shared gate also supports deployment tokens, local browser sessions, and paired-device
signatures. Agent tokens are admitted only by capabilities that enable the agent gate, under their
per-capability access policy, with pseudonymized responses. Health and readiness routes have their own exemptions; consult the implementation
before changing an exemption or assuming all routes require the same credential.

### 3. LAN listener & key pairing

The LAN listener validates paired-device signatures. Pairing claim and join-request routes have
explicit unsigned exceptions so a new device can enroll. The signature headers and pairing
payloads are defined in [device authentication](../../schemas/device-auth.schema.json) and
[pairing](../../schemas/device-pairing.schema.json).

### 4. Zero secrets in repository

Keep credentials, private keys, personal data, and deployment values out of the public tree.
Configuration may refer to a credential file or OS keychain entry; the credential value stays
private. Follow the [security and data doctrine](../../CONTRIBUTING.md#security-and-data).

## Agent access policy

[`agent.rs`](../../libs/sjel-server/src/agent.rs) and
[`agent_policy.rs`](../../libs/sjel-server/src/agent_policy.rs) enforce four per-capability modes:
`off` refuses access, `read-only` admits reads, `ask` requires owner approval for writes, and
`auto` admits writes except routes declared `confirm`. The current default for an absent policy
entry is `auto`. A declared mutating GET counts as a write.

An approval is single-use and bound to the capability, method, target including query, and
restored request body. An initial approval request returns `202`; after the owner allows it,
the agent repeats the same write with `X-Sjel-Approval`. An approval for another capability or
body is refused.

Pseudonym restoration uses the receiving capability's session map. A token not previously issued
by that capability in the current session is refused, including a token seen only in another
capability's response. `X-Sjel-Agent-Session` scopes a conversation; without it, the gate uses the
current UTC day. JSON responses are pseudonymized and `c3` objects are withheld. The call log
records capability, method, path, status, and decision; it contains neither query nor body.

## Data classes

The data classes are `c0` Public, `c1` Mine, `c2` Others, `c3` Secret, ranked in that order. `c0`
and `c1` may reach a cloud model (`c1` only as a redacted derivative); `c2` and `c3` redact before
persistence and never reach a cloud model.

The policy is one function: `content_item::cloud_admission` in `libs/content-item`, a leaf crate
every capability already depends on. It admits two representations — `c0` unchanged to any declared
tier, and `c1` as a reviewed `c1` derivative to the `pseudonymized_personal` tier — and refuses
everything else, including every class outside the vocabulary. The `processing_policy` a reader is
shown is *derived* from it, so the label and the gate cannot disagree; they used to be two
expressions and did.

Enforced today, mechanically, on four independent gates: `cloud_derivative::prepare` builds no
approvable preview for anything that is not `c0` or `c1`, `cloud_derivative::tier_allows` (a thin
wrapper over `cloud_admission` that adds the transformation-version pin) refuses the dispatch,
dispatch re-reads the source row's **current** class so a derivative approved at `c1` stops
dispatching once the row becomes `c2`, and the `comms_content_cloud_derivatives` CHECK constraint
refuses the row.

`c3` is refused every local prompt too, and that is now a gate rather than a declaration:
`content_item::local_prompt_allowed` answers `false` for `c3` and for any unrecognized value, both
prompt-builders that read stored text ask it before they build a prompt —
`capabilities/comms/src/digest.rs` (digest, diagram and chart) and `capabilities/comms/src/media.rs`
(`summarize`) — and `processing_policy(..).local_processing` is derived from the same function. A
refused item gets a `local_refused` row that says so, not a missing one. Embed and rerank
(`libs/inference`) stay class-blind and loopback-or-nothing.

A refusal is missing evidence, never a free pass. A ranker that redistributes a refused factor's
weight over the surviving factors promotes exactly what the gate refused, so the refused share is
withheld instead and the weights sum below 1.0 on purpose (PRD Q89, 2026-09-05; measured on the
mail band in `capabilities/comms/README.md`).

Data may select an allow-listed behavior but may not become executable code.

## Not covered

Device loss is not in the threat model. What a stolen phone exposes is not stated.
