# Data

The [intake contract](../architecture/data-intake.md) owns file, extraction, archive and clipping
boundaries. [Mail evidence](../capabilities/mail-evidence.md) owns its detailed review contract.
Tasks below own unfinished acceptance; [PLAN.md](../../PLAN.md) owns priority.

<a id="registry-derived-project-views"></a>

Registry-derived Mac and phone acceptance belongs to [derived views](gpui.md#derived-views).
[Knowledge cutover](../tasks/storage-knowledge-cutover.md) and
[personal lenses](../tasks/storage-personal-lenses.md) retain their existing task identities.

<a id="mail-evidence-semantics-and-resolution"></a>
<a id="mail-evidence-for-projects-and-inventory"></a>

## Mail evidence

- [Measure mail evidence on independent labels](../tasks/data-mail-evaluation.md).
- [Connect reviewed mail identity resolution](../tasks/data-mail-identities.md).
- [Show mail extraction provenance](../tasks/data-mail-provenance.md).
- [Preserve reviewed purchase corrections](../tasks/data-mail-lifecycle.md).
- [Accept reviewed mail proposals](../tasks/data-mail-end-to-end.md).
- [Join reviewed evidence through owner registries](../tasks/data-evidence-registry.md).

<a id="1-import"></a>
<a id="2-formats"></a>

## Import and archives

- [Accept every mixed-folder import route](../tasks/data-import-routing.md).
- [Finish knowledge-only vault intake](../tasks/data-vault-intake.md).
- [Move journal records to an owning capability](../tasks/data-journal-owner.md).
- [Accept a real folder import in the app](../tasks/data-import-app.md).
- [Accept archived note decoding and rendering](../tasks/data-note-formats.md).
- [Round-trip capability exports](../tasks/data-owner-exports.md).

<a id="content-aware-media-sorting"></a>
<a id="social-posts-from-media"></a>

## Media and Instagram

- [Accept content-aware Media review](../tasks/data-media-review.md).
- [Approve destinations for unsorted Media](../tasks/data-media-organization.md).
- [Stream large Media originals](../tasks/data-media-large.md).
- [Flag face presence without identifying people](../tasks/data-media-face-flag.md).
- [Prepare Instagram drafts in Media](../tasks/data-instagram-drafts.md).
- [Choose Instagram phone handoff](../tasks/data-instagram-handoff.md).

<a id="web-clipping-into-sjel"></a>

## Clipping and prompt handoff

- [Accept imported settings and a real clip](../tasks/data-clipper-first.md).
- [Accept browser Clipper parity](../tasks/data-clipper-parity.md).
- [Accept multi-turn clipping and citations](../tasks/data-clipper-chat.md).
- [Package native-messaging Safari capture](../tasks/data-clipper-safari.md).
- [Route reviewed clips to their owners](../tasks/data-clips-routing.md).
- [Accept prompt copying and retention](../tasks/data-prompt-outbox.md).

<a id="3-settings-surface-for-inference-and-keys"></a>
<a id="afm-and-sjel-owned-data-processing"></a>
<a id="4-adaptive-data"></a>

## Inference and measured adaptation

- [Expose backend health in existing Settings](../tasks/data-inference-settings.md).
- [Measure remaining local processing roles](../tasks/data-processing-roles.md).
- [Measure offered and chosen behavior](../tasks/data-adaptive-preferences.md).

<a id="5-domain-backlog"></a>

## Domain work

- [Evaluate source-linked document enrichment](../tasks/data-documents-enrichment.md).
- [Measure bilingual mail event extraction](../tasks/data-mail-events.md).
- [Label the mail classification corpus](../tasks/data-mail-labels.md).
- [Evaluate an incremental mail classifier](../tasks/data-mail-classifier.md).
- [Accept measured mail authenticity flags](../tasks/data-mail-authenticity.md).
- [Keep awaiting-reply mail visible](../tasks/data-mail-reply.md).
- [Declare personal place filters](../tasks/data-place-facts.md).
- [Reconcile portfolio identities and targets](../tasks/data-investment-inputs.md).
- [Merge duplicate wishes explicitly](../tasks/data-wishlist-merge.md).
- [Accept trip budget and transaction links](../tasks/data-trip-finance.md).

### Health

`crates/health` owns the operator's daily health values (`c1`).

- [Decide the health data owner](../tasks/task-bd7107c0-a1a4-4bbf-8206-19acd4ffc6a6.md).
- [Import the first real Health export](../tasks/task-c30b5f1e-8a96-4643-b3a4-f6c54dc93c7e.md).
- [Sync HealthKit from the iOS app](../tasks/task-fbaa58b2-0aff-47e4-a482-0aa248dbedab.md).
