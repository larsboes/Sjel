# Hardware-Sympathetic & Data-Oriented Design (The Polars Standard)

High-performance data systems (like Polars, Ripgrep, and Arrow) maximize cache locality, eliminate redundant heap allocations, and assist LLVM with autovectorization.

## 1. Struct of Arrays (SoA) vs. Array of Structs (AoS)
For batch query engines, analytics, and bulk scanning (such as `capabilities/finance` projections or `libs/candidate-fingerprint` searches):
* **AoS (Array of Structs)**: `Vec<Record>` interleaves all fields in cache lines. A query filtering only `timestamp` drags unrelated fields into L1/L2 cache, degrading memory bandwidth.
* **SoA (Struct of Arrays)**: Separate contiguous buffers per column/field:
  ```rust
  // Columnar layout: scanning timestamps touches ONLY timestamp cache lines
  pub struct TransactionBatch {
      pub timestamps: Vec<i64>,
      pub amounts_cents: Vec<i64>,
      pub accounts: Vec<String>,
  }
  ```
* Leverage the Apache Arrow columnar format (`arrow`, `arrow-array`, `parquet`) already declared in the workspace for zero-copy IPC and structured analytics.

## 2. Cache Line Awareness & False Sharing
* Modern x86-64 and Apple Silicon (ARM64) cache lines are 64 bytes (or 128 bytes on some Apple cores).
* When multiple threads frequently update independent atomic counters residing on the same cache line, they cause **false sharing**—the CPU invalidates the entire cache line across cores, tanking throughput.
* **Solution**: Align shared atomic counters using `crossbeam_utils::CachePadded`:
  ```rust
  use crossbeam_utils::CachePadded;
  use std::sync::atomic::AtomicU64;

  pub struct ThreadMetrics {
      pub requests_processed: CachePadded<AtomicU64>,
      pub bytes_sent: CachePadded<AtomicU64>,
  }
  ```

## 3. Allocation Budgets & Capacity Discipline
* **Never use `Vec::new()` before a loop of known or estimated size.**
  Repeatedly pushing to an unallocated `Vec` triggers exponential reallocations (0 -> 4 -> 8 -> 16...) and heap copies:
  ```rust
  // BAD: Hidden reallocations in loop
  let mut rows = Vec::new();
  for tx in transactions {
      rows.push(transform(tx));
  }

  // GOOD: Single allocation
  let mut rows = Vec::with_capacity(transactions.len());
  for tx in transactions {
      rows.push(transform(tx));
  }
  ```
* For short-lived, high-volume batch workloads (parsers, tree evaluations, string tokenizers), use an arena allocator like `bumpalo` to allocate in chunks and reset in O(1).

## 4. Autovectorization & SIMD
* LLVM can autovectorize loops if:
  1. Iteration counts are known or easily bounded.
  2. Data is stored contiguously (slices, `Vec`).
  3. The loop body is free of branch mispredictions and panicking operations (`assert!`, `.unwrap()`).
* Write branchless arithmetic where possible (e.g. `val.min(max_val)` or bitwise operations instead of nested `if/else`).
