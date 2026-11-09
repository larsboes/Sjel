# coding pack

Engineering craftsmanship: systems-grade Rust, native GPUI and proportionate prior-art investigation.

- **`effective-rust`**: Systems-grade Rust per Sjel doctrine, Effective Rust principles, zero-unsafe invariants, mechanical sympathy (Polars data layout, Vector async backpressure), typestates, and adversarial testing.
- **`gpui`**: Native Rust desktop development with Zed's pre-1.0 GPUI, including revision-aware source lookup, view/entity patterns, platform setup, and UI event handling. It routes shared visual and product design decisions to `ui-craftsmanship` in the design Pack.

- **`prior-art`**: Minimum-sufficient coding work across planning, implementation and review. Read the affected code, keep known-owner changes local and revisit the build ladder when evidence changes the approach, not after every edit. Preserve safety and contracts rather than minimize line count. Synthetic probes and comparison evidence live in `skills/prior-art/evals/`.

## Activation

Deploy to your active harness:

```bash
tools/packs.sh link coding
```

## Attribution

- **`effective-rust`** is derived from Sjel doctrine and the *Effective Rust Agent Skills* specification.
- **`gpui`** is original Sjel guidance grounded in the linked Zed Industries GPUI source and the pinned evaluation prototype.
- **`prior-art`** owns the existing Sjel build ladder. [Ponytail](https://github.com/DietrichGebert/ponytail) (MIT, `upstreams.toml [ponytail]`) informed the comparison of code understanding and minimality review. The revised guidance is independently written; no upstream workflow, code or text is copied.
