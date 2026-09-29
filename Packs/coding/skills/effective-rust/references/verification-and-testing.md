# Automated Verification & Adversarial Testing

High-assurance systems require verification beyond simple unit tests:

## 1. Property-Based Testing (`proptest`)
For parsers, mathematical evaluators, serialization, and state machines, write property tests using `proptest` to test thousands of randomized edge cases (empty strings, Unicode non-characters, max bounds, NaN):

```rust
#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn parser_never_panics_on_arbitrary_input(s in "\\PC*") {
            let _ = crate::parse_entry(&s);
        }

        #[test]
        fn round_trip_serialization(amount in 0i64..1_000_000_000, currency in "[A-Z]{3}") {
            let money = Money::new(amount, &currency);
            let serialized = serde_json::to_string(&money).unwrap();
            let deserialized: Money = serde_json::from_str(&serialized).unwrap();
            prop_assert_eq!(money, deserialized);
        }
    }
}
```

## 2. Concurrency Permutation Testing (`loom`)
When implementing lock-free data structures, atomics, or custom synchronization channels, use `loom` to simulate all possible thread interleavings and detect data races:
```rust
#[cfg(loom)]
#[test]
fn test_concurrent_counter() {
    loom::model(|| {
        let counter = Arc::new(loom::sync::atomic::AtomicUsize::new(0));
        let c1 = counter.clone();
        let h1 = loom::thread::spawn(move || c1.fetch_add(1, Ordering::Relaxed));
        let c2 = counter.clone();
        let h2 = loom::thread::spawn(move || c2.fetch_add(1, Ordering::Relaxed));
        h1.join().unwrap();
        h2.join().unwrap();
        assert_eq!(counter.load(Ordering::Relaxed), 2);
    });
}
```

## 3. Fuzz Testing (`cargo-fuzz`)
For external data ingest points (e.g. `capabilities/transit` Hafas parsing, `capabilities/finance` CSV import, or `capabilities/comms` mail parsing), set up `cargo-fuzz` targets to stress-test memory safety and reject malformed inputs without crashing.

## 4. Verification Command Matrix
Before concluding changes, run the appropriate verification rungs:

| Command | Purpose |
| --- | --- |
| `cargo clippy --workspace --tests` | Lints, idioms, redundant clones, uninlined format strings |
| `cargo check --workspace` | Workspace compile check |
| `cargo test -p <crate>` | Fast unit and integration tests |
| `cargo miri test -p <crate>` | Undefined behavior, aliasing, and memory leak validation for `unsafe` code |
| `tools/doctor` | Comprehensive Axon/Sjel architecture, bind policy, and deployment hygiene |
