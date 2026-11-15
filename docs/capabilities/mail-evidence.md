# Mail evidence semantics

Comms extracts source claims. Inventory and Projects own review, identity resolution and changes
to their records. The shared staging contracts live in
[`inventory-api::mail_evidence`](../../crates/inventory-api/src/mail_evidence.rs) and
[`projects-api::mail_evidence`](../../crates/projects-api/src/mail_evidence.rs). The receivers
derive their request schemas from these types; Comms serializes the same types.

## Vocabulary and ownership

| Concept | Meaning | Authority |
|---|---|---|
| Source message | A message within a thread, referenced by `mail:v1:<thread>:<message>` | Comms |
| Evidence excerpt | Source text supporting a proposed claim | Comms grounds it; the destination retains it |
| Purchase event | An order, shipment, delivery, cancellation or return reported by a source | Inventory reviews it |
| Product title | A source label; neither a product identifier nor an owned item identifier | Unresolved until review |
| Order reference | A source order label, without a global uniqueness guarantee | Evidence attribute |
| Inventory item | A physical item with a stable identity and separate state history | Inventory |
| Project evidence | A claim assigned to an existing project; a task suggestion has no mutation authority | Projects |
| Reviewed link | A project record's reference to an Inventory item at a reviewed revision | Projects verifies with Inventory |

A product model, an order line and a physical item are distinct concepts. The current mail
pipeline does not create product-model or order-line entities. A title or an order reference must
not silently become either identity. The existing `inv:` reference identifies an Inventory item.

## Extraction and review

AFM is the only generative backend admitted by this pipeline. The default role is
`summarization_light`; an explicit role must also resolve to AFM with a declared context window.
Other local or hosted generators are refused before inference. AFM proposes grounded fields;
Sjel owns parsing, normalization, validation, identity resolution and review decisions.

The flow is source message → grounded proposal → owner review → record change or reviewed link.
An order or shipment cannot create an owned item. Only a reviewed delivery may do so. A
cancellation or return remains evidence for review; it does not change item state automatically.
Matching an existing item requires its current item and state revisions. Project evidence must
target a live project; applying it requires the reviewed destination record revision.

Comms validates excerpts against the in-memory message using whitespace-normalized text.
Truth and correct attribution still require review. A separate delivery statement
travels as `claim`, rather than disappearing when the item title occurs in another paragraph.
Both fields remain sensitive source excerpts. No full mail body is retained by this handoff.

`extraction_version` identifies the prompt/parser contract. It is separate from the proposal's
review `version`; it does not identify the inference backend or model. Legacy proposals omit
the extraction revision and separate claim. Inventory migrates its existing table with nullable
columns; Projects retains the optional field in its existing proposal payload.

Inventory keys proposals by `(source_id, line_id, event)`. Projects adds `project` to that key.
Comms deduplicates using the same distinction. The line hash normalizes whitespace and case;
it identifies an excerpt. Physical items and order lines require separate identities. Re-extraction with different
evidence or provenance at the same key conflicts rather than replacing an existing decision.

The caller supplies classification and source identity. The model cannot select either. `c3`
is excluded before prompting. `c2` evidence stays local; Inventory refuses item creation from it
because item records do not retain classification. Projects refuses evidence that would lower
the destination's classification.

## Verification

```sh
sjel cargo test --locked -p sjel-comms --lib mail_evidence
sjel cargo test --locked -p sjel-inventory --test mail_proposals --test mail_match --test mail_privacy
sjel cargo test --locked -p sjel-projects --bin projects-server mail_proposals
```

The [frozen synthetic corpus](../../crates/comms/eval/mail-evidence.json) checks event semantics,
grounding failures, quoted repeats, multiple lines and shared excerpts. Each accepted result
must deserialize and validate through its receiving contract. It makes no model calls and does
not measure model accuracy, product matching or real-mail recall. The area
[data plan](../plans/data.md#mail-evidence-semantics-and-resolution) owns remaining acceptance.

## Reviewed local evaluation

`comms-mail-evidence-eval` freezes independent labels and compares model predictions through
the production prompt, local inference admission and parser. It never stages proposals or changes
Inventory, Projects or Gmail. Build with `sjel cargo build --locked -p sjel-comms --bin
comms-mail-evidence-eval`, then run the emitted executable against the active deployment.
`sjel cargo run` uses hermetic test configuration and cannot select the deployed roles.

The `freeze CORPUS` command reads a review object from stdin. It contains `reviewed_by`,
`reviewed_at`, `representation` and `cases`. Each case contains a transient `source` and
`expected` labels. A source contains `id`, `source_id`, `data_class`, `stage`, `projects` and
`mail`. A label contains `event`, equivalent `titles`, an optional `project` and a short supporting
`evidence` excerpt. Empty labels explicitly identify a negative case. Labels must be reviewed
before any prediction is inspected. Agent review must be identified separately from human review.

The frozen corpus retains source references, labels, excerpts and input hashes. Full message
bodies travel on stdin and stay in memory. Outputs must be new files under the active private
overlay and are created with mode `0600`. Labels and private fixtures never belong in this
repository. Supply source text from an admitted source adapter without a temporary body file.

`run CORPUS REPORT ROLE` reads an array of the same sources on stdin. It validates every
fingerprint before inference and refuses changed sources, classifications or project candidates.
The role must resolve to AFM; use `summarization_light` on deployments that provide it.
A pseudonymized source representation must be supplied identically on repeat runs; token changes
require a fresh review and corpus. Pseudonymized thread bodies measure that representation,
separately from production's raw per-message classification and chunking.

Reports contain counts, safe failure codes, the resolved backend/model, inference-convention
fingerprint, corpus hash, role, extraction contract revision and run time. They contain no predicted excerpts.
`false_claims` and `missed_claims` compare accepted event/item/project assignments. Item titles
match only the reviewer-declared aliases after case and whitespace normalization; this is a
scoring rule, never a runtime identity matcher. `incorrect_targets` counts wrong assignments and
rejected unknown projects. `missed_item_mentions` counts expected claims whose item mention was
absent regardless of the predicted event. Duplicate predictions and parser rejections are reported
separately. A request failure leaves that case unscored, with a failure code. Empty or unscored
runs cannot be reported as an accuracy result. No acceptance threshold is inferred from a first
small baseline.
