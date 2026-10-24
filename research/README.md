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
- [How much does the assistant's harness matter?](assistant-harness.md): HarnessTax, the local
  Pi/Sjel smoke test, Meta-Harness and Schema, with a source index and limits on transfer to
  factual research, writing and professional tasks. [Track 2](../PLAN.md#track-2-turso-for-agents-and-services)
  owns the evaluation plan.
- [Agent harness development cases](benchmarks/agent-harness/README.md): twelve coding,
  research/experiment and Sjel-document cases with frozen fixtures, expected evidence and
  evaluator checks; labels await human approval. Live Pi and Sjel adapters share one tool
  broker. Recorded so far: unscored pipeline smoke runs and a retrieval-only check.
- [Graph store spike](benchmarks/graph-store/README.md): libSQL edge tables against embedded
  Grafeo on Graphify's repository graph.
- [Can a local model make typed decisions for Sjel?](local-decision-models.md):
  CLM candidate scoring, Apple Foundation Models, keyword rules, and the limits of a synthetic
  English/German Documents comparison.
- [Can the phone read Sjel's data without this Mac?](offline-on-the-phone.md): five routes to an
  offline copy, measured against this machine's 275 MB / 108-table database, and what decides each.

## Ideas

Questions worth an entry, not yet researched. An idea becomes an entry when it has sources. A
project named here has a `watch` row in `../upstreams.toml`.

- Can Ollama's System One API replace the CLM service for typed decisions? Ollama 0.35.0 on this
  Mac answers `/v1/systemone`, and the Nimble model (`nimble` in `../upstreams.toml`) is not
  pulled yet. Nothing is measured. Extends [the local decision entry](local-decision-models.md).
- Is one native GPUI surface worth a third UI stack? Sjel has a Svelte dashboard and a SwiftUI
  menu-bar app. The talk that raised it measures render speed. Measured 2026-10-09: Sjel's waits
  come from a service, not from rendering. A `/context` day switch settles in 26 ms in Chrome, and
  one shell endpoint takes over a second. The 2026-10-09 decision gave GPUI a command palette,
  judged on start time and memory. The wider decision of 2026-10-15 is in
  [`../apps/gpui-prototype/README.md`](../apps/gpui-prototype/README.md). Neither number is
  measured yet. Source:
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
- Could the feed install on its own? A colleague fetches podcasts every day and summarizes them
  with a local Whisper model. On 2026-08-12 they said many people would want Sjel's feed if it
  were easy to set up. The README quick start installs the whole system, and nothing installs
  the feed alone. What a feed-only install needs is not measured.
- Where does the redactor's 100% recall stop? `deterministic-entity-redaction-v3` found 48 of 48
  labels on the frozen 24-fixture corpus, and the `presidio` comparison set that bar. No document
  names a corpus size, language or document class where its rules should fail. Until one does,
  the 100% describes the corpus, not the redactor.
- Is a health domain worth a capability? The PRD named energy, sleep, training and records, and
  nothing in Sjel reads any of them. No source is collected yet.
- How could Sjel be funded, and do parts stay closed? AGPL-3.0 keeps open source and open core
  both possible. The options named so far are support and sponsorship with a build-in-public
  video series (decisions, papers, measurements, in the style of James Simo's city-builder
  devlog), hosting, or a company. Undecided.

## To verify

- Athey, Catalini and Tucker, "The Digital Privacy Paradox", NBER Working Paper 23488, 2017
  (https://www.nber.org/papers/w23488). Reported: a free pizza halved the share of students who
  protected their friends' data. The abstract did not load on 2026-09-27, so it is not an entry yet.
