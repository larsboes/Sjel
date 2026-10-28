# Anti-hallucination protocol

Humanizing prose is exactly when fabrication creeps in. A flat sentence gets "fixed" with a
vivid invented detail. A hedge becomes a confident false claim. A vague gesture grows a fake
statistic. The rewrite must add *voice*, never *facts*.

A regex cannot catch a fabricated fact, so this is judgment rather than a linter check. The
scanner only flags the vague-attribution and fabricated-specificity *shapes* that tend to
accompany it.

## The invariants

These survive the rewrite unchanged. Diff-check before applying anything:

- Every number, unit, date, and measured value.
- All code, commands, config keys and values, file paths, CLI flags.
- All links and citations.
- Defined terms and proper nouns: product, model, and library names.
- PII and confidentiality rules. Never add PII or internal-only URLs.
- Claims. Wording may be sharpened; a claim may never be strengthened, weakened, or invented
  to improve flow.

If a rewrite would change any invariant, revert that span and keep the original.

## The protocol

**1. Build a claim inventory first.** Before editing, list every checkable claim in the
source as `{claim, value, source-span}`: numbers, dates, named entities, citations, and
causal or comparative assertions ("X is faster than Y"). Nothing on this list may change
value, and nothing new may join it. This is the artifact the final diff runs against.

**2. Add no specificity the source does not contain.** Vague to concrete is allowed only when
the concrete detail is already present or directly entailed. If the source says "improved
performance", the rewrite may not say "cut latency 40%" unless that number is in the source.
Sharpen the wording, not the facts.

The reverse holds too: keep the author's precision level. "Overnight the Kafka dashboard view
went towards zero" is a correct observation at the precision the author had. Do not flag it as
vague, do not name the metric for them, and do not pad it with a hedge.

**3. Cut empty sentences rather than dressing them.** When a sentence says nothing, delete
it. Never rescue it by inventing a detail, an example, a quote, or a source.

**4. Never invent attribution.** Do not add "studies show", a named researcher, a date, a
company, or a URL that was not in the source. Removing vague attribution is good. Replacing
it with a fabricated specific source is worse than leaving it vague.

**5. Mark gaps rather than filling them.** When the prose needs a fact that is not available,
emit an explicit `[SOURCE NEEDED]`, `[FIGURE?]`, or `[VERIFY]` and call it out. A visible gap
is honest; invented filler is a hallucination. Every placeholder gets enumerated in the
audit, and any output still carrying one is flagged draft-pending, never presented as
finished. Only the author removes a placeholder.

**6. Preserve quotes and citations verbatim.** Never alter wording inside quotation marks,
never turn a paraphrase into a quote, and never attach a citation to a claim it does not
support in order to make a sharpened sentence look sourced.

**7. GENERATE mode is held to the same bar, with the brief as the invariant set.** Drafting
from a brief does not license invented statistics, quotes, case studies, or citations. Write
only what the brief supports. The audit must list every fact stated that the brief did not
provide, so the author can verify or cut it.

**8. Run the diff before returning.** Compare the rewrite's claim inventory against the
source's and sort every difference into four buckets: **added**, **strengthened**,
**weakened**, **dropped**. Renumbering counts as changed.

- Added, strengthened, and weakened are regressions. Revert that span.
- A dropped real claim or caveat is silent information loss. Restore it unless the deletion
  was deliberate and logged.

Report the bucketed diff in the audit block.

## Scope and ethics

The promise is prose that reads as written by a skilled human, which also happens not to trip
detectors. It is not disguising machine text. Never use the adversarial tricks from the
evasion literature: Unicode homoglyphs, zero-width characters, deliberate typos,
meaning-degrading synonym swaps, or fabricated facts and quotes. They wreck the text.

Detectors are unreliable in ways that matter ethically. Liang et al. (2023) found they
disproportionately misclassify non-native-English writing as AI-generated. That is the
strongest reason no detector is ground truth, and another reason the goal is genuinely better
writing rather than a passing score.

---

Extracted from the anti-hallucination protocol and invariant guard in
[stephenoffer/human-voice](https://github.com/stephenoffer/human-voice) (MIT), pinned
`9bcba2f`, and condensed out of that skill's SKILL.md to keep this one's body lean.
