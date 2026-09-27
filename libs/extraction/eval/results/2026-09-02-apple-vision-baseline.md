# Apple Vision — first run of the frozen DE/EN OCR corpus

**2026-09-02. Prose line PASS at 100.0%. Notation line MISS at 58.8%. Runner exit 1.**

Rung 2 of the extraction ladder is adopted on the prose line. Rung 3 stays empty, and this run
is why it exists (PRD Q63 → B30).

## What was run

| | |
|---|---|
| Engine | Apple Vision, `VNRecognizeTextRequest`, `.accurate`, `usesLanguageCorrection = true`, languages `["de-DE", "en-US"]` |
| Runner | `tools/visocr`, built by `tools/visocr/build.sh` from `tools/visocr/visocr.swift` |
| Corpus | `libs/extraction/eval/ocr-corpus.json`, unchanged acceptance |
| Fixtures | the six committed PNGs, rendered at 150 ppi by `fixtures/render.sh` under typst 0.15.1 (the `upstreams.toml [typst]` pin) |
| Host | macOS 26.6.2, arm64 |
| Cost | 0.86 s wall for all six pages in one process, no model bytes on disk, no network |

```sh
SJEL_VISOCR_BIN=target/tools/visocr \
  cargo run -p sjel-extraction --bin extraction-gate -- libs/extraction/eval/ocr-corpus.json
```

Verbatim output is preserved at `../recorded/apple-vision-2026-09-02.json` and is what
`libs/extraction/src/gate.rs` scores in `cargo test`, so this result stays checkable on a host
that cannot run this engine at all.

## Scorecard

```
  de-prose       prose  de      7/7   survived   detector quiet
  en-prose       prose  en      5/5   survived   detector quiet
  de-en-mixed    mixed  de+en   6/6   survived   detector quiet
  de-table       table  de     12/12  survived   detector quiet
  de-math        math   de      5/9   survived   detector fired
      ✗ did not survive: "q = 10 nC", "π", "∫", "∑"
  en-math        math   en      5/8   survived   detector fired
      ✗ did not survive: "∑", "√", "±"

  prose recall      100.0%   pass   (rung 2)
  notation           58.8%   MISS   (rung 3), 0 forbidden confusion(s)
  detector agrees   100.0%   pass
```

## What held

**German prose is perfect**, including every diacritic and every compound: `Fahrplanänderung`,
`Köln Hbf`, `München Hbf`, `geänderte Streckenführung`, `Ausweichgleis entfällt`. This confirms
the `upstreams.toml [auge]` characterisation on printed text, with no language pack and no
model bytes.

**Both languages on one page hold.** `de-en-mixed` returned all six judged strings across the
German and English halves, so the two-language request is not a compromise between them.

**Every table cell survived**, all twelve. The rows did not: the output interleaves
`Frankfurt(M) Hbf ICE 622` and `Frankfurt(M) Hbf 14:47` across the two legs, exactly the
column-major loss `[auge]` recorded. The corpus judges cells only for that reason, so this is a
pass on what was asked and a known loss on what was not.

## What failed, and it is not the failure that was expected

The recorded 2026-08-31 signature is `=` read as `-` (`q- 10nC`, `d-2am`). **That did not
happen here: zero forbidden confusions.** On *printed* notation the engine does something
different and worse to detect.

**It deletes displayed equations and returns the surrounding prose as though the page had
none.** `de-math` carries four display formulas. `E = F/q`, `W = ∫₀^d F ds` and `E_ges = Σᵢ Eᵢ`
came back as nothing at all — the recognized text runs straight from "Die Feldstärke im Abstand
d folgt aus der Definition" to "und mit dem Coulombgesetz". The Coulomb formula left the
fragments `1`, `E=`, `ATTEO`, `d2`. On `en-math`, `x̄ = (1/n)Σxᵢ` vanished and `SE = s/√n`
became `SE =` followed by `Vn`.

Not one operator survived on either page: no `∫`, no `∑`, no `√`, no `±`, no `π`.

One character confusion, reported as a plain miss because the corpus declares no rule for it:
the italic variable `q` was read as the digit `9`, so `q = 10 nC` came back as `9 = 10 nC`.

**A silent deletion is worse than a corruption**, and for the reason `[auge]` already gives
about wrong notation: a corruption leaves evidence in the text, and a deletion leaves a page
that reads as clean German prose about a formula that is not there.

## Two things this run changed, both recorded rather than done quietly

**1. The corpus judgements were revised, and made stricter.** The original `must_survive` lists
named only inline text, so they scored this page at 90.9% while every formula on it was gone.
The revision adds the operators the `.typ` sources carry. It is written into
`ocr-corpus.json` under `_judgement_revision`, acceptance thresholds untouched. Strengthening a
judgement from the source is corpus work; weakening one to admit an engine is what this gate
exists to prevent.

**2. The math detector gained a fifth signal.** `RelationWithNoRightHandSide` — a line whose
last non-space character is a relation sign, `E=` or `SE =`. Nothing the detector already had
could see this failure, because everything it had looks for a corruption and there was none to
find. With it, the detector fires on both notation pages and stays quiet on all four prose
pages: agreement 100.0%.

That number is the one that matters most in this run. A false positive here would send every
German article Axon reads to a rung that does not exist.

## Verdict

**Rung 2: adopted.** 100.0% prose recall on four pages across two languages, at 0.14 s per page,
with zero model bytes.

**Rung 3: still empty, and now evidence-backed.** 58.8% notation recall is not a threshold this
engine narrowly missed; it is a reader that returns none of a formula sheet's mathematics. The
`ocr` role in `libs/inference` stays undeclared until some engine clears this corpus unchanged.

**Not measurable here:** handwriting. Every fixture is printed. `upstreams.toml [dolphin]` is
unverified on handwriting by its own project's silence, and this corpus cannot settle it either
— a handwritten fixture is a named follow-up, and adding one invalidates nothing above because
it is a seventh page, not a re-render of these six.
