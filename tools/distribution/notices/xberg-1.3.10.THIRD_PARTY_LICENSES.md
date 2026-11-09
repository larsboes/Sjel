# Third-Party Licenses

Xberg itself is licensed under [MIT](LICENSE). This file documents
notable third-party **native** libraries that Xberg links against or that are
redistributed in published release artifacts, with an emphasis on copyleft
(LGPL/GPL) components and how Xberg stays compliant.

> Rust crate dependencies and their licenses are governed by `deny.toml`
> (`cargo deny check licenses`). This file covers the **system/native** libraries
> that are linked at the C ABI level and are not visible to `cargo deny`.

## libheif (HEIF / HEIC / AVIF) — LGPL

- **Feature:** optional `heic` Cargo feature (part of `full`/`formats`). The
  standalone CLI enables `formats`, so its release binaries link `libheif`
  dynamically.
- **License:** GNU Lesser General Public License (LGPL). See the upstream
  [`COPYING`](https://github.com/strukturag/libheif/blob/master/COPYING) for the
  authoritative version and text.
- **Linking:** **Dynamic only.** Xberg resolves `libheif` via `pkg-config`
  (`-lheif`) against the system shared library; it is never statically linked.
  The musl CLI container build explicitly disables `crt-static`
  (`RUSTFLAGS="-C target-feature=-crt-static"`) so the resulting binary loads
  `libheif.so` at runtime rather than embedding it. The static-build
  (`embedded-libheif`) feature has been **removed** from `xberg-libheif`, so
  there is no supported way to statically link `libheif` into a Xberg build.
- **Redistribution:** the Linux CLI archives and the `full`/`core` images ship
  unmodified `libheif` shared objects separately from the Xberg executable.
  The glibc builds use v1.23.0 from the official release tarball; musl builds
  use Alpine's shared package. The library remains replaceable to satisfy LGPL
  §6. Upstream source: <https://github.com/strukturag/libheif>.

## libheif codec libraries

Depending on how `libheif` was built, its codec backends are linked shared
libraries or dynamically-loaded plugins. Linux CLI packaging vendors the
non-system shared-library closure required by its `libheif` build; container
images install the same codecs from the distro package manager. Each remains a
separate, replaceable shared object and retains its upstream license:

| Library  | Role            | License (upstream)          |
| -------- | --------------- | --------------------------- |
| libde265 | HEVC **decode** | LGPL-3.0-or-later           |
| libdav1d | AV1 **decode**  | BSD-2-Clause                |
| libaom   | AV1 dec/enc     | BSD-2-Clause + patent grant |
| libx265  | HEVC **encode** | **GPL-2.0-or-later**        |

Xberg supports both HEIF-family input decoding and optional HEIF image output.
`libx265` is needed only for the latter and is redistributed only when the
platform's `libheif` build links it.

## libwpd + librevenge (WordPerfect) — MPL-2.0

- **Feature:** the `xberg-libwpd` crate, consumed by `xberg`'s `wordperfect`
  feature (included in `formats`, `full` and `windows-target`). Desktop only
  (Linux, macOS and Windows).
- **License:** libwpd and librevenge are dual-licensed **MPL-2.0 OR
  LGPL-2.1+**. Xberg builds and links them under the **MPL-2.0** arm.
- **Linking:** **Static, from source.** `xberg-libwpd` vendors the upstream
  librevenge and libwpd release tarballs as committed `.tar.gz` archives under
  `crates/xberg-libwpd/vendor/`; its build script decompresses them
  (checksum-verified at extract time), compiles them from source, and
  statically links them together with a small C++ shim. MPL-2.0 is file-level
  (weak) copyleft and explicitly permits static linking into a larger,
  differently-licensed work; obligations are limited to keeping the
  libwpd/librevenge source and any modifications to their files available under
  MPL-2.0 and preserving license notices. No copyleft reaches Xberg's MIT core
  or the language bindings. The LGPL arm is intentionally not used, as LGPL
  static linking would impose relink/object-file obligations on redistributed
  binaries.
- **Modifications:** none; both libraries are built from unmodified upstream
  source (librevenge 0.0.6, libwpd 0.10.3). Upstream source:
  <https://libwpd.sourceforge.net/>.
- **Boost (build-time headers) — BSL-1.0:** the C++ shim compiles against a
  `bcp`-extracted header-only subset of the Boost C++ Libraries, vendored as
  `crates/xberg-libwpd/vendor/boost-subset.tar.gz` and decompressed at build
  time. Boost is licensed under the **Boost Software License 1.0** (BSL-1.0), a
  permissive license whose only obligation is preserving the license text; no
  copyleft or attribution-in-binary obligation reaches Xberg. Boost headers are
  a build-time dependency only — no Boost code is redistributed in Xberg binaries.

## ONNX Runtime (OCR / ML features)

- **Feature:** optional (`paddle-ocr`, `layout-detection`, `embeddings`,
  `reranker`, `auto-rotate`, transcription). License: MIT. Linked dynamically
  (system `libonnxruntime.so`) in the musl/container builds; bundled per the
  `ort-bundled` feature (official Microsoft binaries) otherwise.

## Tesseract OCR + Leptonica

- **Feature:** `xberg-tesseract` builds Tesseract 5.5.3 at commit
  `db0ec62f81b0737fbbe184d8fea40af5738f8eef` and Leptonica 1.87.0 at commit
  `13275a278eb55b5746e33f95fbf5a2c8f604b3ab` when `build-tesseract` or
  `build-tesseract-wasm` is enabled.
- **Licenses:** Tesseract is Apache-2.0; the exact upstream text is preserved in
  [`crates/xberg-tesseract/licenses/tesseract-LICENSE`](crates/xberg-tesseract/licenses/tesseract-LICENSE).
  Leptonica is BSD-2-Clause; the exact upstream notice is preserved in
  [`crates/xberg-tesseract/licenses/leptonica-LICENSE`](crates/xberg-tesseract/licenses/leptonica-LICENSE).
- **Linking:** both native libraries are built from their checksum-verified
  source archives and statically linked into the consuming artifact. Their
  permissive licenses permit static linking when the license texts and notices
  accompany redistribution.
- **Model redistribution:** `eng.traineddata` is redistributed from
  `tessdata_fast` commit `87416418657359cb625c412a48b6e1d6d41c29bd` after
  checksum verification. `tessdata_fast` uses the same Apache-2.0 license text
  as Tesseract, so the preserved Tesseract license covers the model artifact.
