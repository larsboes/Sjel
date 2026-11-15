# coding pack

Engineering craftsmanship: systems-grade Rust, native GPUI and proportionate prior-art investigation.

- **`effective-rust`**: Systems-grade Rust per Sjel doctrine, Effective Rust principles, zero-unsafe invariants, mechanical sympathy (Polars data layout, Vector async backpressure), typestates, and adversarial testing.
- **`gpui`**: Native Rust desktop development with GPUI and GPUI Kit, including release-specific source lookup, component ownership and recipes, headless UI interaction tests, platform setup, and UI event handling. It routes shared visual and product design decisions to `ui-craftsmanship` in the design Pack.

- **`prior-art`**: Minimum-sufficient coding work across planning, implementation and review. Read the affected code, keep known-owner changes local and revisit the build ladder when evidence changes the approach, not after every edit. Preserve safety and contracts rather than minimize line count. Synthetic probes and comparison evidence live in `skills/prior-art/evals/`.

## Activation

Deploy to your active harness:

```bash
tools/packs.sh link coding
```

## Attribution

- **`effective-rust`** is derived from Sjel doctrine and the *Effective Rust Agent Skills* specification.
- **`gpui`** is original Sjel guidance grounded in Zed Industries GPUI and [Longbridge GPUI Kit](https://github.com/longbridge/gpui-kit), reviewed at `39ab849a93b5f15507b969164d49891db8d5b470` (the source revision recorded by the cached `gpui-kit` release). The Kit skill references inform component ownership, complete recipes and headless interaction testing; the local guidance is independently written and links to the application's resolved revision. No upstream skill, code example or prose is copied. Upstream normative design, architecture, naming and public-field policies are not adopted. GPUI Kit uses [Apache-2.0](https://github.com/longbridge/gpui-kit/blob/39ab849a93b5f15507b969164d49891db8d5b470/LICENSE-APACHE), with an additional [CC BY 4.0 documentation offer](https://github.com/longbridge/gpui-kit/blob/39ab849a93b5f15507b969164d49891db8d5b470/LICENSE-DOCS.md) for the specified website material.
- **`prior-art`** owns the existing Sjel build ladder. [Ponytail](https://github.com/DietrichGebert/ponytail) (MIT, `upstreams.toml [ponytail]`) informed the comparison of code understanding and minimality review. The revised guidance is independently written; no upstream workflow, code or text is copied.
