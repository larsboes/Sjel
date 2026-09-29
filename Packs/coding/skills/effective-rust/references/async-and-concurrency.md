# High-Throughput Async & Concurrency (The Vector Standard)

Vector and Tokio achieve rock-solid reliability by enforcing backpressure, thread budgeting, cancellation safety, and precise atomic ordering.

## 1. Strictly Bounded Channels & Backpressure
* **Never use unbounded channels (`tokio::sync::mpsc::unbounded_channel`).**
  An unbounded queue under high ingestion rates will consume memory without limit until the OS triggers an Out-Of-Memory (OOM) kill.
* **Always use bounded channels (`tokio::sync::mpsc::channel(N)`).**
  Choose buffer size `N` based on expected burst size. When the channel fills, the sender's `.send().await` suspends, naturally pushing backpressure upstream to the network or caller.

## 2. Cancellation Safety in `tokio::select!`
In Tokio, when one branch of a `tokio::select!` completes, the futures in all other branches are immediately dropped.
* **Rule: Ensure every future in a `tokio::select!` branch is cancellation-safe.**
* If a future is cancelled midway:
  - Reading from an `AsyncRead` buffer via `.read()` drops any partially buffered bytes that were not returned. Use `tokio_util::codec` or save partially read data in an external struct.
  - An in-flight database transaction must roll back cleanly on drop rather than leaving orphaned state.
* If an operation must not be interrupted, shield it:
  ```rust
  // Protect non-cancellation-safe steps:
  tokio::spawn(async move {
      atomic_multi_step_flush().await;
  });
  ```

## 3. Tokio Thread Budgeting
* Axum handlers and async tasks run on a cooperative thread pool (typically `num_cpus` worker threads).
* **Rule: Never perform blocking I/O or long CPU computation in cooperative tasks.**
  - `rusqlite` / database queries.
  - Filesystem reads/writes (`std::fs`).
  - CPU-heavy parsing, regex scanning, or cryptographic hashing.
* **Always offload blocking work via `tokio::task::spawn_blocking`**:
  ```rust
  let result = tokio::task::spawn_blocking(move || {
      store.query_items()
  }).await?;
  ```

## 4. Atomic Memory Ordering (No Blind `SeqCst`)
Do not use `Ordering::SeqCst` as a blanket default. Sequential consistency adds expensive hardware memory barriers on modern multi-core architectures (especially ARM64 / Apple Silicon).
Always specify the minimal required ordering and document why:
* **`Ordering::Relaxed`**: For simple counters or stats where inter-variable ordering does not matter (e.g. `request_count.fetch_add(1, Ordering::Relaxed)`).
* **`Ordering::Release`**: When publishing state or setting a dirty flag so all prior writes are visible to other threads (e.g. `DIRTY_FLAG.store(true, Ordering::Release)`).
* **`Ordering::Acquire`**: When consuming published state to ensure subsequent reads see all changes prior to the `Release` store (e.g. `DIRTY_FLAG.load(Ordering::Acquire)`).
* **`Ordering::AcqRel`**: On read-modify-write swaps/CAS operations that both acquire previous state and release new state (e.g. `DIRTY_FLAG.swap(false, Ordering::AcqRel)`).

## 5. Mutex Discipline Across `.await`
* **Rule: Never hold a `std::sync::MutexGuard` across an `.await` point.**
  Holding a synchronous guard across `.await` leads to deadlocks and breaks `Send` requirements.
* If a lock does not span `.await`, prefer `parking_lot::Mutex` (faster, non-poisoning).
* If a lock must span `.await`, use `tokio::sync::Mutex` or `tokio::sync::RwLock`.
