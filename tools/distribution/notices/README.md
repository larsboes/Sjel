# Supplemental dependency notices

These grants restore notices omitted from published dependency archives. Packaging applies each
supplement only to the named versions and matching declared license. License wording and copyright
notices are preserved; the qcms copy normalizes trailing whitespace.

| Copy | Applies to | Authoritative upstream notice |
| --- | --- | --- |
| `boolbase-1.0.0.LICENSE` | `boolbase@1.0.0`, ISC | [ISC grant](https://github.com/fb55/boolbase/blob/be0bcd8a4e917a0a5895e95b523fbbed05a64871/LICENSE) |
| `rust-alloc-no-stdlib.LICENSE` | `alloc-stdlib@0.2.4` and `0.3.0`, BSD-3-Clause | Identical grants at [0.2.4 source](https://github.com/dropbox/rust-alloc-no-stdlib/blob/ae42d22078b98549e987d2f03d12df7b984fde47/LICENSE) and [0.3.0 source](https://github.com/dropbox/rust-alloc-no-stdlib/blob/0a81fd6928ea3b33c8cd484aa4575d50ffb98012/LICENSE) |
| `html-to-markdown-rs-3.21.2.LICENSE` | `html-to-markdown-rs@3.21.2`, MIT | [License at the crate's recorded source revision](https://github.com/xberg-io/html-to-markdown/blob/3e109afcc2a51e5f379b7c98d6c5447a3a7db64f/LICENSE) |
| `qcms-0.3.0.COPYING` | `qcms@0.3.0`, MIT | [Copying notice at the crate's recorded source revision](https://github.com/FirefoxGraphics/qcms/blob/5ba6ed804b59273afc37794d5b04d27fb8835e48/COPYING) |
| `xberg-1.3.10.LICENSE` | `xberg@1.3.10`, MIT | [License at the crate's recorded source revision](https://github.com/xberg-io/xberg/blob/2b2d4186e7b0db959c15150008564ee41aa73d62/LICENSE) |
| `xberg-1.3.10.THIRD_PARTY_LICENSES.md` | `xberg@1.3.10` upstream native-library inventory | [Inventory at the same revision](https://github.com/xberg-io/xberg/blob/2b2d4186e7b0db959c15150008564ee41aa73d62/THIRD_PARTY_LICENSES.md) |

Rust revisions come from the published crates' `.cargo_vcs_info.json`. The Xberg inventory describes
upstream optional features and release configurations, not Sjel's enabled features. Relative links
inside that preserved upstream text refer to its repository. No upstream implementation is adopted
here. The corresponding-source archive includes these grants and their provenance with the source.
