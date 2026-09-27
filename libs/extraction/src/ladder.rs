//! Walking rung 2 to rung 3, which is the only place the two meet.
//!
//! `for_class` is the whole ladder for a document that has a text layer: one
//! reader per class, cost-ordered, first match wins. A page with NO text layer
//! is the case that needs a walk instead of a lookup, because whether the next
//! rung is required depends on what the previous rung returned — not on the
//! input class, which is identical either way.
//!
//! The rule, from PRD Q63 -> B30 and from the measurement behind it
//! (`upstreams.toml [auge]`):
//!
//! ```text
//! rung 2 (Apple Vision)
//!   ├─ Unavailable ────────────────────────────────► rung 3
//!   ├─ Engine failure ─────────────────────────────► stop, report it
//!   └─ Ok(text) → math detector
//!                   ├─ quiet ──────────────────────► stop, keep the text
//!                   └─ fires ──────────────────────► rung 3
//! ```
//!
//! `Unavailable` advances and `Engine` stops, which is the distinction
//! `ExtractionError` exists to make: "no rung here" is not "this rung read this
//! document and failed".

use sjel_inference::ResolvedRole;

use crate::vision::VisionOcr;
use crate::{math, ocr_role, Document, Extraction, ExtractionError, Extractor, Result};

/// Read a page that carries no text layer.
///
/// `role` is the deployment's `ocr` role, looked up by the caller — this crate
/// never loads configuration. `None` is the ordinary state today and is not an
/// error by itself; it only becomes one on a page that needs rung 3.
pub fn read_scanned(doc: &Document<'_>, role: Option<&ResolvedRole>) -> Result<Extraction> {
    walk(VisionOcr.extract(doc), doc, role, ocr_role::read)
}

/// What rung 3 is, as a signature: the role the caller resolved, and the page.
///
/// Taken as a parameter rather than called by name so a test can hand in a
/// recording stub and assert what rung 3 actually received. Without that seam,
/// the whole hand-off is invisible — [`ocr_role::read`] discards `doc` today, so
/// passing it the wrong bytes compiles, runs and passes every assertion.
trait Rung3 {
    fn read(self, role: Option<&ResolvedRole>, doc: &Document<'_>) -> Result<Extraction>;
}

impl<F> Rung3 for F
where
    F: FnOnce(Option<&ResolvedRole>, &Document<'_>) -> Result<Extraction>,
{
    fn read(self, role: Option<&ResolvedRole>, doc: &Document<'_>) -> Result<Extraction> {
        self(role, doc)
    }
}

/// [`read_scanned`] with rung 2's answer handed in.
///
/// Split out so the WALK can be tested without an OCR engine, a subprocess or
/// macOS. Every branch below is a policy decision, and a policy that can only be
/// exercised on one operating system is a policy nobody reviews.
fn walk<R: Rung3>(
    rung2: Result<Extraction>,
    doc: &Document<'_>,
    role: Option<&ResolvedRole>,
    rung3: R,
) -> Result<Extraction> {
    match rung2 {
        Ok(extraction) => escalate_with(doc, extraction, role, rung3),
        // The rung is not here: wrong operating system, tool not installed. A
        // rung above may still read this document.
        Err(unavailable @ ExtractionError::Unavailable { .. }) => rung3
            .read(role, doc)
            // Both rungs declined. The message keeps both sentences, because
            // "macOS only" and "no engine adopted" are two different repairs and
            // a reader told only the second one goes looking for the wrong
            // thing.
            .map_err(|above| because(above, format!("{unavailable}; and "))),
        Err(other) => Err(other),
    }
}

/// Decide what rung 2's output is worth, and reach rung 3 when it is not enough.
///
/// Separate from [`read_scanned`] because this half is pure: it is the rule, and
/// a rule that can only be exercised by running an OCR engine on macOS is a rule
/// nobody can test. Every branch below is covered without a binary.
///
/// When the detector fires and rung 3 cannot answer, this returns rung 3's
/// error rather than rung 2's text. That is the deliberate, expensive choice:
/// the text exists and it is readable, and handing it back would store a page of
/// notation that upstreams.toml already calls "plausible-looking wrong text
/// rather than a failure -- which is worse than an error". A caller that wants
/// the text anyway can call [`VisionOcr`] directly and record what it did.
///
/// `doc` is the ORIGINAL page, and rung 3 gets exactly it. Rung 3 is an OCR
/// engine: handing it rung 2's text as though the characters were pixels would
/// re-read the corruption instead of the page the corruption came from, on
/// precisely the document this rung exists to rescue.
pub fn escalate(
    doc: &Document<'_>,
    rung2: Extraction,
    role: Option<&ResolvedRole>,
) -> Result<Extraction> {
    escalate_with(doc, rung2, role, ocr_role::read)
}

fn escalate_with<R: Rung3>(
    doc: &Document<'_>,
    rung2: Extraction,
    role: Option<&ResolvedRole>,
    rung3: R,
) -> Result<Extraction> {
    let suspicion = math::inspect(&rung2.text);
    if !suspicion.fires() {
        return Ok(rung2);
    }
    rung3.read(role, doc).map_err(|above| {
        because(
            above,
            format!(
                "{} read this page as notation it cannot express ({}), and ",
                rung2.producer,
                reasons(&suspicion)
            ),
        )
    })
}

fn reasons(suspicion: &math::MathSuspicion) -> String {
    suspicion
        .reasons
        .iter()
        .map(|reason| format!("{reason:?}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Put rung 2's reason in front of rung 3's error WITHOUT changing what kind of
/// failure rung 3 reported.
///
/// [`ExtractionError`]'s own doc comment is what this preserves: `Engine` says
/// this reader ran on THIS document and failed, and the three "not here"
/// variants say a rung above may still read it. Rung 3 is the top rung, so
/// nothing above it can act on that distinction — but a caller can, and
/// rewriting a real failure as an absent rung sends it looking for a model to
/// install instead of at the page. Only the "not here" variants collapse into
/// [`ExtractionError::Unavailable`], which is the sentence they already say.
fn because(above: ExtractionError, context: String) -> ExtractionError {
    match above {
        ExtractionError::Engine { engine, why } => ExtractionError::Engine {
            engine,
            why: format!("{context}{why}"),
        },
        absent => ExtractionError::Unavailable {
            engine: ocr_role::ENGINE,
            why: format!("{context}{absent}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::cell::RefCell;

    fn vision_output(text: &str) -> Extraction {
        Extraction {
            title: None,
            text: text.to_string(),
            markdown: None,
            producer: crate::vision::ENGINE,
        }
    }

    /// The page a rung-2 read came from. Nothing here reads the bytes, so the
    /// content only has to be distinguishable from rung 2's text.
    fn scan() -> Document<'static> {
        Document::image(b"\x89PNG\r\n\x1a\nthe original pixels")
    }

    /// A rung 3 that records what it was handed and then declines, so the
    /// hand-off is an assertion rather than a hope. [`ocr_role::read`] discards
    /// its `doc`, which means the wiring is otherwise unobservable.
    fn recorder<'a>(
        seen: &'a RefCell<Option<(Vec<u8>, crate::InputClass)>>,
    ) -> impl FnOnce(Option<&ResolvedRole>, &Document<'_>) -> Result<Extraction> + 'a {
        move |_role, doc| {
            *seen.borrow_mut() = Some((doc.bytes.to_vec(), doc.class));
            Err(ExtractionError::Unavailable {
                engine: ocr_role::ENGINE,
                why: "recorded".into(),
            })
        }
    }

    #[test]
    fn rung_three_is_handed_the_page_and_never_rung_twos_text() {
        // Rung 3 is an OCR engine. Given the UTF-8 of rung 2's output labelled
        // as an image, it would re-read the corruption instead of the page the
        // corruption came from -- on exactly the document rung 3 exists for.
        let seen = RefCell::new(None);
        let page = scan();
        let _ = escalate_with(
            &page,
            vision_output("q- 10nC\nd-2am\nE F q"),
            None,
            recorder(&seen),
        );
        let (bytes, class) = seen.into_inner().expect("rung three was reached");
        assert_eq!(bytes, page.bytes, "rung three read something else");
        assert_eq!(class, crate::InputClass::Image);
    }

    #[test]
    fn the_walk_hands_rung_three_the_same_page_when_rung_two_is_absent() {
        // The other route to rung 3, asserted separately: one caller passing the
        // page and the other passing a substitute is the defect this pair
        // catches.
        let seen = RefCell::new(None);
        let page = scan();
        let absent = Err(ExtractionError::Unavailable {
            engine: crate::vision::ENGINE,
            why: "Apple Vision is a macOS framework and this host runs linux".into(),
        });
        let _ = walk(absent, &page, None, recorder(&seen));
        let (bytes, class) = seen.into_inner().expect("rung three was reached");
        assert_eq!(bytes, page.bytes);
        assert_eq!(class, crate::InputClass::Image);
    }

    #[test]
    fn a_rung_three_engine_failure_stays_an_engine_failure_through_both_routes() {
        // ExtractionError's ruling, one level up from where it is documented:
        // "this reader ran on THIS document and failed" must not come back out
        // as "that rung is not here". A caller told the wrong one goes looking
        // for a model to install instead of at the page.
        let failed = |_role: Option<&ResolvedRole>, _doc: &Document<'_>| {
            Err(ExtractionError::Engine {
                engine: "some-adopted-ocr",
                why: "the page decoded but recognition returned nothing".into(),
            })
        };

        let escalated = escalate_with(
            &scan(),
            vision_output("q- 10nC\nd-2am\nE F q"),
            None,
            failed,
        )
        .expect_err("rung three failed on this page");
        assert!(
            matches!(escalated, ExtractionError::Engine { .. }),
            "{escalated:?}"
        );
        assert!(escalated.to_string().contains("recognition returned"));
        // Rung 2's reason survives in front of it rather than replacing it.
        assert!(escalated.to_string().contains("RelationReadAsHyphen"));

        let absent = Err(ExtractionError::Unavailable {
            engine: crate::vision::ENGINE,
            why: "Apple Vision is a macOS framework and this host runs linux".into(),
        });
        let walked = walk(absent, &scan(), None, failed).expect_err("rung three failed");
        assert!(
            matches!(walked, ExtractionError::Engine { .. }),
            "{walked:?}"
        );
        assert!(walked.to_string().contains("macOS framework"), "{walked}");
    }

    #[test]
    fn prose_that_read_cleanly_stops_at_rung_two() {
        let read = vision_output(
            "Ab dem kommenden Fahrplanwechsel fahren die Züge zwischen Köln Hbf und \
             München Hbf über eine geänderte Streckenführung.",
        );
        let out = escalate(&scan(), read.clone(), None).expect("prose never needs rung three");
        assert_eq!(out, read);
    }

    #[test]
    fn notation_read_as_hyphens_reaches_for_rung_three_and_reports_it_missing() {
        // The recorded signature, end to end through the rule.
        let error = escalate(&scan(), vision_output("q- 10nC\nd-2am\nE F q"), None)
            .expect_err("no engine has cleared the corpus gate");
        assert!(
            matches!(error, ExtractionError::Unavailable { .. }),
            "{error:?}"
        );
        assert!(
            error.to_string().contains("RelationReadAsHyphen"),
            "{error}"
        );
        assert!(error.to_string().contains("apple-vision"), "{error}");
    }

    #[test]
    fn a_page_the_detector_flags_is_never_returned_as_clean_text() {
        // Stated as its own test because it is the expensive half of the
        // ruling: the text was readable and is deliberately withheld.
        assert!(escalate(&scan(), vision_output("q- 10nC\nd-2am\nE F q"), None).is_err());
    }

    #[test]
    fn an_engine_failure_on_the_page_stops_the_walk_instead_of_advancing_it() {
        // A rung that RAN and failed states a fact about the document. Falling
        // through to rung 3 would swallow it and report the wrong repair.
        let failed = Err(ExtractionError::Engine {
            engine: crate::vision::ENGINE,
            why: "recognized no text on this page".into(),
        });
        let error = walk(failed, &scan(), None, ocr_role::read).expect_err("rung 2 failed");
        assert!(matches!(error, ExtractionError::Engine { .. }), "{error:?}");
        assert!(error.to_string().contains("recognized no text"), "{error}");
    }

    #[test]
    fn a_missing_rung_two_advances_to_rung_three_and_keeps_both_reasons() {
        // "Vision is macOS-only" and "no engine cleared the gate" are two
        // different repairs. A reader told only the second goes looking for the
        // wrong thing.
        let absent = Err(ExtractionError::Unavailable {
            engine: crate::vision::ENGINE,
            why: "Apple Vision is a macOS framework and this host runs linux".into(),
        });
        let error =
            walk(absent, &scan(), None, ocr_role::read).expect_err("neither rung can read it");
        let message = error.to_string();
        assert!(message.contains("macOS framework"), "{message}");
        assert!(message.contains("no \"ocr\" inference role"), "{message}");
    }

    #[test]
    fn an_unsupported_class_is_reported_rather_than_walked_around() {
        let error = read_scanned(&Document::html(b"<p>not pixels</p>"), None)
            .expect_err("html is not this ladder's input");
        assert!(
            matches!(error, ExtractionError::UnsupportedClass { .. }),
            "{error:?}"
        );
    }
}
