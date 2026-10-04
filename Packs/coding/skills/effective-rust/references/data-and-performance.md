# Data Access, Performance & Idioms

## 0. Measure Before Optimizing
Treat performance changes as hypotheses. Start with a representative workload and a repeatable release-mode baseline, change one relevant thing, then measure again under the same conditions. Prefer an existing benchmark target; add a focused `cargo bench` benchmark when repeated measurement will help answer the question. Record the workload and result so a claimed improvement can be checked.

Use profiling to locate CPU or allocation costs before introducing layout changes, custom allocators, SIMD, or unsafe code. Check which profiler or benchmark tools are actually available in the host toolchain rather than assuming one is installed. Do not add Criterion or another benchmark dependency just to satisfy a template; use the repository's existing setup or the smallest useful benchmark.

## 1. SQLite Discipline with `sjel-store`
All persistent capabilities share one SQLite database file on disk, separated by table prefixes (`sjel-store` doctrine):
* **One Connection Pool**: `sjel_store::pool_for(db_path)` maintains pooled connections with WAL mode, `busy_timeout = 5000`, and `foreign_keys = ON`.
* **Migrate Once**: Call `sjel_store::migrate_once(db_path, prefix, migration_sql)` on startup. Never run raw DDL on request paths.
* **Prepared Statements**: `rusqlite` connections in `sjel-store` cache 64 prepared statements. Reuse parameterized queries (`conn.prepare_cached(...)`) rather than dynamically formatting query strings.

## 2. Floating Point Safety (`total_cmp`)
* **Never call `.partial_cmp(&other).unwrap()` on `f32` or `f64`.**
  - If either operand is `NaN`, `partial_cmp` returns `None`, and `.unwrap()` panics immediately.
* **Always use `.total_cmp(&other)`** (available since Rust 1.62):
  ```rust
  // INCORRECT (Panics on NaN):
  items.iter().max_by(|a, b| a.score.partial_cmp(&b.score).unwrap());

  // CORRECT (Safe, total ordering conforming to IEEE 754-2008):
  items.iter().max_by(|a, b| a.score.total_cmp(&b.score));
  ```

## 3. Allocation & Ownership Hygiene
* **Pass slices instead of owned containers**:
  - Use `&[T]` instead of `&Vec<T>`.
  - Use `&str` instead of `&String`.
  - Use `&Path` instead of `&PathBuf`.
* **Avoid unnecessary `.clone()`**:
  - Check whether a reference or borrowing suffices before cloning.
  - When storing strings in immutable registries or long-lived caches, consider `Arc<str>` or `Box<str>` to save pointer indirection / heap excess.
  - Use `std::borrow::Cow<'a, str>` when string modifications (such as redaction, trimming, or normalization) happen only on some inputs:
    ```rust
    fn normalize_slug(input: &str) -> std::borrow::Cow<'_, str> {
        if input.chars().all(|c| c.is_ascii_lowercase()) {
            std::borrow::Cow::Borrowed(input)
        } else {
            std::borrow::Cow::Owned(input.to_ascii_lowercase())
        }
    }
    ```

## 4. Serde Idioms
* Avoid allocating intermediate `serde_json::Value` structures when deserializing known schemas into concrete structs.
* Use `#[serde(default)]` on optional or additive fields to maintain forward compatibility across schema versions.
* For zero-copy deserialization where input JSON/bytes outlive the model, use `&'a str` fields with `#[derive(Deserialize)]`.
