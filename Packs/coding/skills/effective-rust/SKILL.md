---
name: effective-rust
description: Guides authoring, review, refactoring, and auditing of idiomatic, clean, safe Rust across Sjel workspace crates (capabilities, libs, tools) per Sjel doctrine, Effective Rust principles, and zero-unsafe invariants. Use when writing new Rust modules, refactoring crates, reviewing error handling or concurrency, optimizing performance, or auditing unsafe code in the workspace. Do not use for frontend TypeScript/Svelte code or non-Rust scripting.
allowed-tools: Read, Write, Edit, Bash
---

# Effective Rust

Write clean, idiomatic, and robust Rust aligned with Sjel doctrine, Effective Rust principles, and high-performance systems engineering (Polars, Tokio, Vector standards).
Zero-context ground truth: verify with `cargo check --workspace` and `cargo test -p <crate>`.

## Core Invariants

1. **Safe Rust by Default**: The root workspace is 100% safe Rust (`unsafe_code = "deny"`). No `unsafe` code without a benchmark proving necessity, an explicit authorization, and a documented `// SAFETY:` contract (PRD Q77, ISA.md).
2. **Backend Logic Defaults to Rust**: New backend logic is written in Rust before shell, Python, or TypeScript.
3. **One Database, Table Prefixes**: Persistent capabilities open SQLite through `sjel-store` using assigned table prefixes (`<prefix>_<table>`) and `migrate_once`. Never run unpooled, unmigrated raw DDL.
4. **Mechanical Sympathy**: Pre-allocate known capacities (`Vec::with_capacity`), avoid false sharing with cache line padding, and write branchless code for autovectorization.
5. **Resilient Concurrency**: Bounded channels only; cancel-safe `tokio::select!` branches; no synchronous blocking calls on async worker threads; no blind `Ordering::SeqCst`.

## The 5 Systems Disciplines

### 1. Soundness & Unsafe Guardrails (The Rustonomicon Standard)
* Default to 100% safe Rust.
* Every `unsafe` block must have an explicit `// SAFETY:` comment detailing:
  1. Preconditions (pointer validity, non-null, alignment).
  2. Invariants preserved.
  3. Aliasing & Provenance (Stacked/Tree Borrows proof).
* No raw uninitialized memory (`mem::zeroed`); mandate `std::mem::MaybeUninit<T>`.
* Use strict pointer provenance APIs (`.wrapping_add()`, `.with_addr()`, `.cast()`) over `usize as *mut T`.
* Validate unsafe code with `cargo miri test`.
* Read [`references/unsafe-and-soundness.md`](references/unsafe-and-soundness.md).

### 2. Hardware-Sympathetic & Data-Oriented Design (The Polars Standard)
* Use Struct of Arrays (SoA) over Array of Structs (AoS) for analytics and batch engines.
* Align hot data and shared atomics to 64 bytes using `crossbeam_utils::CachePadded` to prevent false sharing.
* Forbid hidden allocations in hot loops; always pre-allocate with `Vec::with_capacity(N)` when bounds are known.
* Leverage Apache Arrow columnar layouts (`arrow`, `parquet`) for structured data.
* Read [`references/data-oriented-and-cache.md`](references/data-oriented-and-cache.md).

### 3. High-Throughput Async & Backpressure (The Vector Standard)
* Never use unbounded channels (`tokio::sync::mpsc::unbounded_channel`). Always use bounded channels to propagate backpressure.
* Ensure all branches in `tokio::select!` are cancellation-safe.
* Offload blocking I/O (SQLite, disk, CPU-heavy operations) strictly via `tokio::task::spawn_blocking`.
* Avoid blind `Ordering::SeqCst`; specify explicit `Acquire`, `Release`, or `Relaxed` ordering with rationale.
* Never hold `std::sync::MutexGuard` across an `.await` boundary; prefer `parking_lot::Mutex` or `tokio::sync::Mutex`.
* Read [`references/async-and-concurrency.md`](references/async-and-concurrency.md).

### 4. Ergonomic Type-System Modeling & Zero-Cost Abstractions
* Model state machines using the Type-State pattern (`Order<Draft>` -> `Order<Confirmed>`) to make illegal states unrepresentable.
* Parse, don't validate: wrap primitives into Newtypes (`StationId`, `PlanId`).
* Maximize zero-copy transformations with `Cow<'a, str>` and `#[serde(borrow)]`.
* Define fine-grained error hierarchies with `thiserror` and convert them into Axum HTTP responses with `IntoResponse`.
* Read [`references/type-state-and-modeling.md`](references/type-state-and-modeling.md) and [`references/error-architecture.md`](references/error-architecture.md).

### 5. Automated Verification & Adversarial Testing
* Write property-based tests using `proptest` for parsers, numerical routines, and state machines.
* Validate lock-free primitives and atomic interleavings with `loom`.
* Run fuzz testing (`cargo-fuzz`) on untrusted data boundaries.
* Verify clean compiler and linter passes with `cargo clippy --workspace --tests` and `cargo check --workspace`.
* Read [`references/verification-and-testing.md`](references/verification-and-testing.md).
* Keep panics out of fallible input and runtime paths; use checked arithmetic when overflow is a meaningful failure. See [`references/error-architecture.md`](references/error-architecture.md).
* Measure performance changes against a repeatable baseline before optimizing; profile to find the cost first. See [`references/data-and-performance.md`](references/data-and-performance.md).

## Reference Routing

| Topic | Reference |
| --- | --- |
| Sjel placement, workspace dependencies, safety rules | [`references/core-doctrine.md`](references/core-doctrine.md) |
| Unsafe rules, `// SAFETY:`, provenance, `MaybeUninit`, Miri | [`references/unsafe-and-soundness.md`](references/unsafe-and-soundness.md) |
| Cache alignment, SoA, capacity budgets, SIMD, Arrow | [`references/data-oriented-and-cache.md`](references/data-oriented-and-cache.md) |
| Bounded channels, cancellation safety, thread budgeting, atomics | [`references/async-and-concurrency.md`](references/async-and-concurrency.md) |
| Typestates, Newtypes, `Cow`, `#[serde(borrow)]` | [`references/type-state-and-modeling.md`](references/type-state-and-modeling.md) |
| Domain errors, `thiserror`, Axum `IntoResponse` | [`references/error-architecture.md`](references/error-architecture.md) |
| Property testing (`proptest`), `loom`, `cargo-fuzz`, verification | [`references/verification-and-testing.md`](references/verification-and-testing.md) |
| Panics, checked arithmetic, performance measurement | [`references/error-architecture.md`](references/error-architecture.md) · [`references/data-and-performance.md`](references/data-and-performance.md) |
