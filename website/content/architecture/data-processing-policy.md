# Data processing policy direction

This document records the intended direction for the next classification redesign. The current
four-class policy in [security](security.md) remains authoritative until the cross-capability gates
below are implemented and audited.

## Separate sensitivity from processor trust

Sensitivity describes the content and the harm if it is exposed. Processor trust describes where a
specific representation may be sent. These are separate decisions: a trusted processor does not
make a secret safe to send, and an ordinary cloud provider does not become eligible because a row
has a broad label.

The policy should evaluate the exact fields and their relationships before constructing a prompt:

1. **Deterministic processing** handles metadata rules, redaction and routing without a model.
2. **Local models** may process private and confidential material when the task needs it. Secrets,
   credentials, authentication codes and recovery material stay out of every model prompt.
3. **Verified zero data retention configurations** may receive only explicitly eligible,
   minimized fields. Invoices and similar records can be eligible after a field-level review.
   Names must not be joined to exact locations, detailed itineraries, or other sensitive context in
   the same request. Provider eligibility never overrides the secret block.
4. **Other cloud models** receive only public, uncritical or effectively pseudonymized material.

ZDR is a property of the exact account, product, endpoint and configuration, not a provider name.
OpenAI says API inputs and outputs are normally removed within 30 days and ZDR is available only
for eligible customers and endpoints; some features can still store application state. Anthropic
states its API has 30-day default retention, and ZDR requires an approved arrangement. Anthropic's
consumer Claude plans, including Claude Code used with those accounts, are not ZDR; their ZDR
arrangement applies to the API and products using a commercial organization API key. The
implementation must therefore fail closed unless the active route's ZDR eligibility is verified
for the deployed account and endpoint. See [OpenAI data controls](https://developers.openai.com/api/docs/guides/your-data),
[Anthropic commercial retention](https://privacy.claude.com/en/articles/7996866-how-long-do-you-store-my-organizations-data),
and [Anthropic ZDR scope](https://privacy.anthropic.com/en/articles/8956058-i-have-a-zero-data-retention-agreement-with-anthropic-what-products-does-it-apply-to).

Pseudonymization must be evaluated as a property of the whole request, not a token replacement
count. Stable identifiers, rare details, exact dates and location combinations can restore identity
or reveal a sensitive relationship. A derivative therefore needs a declared purpose, allowed field
set, transformation version, processor tier and source-class recheck at dispatch.

## Agent tiers at the gate

The agent gate (`libs/sjel-server/src/agent.rs`) applies the cloud tiers to agent sessions. Each
harness holds its own token (`sjel agent enroll <name>`, selected by `SJEL_AGENT_NAME`), and the
owner sets each name's tier with `sjel agent tier <name> <tier>`. The tier lives in the policy
file under `secrets/agent/`, which no agent can write.

| Tier | Meaning | View |
|---|---|---|
| `pseudonymized` | Any cloud model. The default for an unnamed or unknown token. | Names and identities become session tokens (`agent_view_public`). |
| `zdr` | A route whose zero data retention was verified for the account and endpoint. | `trusted_view` |
| `trusted` | A route the owner trusts without that proof. | `trusted_view` |

`trusted_view` (`libs/pseudonymize/src/view.rs`) keeps names. It still removes every c3 object,
drops coordinate and street-address keys, and replaces email addresses, phone numbers, IBANs and
token-shaped values. An ISO date stays. A sender keeps its display name and an organisation's
domain. Each call log line records the agent name and tier.

The gate gives each admitted agent request an `AdmittedAgent` extension that holds the name and
tier. It is a request extension and not a header, so a client cannot set it. A handler that
serves more than stored data reads the extension. For example, comms `GET /triage/{id}/body`
fetches one mail body from Gmail, stores nothing, and answers only `zdr` and `trusted`. The
deployment token gets 403 on that route.

## Implementation sequence

1. Inventory every class definition, local prompt gate, cloud admission gate, derivative builder,
   provider registry and persisted approval across `libs/content-item` and all capabilities.
2. Define a shared processing decision in the leaf policy library. Inputs should include source
   sensitivity, field categories, requested purpose and processor tier. Unknown values fail closed.
3. Migrate one capability at a time. Build prompts only from the approved field set; keep source
   text local; persist safe provenance and refusal reasons; recheck the current source class before
   dispatch.
4. Add adversarial tests for secrets, identity plus exact location, invoice fields, pseudonym joins,
   unknown classes and provider substitution. Audit API and agent projections for accidental
   disclosure.
5. Remove legacy gates only after every capability uses the shared decision and the migration
   evidence covers stored approvals and retries.

## Decisions still needed in implementation

- Define sensitivity and field-category vocabulary without overloading mail workflow categories.
- Choose how the trusted-provider allowlist is verified and refreshed at runtime.
- Define the minimum invoice fields needed for each supported task.
- Specify whether provider eligibility is keyed to a product/API, account configuration, or both.

The redesign must preserve the existing invariant that secrets never enter a model prompt and must
not permit identity plus exact-location/detail combinations to cross the cloud boundary.
