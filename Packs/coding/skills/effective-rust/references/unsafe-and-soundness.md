# Soundness & Unsafe Guardrails (The Rustonomicon Standard)

Systems code in the caliber of Polars and Tokio requires strict soundness invariants. In the Sjel workspace, the default is 100% safe Rust (`unsafe_code = "deny"`). When an explicit bottleneck or FFI boundary necessitates `unsafe`, adhere strictly to these rules:

## 1. Mandatory `// SAFETY:` Contract
Never write an `unsafe` block or `unsafe fn` without an explicit `// SAFETY:` comment documenting three proofs:
1. **Preconditions**: What must be true about the memory, pointers, and alignment before entry (e.g. non-null, aligned to `align_of::<T>()`, dereferenceable for N bytes).
2. **Invariants**: What state remains intact during execution.
3. **Aliasing & Provenance (Stacked/Tree Borrows)**: Why references do not overlap, how mutable borrows remain exclusive, and why raw pointers retain valid provenance without undefined behavior (UB).

```rust
// SAFETY:
// 1. Precondition: `ptr` is non-null, properly aligned for `Header`, and points to an
//    initialized block allocated with layout `Header::LAYOUT`.
// 2. Invariant: No concurrent thread has access to this memory block; we hold exclusive ownership.
// 3. Aliasing: No active references exist to this block; creating `&mut *ptr` preserves Stacked Borrows.
unsafe {
    (*ptr).magic = MAGIC_BYTES;
}
```

## 2. Pointer Provenance vs. Integer Casts
Never cast `usize` directly to raw pointers. LLVM tracks pointer provenance; casting an arbitrary integer to a pointer invalidates alias analysis and causes UB under Tree Borrows:

```rust
// BAD: Destroys pointer provenance
let ptr = (addr + offset) as *mut u8;

// GOOD: Preserves pointer provenance
let ptr = base_ptr.wrapping_add(offset);
// Or Rust strict provenance APIs:
let ptr = base_ptr.with_addr(new_addr);
```

## 3. Uninitialized Memory: Mandate `MaybeUninit<T>`
Never use `std::mem::zeroed()` or uninitialized references for types with invalid bit patterns (references, `bool`, `char`, `NonZero*`, enums). Always use `std::mem::MaybeUninit<T>`:

```rust
// BAD (UB if T contains non-zeroable types like references or NonZeroU32):
let mut buf: [T; 64] = unsafe { std::mem::zeroed() };

// GOOD:
let mut buf: [std::mem::MaybeUninit<T>; 64] = [const { std::mem::MaybeUninit::uninit() }; 64];
// Initialize elements...
let initialized = unsafe { std::mem::transmute_copy(&buf) }; // or MaybeUninit::slice_assume_init_ref
```

## 4. No Arbitrary `mem::transmute`
Never use `mem::transmute` when standard library conversions, pointer casting (`cast::<U>()`), or safe byte manipulation (`bytemuck`, `zerocopy`) exist. `transmute` bypasses type-checking completely and is the most common source of LLVM alignment UB.

## 5. Verification: Miri in the Loop
Any module declaring `unsafe` must be validated with Miri:
```bash
cargo miri test -p <crate>
```
If Miri reports an invalid borrow, undefined byte read, or memory leak, treat it as a hard compiler error.
