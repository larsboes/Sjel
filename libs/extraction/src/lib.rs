//! The seam between bytes off the wire and the text a consumer stores.
//!
//! Fetching stays per source and is not extraction: the GitHub API, Reddit's
//! JSON, yt-dlp, a plain GET and a ticket file dropped on an HTTP endpoint
//! speak five different protocols. What they all end up holding is a document
//! plus what kind of document it is, and turning that into text is one job —
//! one implementation per input class, behind this trait (#77).
//!
//! Extraction stops at "faithful bytes to text". `comms::normalize::normalize`
//! owns "clean" (#86). An extractor that stripped page furniture would be
//! making, silently, the judgement that stage exists to make inspectably.
//!
//! Two implementations, and which owns which class is policy. [`Builtin`] reads
//! HTML and plain text; [`Xberg`] reads PDF, the class nothing here could read
//! before (#77). HTML deliberately did not move to xberg: see [`Xberg`]'s own
//! note for what its PDF output actually looks like, which is also why that
//! rung sits last in `comms::media::fetch_arxiv`. Whichever route ran is
//! recorded per item as `transcript_source` and `producer` (#78).
//!
//! ## Why this is a lib and not a comms module
//!
//! It was one, until a second capability needed the same job.
//! `capabilities/transit` reads a ticket file into text plus optional Markdown
//! and had grown its own `Document`/`DocumentBackend` vocabulary for it — the
//! same job under a second set of names, already diverged. CONTRIBUTING.md#schemas-and-dependency-direction
//! promotes code to `libs/` at the second real consumer, provided it owns no
//! domain of its own, and this owns none: no store, no config, no HTTP client.
//! Fetching, normalization and what to do with an empty body all stay with the
//! caller (PRD Q63 → B30).
//!
//! ## The three rungs
//!
//! The ladder B30 describes is cost-ordered and first match wins, and for a
//! document that HAS a text layer that is [`for_class`]'s ordering and nothing
//! more. A page with no text layer is the one case that needs a walk instead of
//! a lookup, because whether the next rung is required depends on what the last
//! rung returned rather than on the input class: see [`ladder::read_scanned`].
//!
//! | Rung | Reader | Cost |
//! |---|---|---|
//! | 1 | [`Xberg`] — a PDF's own text layer | a linked crate, feature-gated |
//! | 2 | [`vision::VisionOcr`] — Apple Vision over `tools/visocr` | one subprocess, macOS only |
//! | 3 | [`ocr_role`] — the `ocr` role in `libs/inference` | a model, and no engine has cleared the gate |

use thiserror::Error;

// `self::` is load-bearing: this module has the same name as the crate it
// wraps, and a bare `xberg::` path at the crate root is ambiguous between the
// two (E0659).
#[cfg(feature = "xberg")]
mod xberg;
#[cfg(feature = "xberg")]
pub use self::xberg::Xberg;

mod builtin;
pub use builtin::{
    collapse_ws, decode_basic_entities, html_title, html_to_lines, looks_like_html, Builtin,
};

pub mod gate;
pub mod ladder;
pub mod math;
pub mod ocr_role;
pub mod vision;

/// What a document is, which is what decides who can read it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InputClass {
    Html,
    Pdf,
    PlainText,
    /// Pixels: a scan, a photographed page, a screenshot. Distinct from `Pdf`
    /// because the question a reader asks is different — a PDF may carry its
    /// own text and an image never does, so this class has no rung 1 at all.
    Image,
}

impl InputClass {
    pub fn as_str(self) -> &'static str {
        match self {
            InputClass::Html => "html",
            InputClass::Pdf => "pdf",
            InputClass::PlainText => "text",
            InputClass::Image => "image",
        }
    }
}

impl std::fmt::Display for InputClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Bytes plus what they are. Borrowed, because an extractor reads a document
/// and never owns one — the fetch that produced it owns its buffer.
pub struct Document<'a> {
    pub class: InputClass,
    pub bytes: &'a [u8],
}

impl<'a> Document<'a> {
    pub fn html(bytes: &'a [u8]) -> Self {
        Self {
            class: InputClass::Html,
            bytes,
        }
    }

    pub fn pdf(bytes: &'a [u8]) -> Self {
        Self {
            class: InputClass::Pdf,
            bytes,
        }
    }

    pub fn text(bytes: &'a [u8]) -> Self {
        Self {
            class: InputClass::PlainText,
            bytes,
        }
    }

    pub fn image(bytes: &'a [u8]) -> Self {
        Self {
            class: InputClass::Image,
            bytes,
        }
    }
}

/// What an extractor produced, and which one produced it.
///
/// `producer` is recorded rather than inferred: once there are two
/// implementations for the same class, "which one read this item" is the first
/// question a surprising result raises.
#[derive(Debug, Clone, PartialEq)]
pub struct Extraction {
    pub title: Option<String>,
    pub text: String,
    /// The same content with layout preserved, when the reader can do that.
    ///
    /// `None` is a real answer, not a missing one: it means no structure was
    /// recovered, and a caller must not treat `text` as though rows survived
    /// in it. This is `capabilities/transit`'s reason for existing at all — a
    /// DB confirmation puts the journey in a table, and a flattened table
    /// parses into two legs running from "Bahnhof" to "Bahnhof Zug Gleis"
    /// while reporting success.
    pub markdown: Option<String>,
    pub producer: &'static str,
}

/// What went wrong, at the granularity a ladder walker has to distinguish.
///
/// Two failures look alike from a call site and must not be treated alike.
/// [`UnsupportedClass`](ExtractionError::UnsupportedClass),
/// [`NoExtractor`](ExtractionError::NoExtractor) and
/// [`Unavailable`](ExtractionError::Unavailable) say "not here"; a rung above
/// may still read this document. [`Engine`](ExtractionError::Engine) says this
/// reader ran on THIS document and failed, which is a fact about the document.
///
/// That distinction is what [`ladder::read_scanned`] walks on: the first three
/// advance and the fourth stops. Collapsing them would let a real failure be
/// swallowed by a fallback.
///
/// None of them is ever an empty `Ok`. An empty body is indistinguishable from
/// "this page had nothing", which is what a consumer's `content_status` means,
/// and would be a lie here.
#[derive(Debug, Error)]
pub enum ExtractionError {
    /// This extractor is not registered for this class.
    #[error("{extractor} does not read {class} input")]
    UnsupportedClass {
        extractor: &'static str,
        class: InputClass,
    },
    /// Nothing in the registry reads this class.
    #[error("no extractor is registered for {class} input")]
    NoExtractor { class: InputClass },
    /// The reader exists but cannot run HERE: wrong operating system, tool not
    /// installed, no engine declared, or an input shape it refuses to read
    /// partially. Nothing about the document is being reported.
    #[error("{engine} is unavailable: {why}")]
    Unavailable { engine: &'static str, why: String },
    /// The reader ran on this document and failed on it.
    #[error("{engine}: {why}")]
    Engine { engine: &'static str, why: String },
}

pub type Result<T> = std::result::Result<T, ExtractionError>;

pub trait Extractor: Sync {
    fn name(&self) -> &'static str;
    fn handles(&self, class: InputClass) -> bool;
    fn extract(&self, doc: &Document<'_>) -> Result<Extraction>;
}

/// The extractor for an input class, or `None` when nothing here reads it.
///
/// `None` is an answer, not a failure: a caller that can degrade (arXiv to its
/// abstract) checks this and records what it did. A caller that cannot says so
/// with [`require`].
///
/// First match wins, so this order is policy rather than convenience.
/// [`Builtin`] keeps HTML: it is measured by the corpus gate
/// (`capabilities/comms/eval/`) and produces the line structure `normalize`
/// needs, and xberg is an extractor rather than a readability cleaner (#77).
/// xberg owns what nothing here could read.
///
/// The order is B30's cost order. `Builtin` costs a string scan, `Xberg` costs a
/// linked extractor, and `VisionOcr` costs a subprocess and an OS framework, so
/// it sits last — and it owns exactly one class, `Image`, because pixels are the
/// one input with no text layer to try first.
///
/// `Pdf` is deliberately NOT its second class. A PDF that defeated rung 1 needs
/// rasterizing before rung 2 can see it, `tools/visocr` renders page one of a
/// PDF and nothing else, and a registry entry here would turn that into a silent
/// first-page read. [`ladder::read_scanned`] is where a caller that has already
/// rasterized comes in.
///
/// A consumer that builds this crate without the `xberg` feature has no PDF
/// rung and gets `None` for that class. That is the same answer, for the same
/// reason: the rung is absent, and saying so is what lets the caller degrade.
pub fn for_class(class: InputClass) -> Option<&'static dyn Extractor> {
    // Written twice rather than cfg'd inside one literal, so the registry a
    // given build actually has is readable without evaluating an attribute.
    #[cfg(feature = "xberg")]
    const REGISTERED: &[&dyn Extractor] = &[&Builtin, &Xberg, &vision::VisionOcr];
    #[cfg(not(feature = "xberg"))]
    const REGISTERED: &[&dyn Extractor] = &[&Builtin, &vision::VisionOcr];

    REGISTERED
        .iter()
        .copied()
        .find(|extractor| extractor.handles(class))
}

/// [`for_class`], for a caller with no fallback to offer.
pub fn require(class: InputClass) -> Result<&'static dyn Extractor> {
    for_class(class).ok_or(ExtractionError::NoExtractor { class })
}

/// Max characters of extracted body kept. Extraction caps; it does not clean.
pub const TEXT_CAP: usize = 20_000;

/// Cap a body at [`TEXT_CAP`] characters (not bytes — the cap is about what a
/// summarizer can read, and a multibyte split would corrupt the text).
pub fn cap(s: String) -> String {
    if s.chars().count() <= TEXT_CAP {
        s
    } else {
        s.chars().take(TEXT_CAP).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_class_has_exactly_the_reader_the_registry_says_it_does() {
        // Which extractor owns which class is policy, not an accident of
        // ordering, so it is asserted by name. `Builtin` keeping HTML is the
        // part most likely to be changed by someone who assumes registering
        // xberg meant handing it everything.
        assert_eq!(for_class(InputClass::Html).unwrap().name(), "builtin");
        assert_eq!(for_class(InputClass::PlainText).unwrap().name(), "builtin");
        // Pixels have no text layer to read, so the image class starts at rung
        // 2 and there is nothing above it in the registry.
        assert_eq!(for_class(InputClass::Image).unwrap().name(), "apple-vision");
    }

    #[cfg(feature = "xberg")]
    #[test]
    fn the_pdf_rung_is_present_exactly_when_its_feature_is() {
        assert_eq!(for_class(InputClass::Pdf).unwrap().name(), "xberg-1.0.5");
        assert!(require(InputClass::Pdf).is_ok());
    }

    #[cfg(not(feature = "xberg"))]
    #[test]
    fn the_pdf_rung_is_absent_rather_than_silently_empty_without_its_feature() {
        // A consumer that opted out of the dependency must be told the rung is
        // missing, not handed an empty body for every PDF. Rung 2 does not
        // cover for it either: pixels and a text layer are different questions.
        assert!(for_class(InputClass::Pdf).is_none());
        assert!(require(InputClass::Pdf).is_err());
    }

    #[test]
    fn a_class_nothing_reads_names_the_class_rather_than_saying_no() {
        let message = ExtractionError::NoExtractor {
            class: InputClass::Pdf,
        }
        .to_string();
        assert_eq!(message, "no extractor is registered for pdf input");
    }

    #[test]
    fn the_cap_counts_characters_so_a_multibyte_body_is_not_split_mid_glyph() {
        let long = "ü".repeat(TEXT_CAP + 100);
        let capped = cap(long);
        assert_eq!(capped.chars().count(), TEXT_CAP);
    }
}
