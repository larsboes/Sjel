# Research

Why this project exists, and the evidence for it. One file per question. The dashboard renders
every file here under `/research`, beside a Projects view built from `../systems.toml` and
`../upstreams.toml`: the software Sjel runs next to, builds on, learned from, watches or declined.

Each entry states its claim, cites a durable source for every fact it did not measure, and ends
with what its sources do not show. An entry grows when new evidence arrives. It is corrected, not
deleted, when a source turns out wrong.

## Entries

- [Why Sjel exists](why-sjel.md): life data sits in many services outside the person's control.
- [Will people without technical experience use it?](simplicity-and-adoption.md): defaults,
  progressive disclosure, browsing over search, and limited interest in data ownership.
- [What can a model on the phone do?](on-device-models.md): Apple's 3B model, device limits, phone
  latency.
- [What reaches a cloud model, and does pseudonymizing it help?](cloud-models-and-privacy.md):
  personal data in chat logs, and inference from pseudonymized text.
- [Can an assistant act on a person's data safely?](agent-safety.md): agent benchmarks, prompt
  injection, confirmation fatigue.
- [How much does the assistant's harness matter?](assistant-harness.md): HarnessTax.
- [Can a local model make typed decisions for Sjel?](local-decision-models.md):
  CLM-8B, candidate scoring, the existing local adapter, and unproven deployment and calibration.

## Ideas

Questions worth an entry, not yet researched. An idea becomes an entry when it has sources. A
project named here has a `watch` row in `../upstreams.toml`.

- Can Ollama's System One API replace the CLM service for typed decisions? Ollama 0.35.0 on this
  Mac answers `/v1/systemone`, and the Nimble model (`nimble` in `../upstreams.toml`) is not
  pulled yet. Nothing is measured. Extends [the local decision entry](local-decision-models.md).
- Is one native GPUI surface worth a third UI stack? Sjel has a Svelte dashboard and a SwiftUI
  menu-bar app. The talk that raised it measures render speed; whether Sjel's waits come from
  rendering or from its services is not measured. Source:
  [Conrad Irwin, EuroRust 2025](https://www.youtube.com/watch?v=sheIOOf-xRo).
- Does a booking document need a reader of its own? `kitinerary` is read AND run: it installs on
  the Linux runtime, takes stdin and emits schema.org JSON-LD, and it extracted a JSON-LD
  `TrainReservation` from HTML intact, while a plain iCal `VEVENT` returned empty. macOS was
  measured as the expensive half: no Homebrew formula, KDE's prebuilt macOS tarball does not run
  (its only `LC_RPATH` is KDE's CI path), and the workable route is a Craft root from KDE's
  prebuilt packages. What stays unmeasured is the part that decides adoption: a real `.pkpass`,
  PDF or UIC barcode from this household. Watch rows: `db-rest`, `motis`.
- Is reviewed CSV enough for a bank, or do the European export formats need a reader? The CSV path
  exists and calls itself an edge format; `camt`, `ofx` and `mt940` appear nowhere. Which formats
  this household's bank produces is not recorded, so the gap has no size. Watch rows: `beancount`,
  `firefly-iii`.
- Is a standard contacts and calendar surface wanted at all? No vCard, iCalendar or CalDAV code
  exists and Google is reached through its own API. Ask what outside the phone app and the
  dashboard would read these records before asking which server. Watch row: `radicale`.
- Would an index beat the linear cosine? `capabilities/comms/src/relevance.rs` scores candidates
  in process and no vector extension is loaded. The candidate-set size at retrieval time is the
  missing number and it decides the row. Watch row: `sqlite-vec`.
- Is an in-process ONNX encoder cheaper per call than the MLX role? `libs/inference` resolves
  embeddings to MLX models chosen by `libs/extraction/eval`, and no row records why an in-process
  runtime was not the comparison. This is a benchmark rather than a research question. Watch row:
  `fastembed-rs`.
- What can a host's history see that `host-watch` cannot? It watches cumulative CPU and disk and
  stays silent otherwise by design. No list exists of the failures that leaves uncovered, and
  installing an agent to find out is the wrong order. Watch row: `beszel`.

## To verify

- Athey, Catalini and Tucker, "The Digital Privacy Paradox", NBER Working Paper 23488, 2017
  (https://www.nber.org/papers/w23488). Reported: a free pizza halved the share of students who
  protected their friends' data. The abstract did not load on 2026-09-27, so it is not an entry yet.
