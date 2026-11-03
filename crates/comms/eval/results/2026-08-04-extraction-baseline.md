# Extraction baseline — 2026-08-04

Command:

```sh
cargo run --quiet --bin comms-extraction-eval -- eval/extraction-corpus.json
```

Fixed acceptance gate: useful retention at least 100.0%; boilerplate leakage at
most 0.0%. The stored snapshots and judgements were written before this result
was run.

| Fixture | Input class | Raw → clean chars | Total retained | Useful retained | Boilerplate leaked | Result |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| article-with-page-furniture | article | 123 → 75 | 61.0% | 100.0% | 0.0% | PASS |
| repository-readme | repository | 104 → 75 | 72.1% | 100.0% | 0.0% | PASS |
| paper-abstract | paper | 106 → 95 | 89.6% | 100.0% | 0.0% | PASS |
| client-rendered-page | client-rendered page | 204 → 81 | 39.7% | 100.0% | 0.0% | PASS |
| captured-page | captured page | 106 → 80 | 75.5% | 100.0% | 0.0% | PASS |
| pdf-text | PDF | 98 → 81 | 82.7% | 100.0% | 0.0% | PASS |

Overall: **PASS**. All six declared input classes and all six inspectable
normalization rules have fixture coverage.

This baseline measures deterministic snapshots at the extractor/normalizer
seam. It does not claim byte-level PDF extraction coverage: xberg adoption is
still under its dependency cooldown, so the PDF snapshot begins with extracted
text. Live network behavior remains outside this offline gate.
