# Feed & communications architecture

Comms ingests observed information and serves the shared reader contract. The
[capability README](../../capabilities/comms/README.md) explains its scope; the schema and policy
implementation below define what a reader or inference caller may assume.

## 1. Unified content schema (`content-item-v2`)

[`schemas/content-item.schema.json`](../../schemas/content-item.schema.json) defines the canonical
reader object. It is a presentation contract, rather than a requirement that each ingestion
adapter persist its raw input in exactly that shape.

| Field | Contract |
| --- | --- |
| `schema_version` | Constant `content-item-v2` |
| `source` | `feed`, `mail`, or `calendar`; provider-specific kinds are separate |
| `id` | Nonempty identifier |
| `title` | String or null |
| `data_class` | Object containing the stored class, label, rationale, method, and version |
| `processing_policy` | Explicit policy derived from the class |
| `cloud_processing` | Cloud staging and processing state |
| `mail`, `calendar` | Source extensions, each nullable |

The schema also requires origins, links, processing evidence, and digest state. Import it rather
than reproducing a partial interface in a new reader. Rust types and policy decisions live in
[`content-item`](../../libs/content-item/src/lib.rs).

## 2. Privacy & redaction ladder

The classes are `c0` Public, `c1` Mine, `c2` Others, and `c3` Secret. An undeclared legacy item
reads as `c1/legacy`; an unknown class is refused by policy rather than treated as public.

| Class | Cloud-processing boundary |
| --- | --- |
| `c0` | Public content may have a verbatim representation, subject to the configured provider tier. |
| `c1` | A cloud representation must be locally reduced or pseudonymized, reviewed, and approved by its exact preview hash. |
| `c2`, `c3` | No cloud derivative can be prepared or staged. Pseudonymization does not create an exception. |

[`cloud_derivative.rs`](../../capabilities/comms/src/cloud_derivative.rs) prepares previews without
network access. [`cloud_admission`](../../libs/content-item/src/lib.rs) checks provider tier,
source class, derivative class, and transformation. Keep this boundary separate from the
[agent response view](../architecture/security.md#agent-access-policy), which pseudonymizes JSON
and withholds `c3` objects. Secret content never enters an inference prompt.

## 3. Gmail mutation boundary

Ingestion and remote mailbox changes have separate handlers. Inspect the
[Comms server](../../capabilities/comms/src/server/) before adding an action or changing its
confirmation policy. An agent token is not universally read-only: the owning capability's
`off`, `read-only`, `ask`, or `auto` policy governs access, and confirmation routes still require
approval. The [shared agent gate](../architecture/security.md#agent-access-policy) owns that
contract, including single-use approvals and pseudonym restoration.
