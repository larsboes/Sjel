# tools/capability-auth — the header `sjel capability call` sends

`sjel capability call` sent no credential, so every gated route answered
`invalid or missing authentication token` to a caller with no browser session. This tool
resolves the deployment-wide inbound token and prints it as one request header.

The crate is `sjel-capability-auth`, a member of the root Cargo workspace. It calls
`InboundAuth::from_deployment().bearer_header()` (`libs/sjel-server/src/auth.rs:209`, `:304`),
the same call `capabilities/sjel-status/src/status/health.rs:192` and the `trips`, `finance`
and `vault` clients make. The token rule stays in one place.

## Behaviour

| Invocation | Output | Exit |
|---|---|---|
| `capability-auth` | `Authorization: Bearer <token>` | 0 |
| `capability-auth` with no token declared | nothing | 3 |
| `capability-auth comms` | comms' own token if `api_secret_file` names one, else the deployment's | 0 |
| `capability-auth --agent` | the agent token, read from the login Keychain (`sjel-agent-token`) | 0, or 3 if not enrolled |
| `capability-auth --check [--agent] [<capability>]` | `configured` or `absent` | 0 |
| `capability-auth enroll` (`sjel agent enroll`) | creates or rotates the agent token; prints paths, never the token | 0 |
| any other argument | a message on stderr | 2 |

`sjel` reads the header through process substitution (`curl -H @<(...)`), so the token is in
no argv and `ps` cannot show it. It sends the header to `http://127.0.0.1:` capabilities only.
An `external` capability answers on another host, and this token is a credential for this
machine's gate.

## What it does not do

It knows one per-capability token, comms' `api_secret_file`, read by
`sjel_server::comms_config_token` (`libs/sjel-server/src/auth.rs`), the same reader
sjel-status's proxy uses. Any other capability name gets the deployment-wide token.

Tests: `tools/capability-auth.test.sh`, against a throwaway overlay and a synthetic token.

## The agent token (ISA F9)

`enroll` puts a random token in the login Keychain through `security -i` on stdin, so it is in
no argument list. The server side gets only its SHA-256, in
`<overlay>/config/agent-token.sha256`, and a pseudonym key in
`<overlay>/secrets/agent-pseudonym.key` (created once, then kept). A capability that calls
`InboundAuth::admit_agents` admits that token for `GET` and `HEAD` only and pseudonymizes every
response (`libs/sjel-server/src/agent.rs`).

In an agent session (`CLAUDECODE=1` or `SJEL_AGENT=1`), `sjel capability call` sends the agent
token and nothing else. If no agent is enrolled, it fails and does not fall back to a full token.
