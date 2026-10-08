# coding pack

Foundational engineering and craftsmanship across backend and frontend: systems-grade Rust, official Svelte 5 with runes, and TypeScript.

- **`effective-rust`**: Systems-grade Rust per Sjel doctrine, Effective Rust principles, zero-unsafe invariants, mechanical sympathy (Polars data layout, Vector async backpressure), typestates, and adversarial testing.
- **`gpui`**: Native Rust desktop development with Zed's pre-1.0 GPUI, including revision-aware source lookup, view/entity patterns, platform setup, and UI event handling.
- **`svelte-core-bestpractices`**: Modern Svelte 5 best practices covering runes (`$state`, `$derived`, `$effect`, `$props`), snippets, function bindings, stores-to-runes migration in `.svelte.ts`, SvelteKit 2 routing with `$app/state`, and local API/IPC state orchestration.
- **`svelte-code-writer`**: Official Svelte 5 CLI tools for documentation lookup (`list-sections`, `get-documentation`) and automated component analysis (`svelte-autofixer`).

## Activation

Deploy to your active harness:

```bash
tools/packs.sh link coding
```

## Attribution

- **`svelte-core-bestpractices`** and **`svelte-code-writer`** are adopted from [sveltejs/ai-tools](https://github.com/sveltejs/ai-tools) (MIT license, Copyright (c) 2025 Svelte Contributors).
- **`effective-rust`** is derived from Sjel doctrine and the *Effective Rust Agent Skills* specification.
- **`gpui`** is original Sjel guidance grounded in the linked Zed Industries GPUI source and the pinned evaluation prototype.
