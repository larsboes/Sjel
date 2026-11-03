//! Rung 1 of the ladder: PDFs that already carry a text layer.

use crate::{cap, Document, Extraction, ExtractionError, Extractor, InputClass, Result};

/// PDF, through [xberg](https://github.com/xberg-io/xberg) (`=1.3.10`).
///
/// Registered for `Pdf` only. xberg reads HTML too and returns more text than
/// `Builtin` on every URL in the recorded benchmark, but it is an extractor and
/// not a readability cleaner: its output keeps navigation and share widgets,
/// and the short-line share runs 0.03-1.00. Swapping the HTML path to it is a
/// separate change with a scorecard already in place, the corpus gate's `html`
/// class, rather than a side effect of wanting PDFs.
///
/// **Its PDF output is last-resort quality, measured rather than assumed.** On
/// arXiv 2608.02599 (2026, two-column) the text is correct but woven column
/// against column, line by line, so prose order is lost. On arXiv 0704.0001
/// (2007, Type1 fonts) every `c` is dropped: "Abstrat", "Mihigan", "quantum
/// hromo dynamis". Both took 2.4s and 11.7s respectively, against ~0.3s for
/// the same papers as HTML.
///
/// That is why the PDF rung sits below both HTML hosts in
/// `comms::media::fetch_arxiv` rather than replacing them, and why `producer`
/// is stored per item: an embedding built from scrambled columns is not the
/// same evidence as one built from LaTeXML output, and nothing downstream can
/// tell them apart without being told.
pub struct Xberg;

impl Extractor for Xberg {
    fn name(&self) -> &'static str {
        "xberg-1.3.10"
    }

    fn handles(&self, class: InputClass) -> bool {
        matches!(class, InputClass::Pdf)
    }

    fn extract(&self, doc: &Document<'_>) -> Result<Extraction> {
        if !self.handles(doc.class) {
            return Err(ExtractionError::UnsupportedClass {
                extractor: self.name(),
                class: doc.class,
            });
        }
        let result = extract_blocking(doc.bytes.to_vec(), "application/pdf")?;
        let document = result.results.into_iter().next().ok_or_else(|| {
            let why = result
                .errors
                .first()
                .map(|e| e.message.clone())
                .unwrap_or_else(|| "no document and no error".into());
            ExtractionError::Engine {
                engine: "xberg",
                why: format!("returned nothing: {why}"),
            }
        })?;

        // OCR is folded into the producer rather than given a field of its own.
        // "which extractor produced this" is the question `producer` answers,
        // and text recovered from a scan is a different answer from text read
        // out of the file, for anything that later doubts the content.
        let producer: &'static str = match document.extraction_method {
            Some(::xberg::ExtractionMethod::Ocr) => "xberg-1.3.10/ocr",
            Some(::xberg::ExtractionMethod::Mixed) => "xberg-1.3.10/mixed",
            _ => self.name(),
        };

        let mut remaining = crate::TEXT_CAP;
        let pages = document
            .pages
            .unwrap_or_default()
            .into_iter()
            .map(|page| {
                let text: String = page.content.chars().take(remaining).collect();
                remaining = remaining.saturating_sub(text.chars().count());
                crate::ExtractedPage {
                    page_number: page.page_number,
                    text,
                    engine: producer.to_owned(),
                    failure: None,
                }
            })
            .collect();

        Ok(Extraction {
            title: document.metadata.title.filter(|t| !t.trim().is_empty()),
            text: cap(document.content),
            // xberg can emit Markdown, but this rung is not asked for it: the
            // consumer that needs layout (`capabilities/transit`) drives the
            // xberg CLI itself and asks for both formats there.
            markdown: None,
            producer,
            pages,
        })
    }
}

/// Run xberg's async API from a synchronous ingest path.
///
/// xberg is async; the callers are not. The `comms` CLI carries no runtime at
/// all by design (see that capability's README), and comms-server is already
/// inside one, so neither `Runtime::block_on` nor a bare `Handle` works for
/// both: building a runtime inside a runtime panics.
///
/// A thread that owns its own runtime is correct from either caller without
/// either having to know which it is. The cost is one thread per document,
/// which an ingest path can afford, and the alternative was making the trait
/// async and colouring every caller up to the CLI.
fn extract_blocking(bytes: Vec<u8>, mime: &'static str) -> Result<::xberg::ExtractionResult> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let outcome = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| ExtractionError::Engine {
                engine: "xberg",
                why: format!("runtime: {e}"),
            })
            .and_then(|runtime| {
                runtime.block_on(async {
                    let input = ::xberg::ExtractInput::from_bytes(bytes, mime, None);
                    let config = ::xberg::ExtractionConfig {
                        pages: Some(::xberg::PageConfig {
                            extract_pages: true,
                            ..::xberg::PageConfig::default()
                        }),
                        ..::xberg::ExtractionConfig::default()
                    };
                    ::xberg::extract(input, &config)
                        .await
                        .map_err(|e| ExtractionError::Engine {
                            engine: "xberg",
                            why: e.to_string(),
                        })
                })
            });
        let _ = tx.send(outcome);
    });
    rx.recv().map_err(|_| ExtractionError::Engine {
        engine: "xberg",
        why: "extraction thread died".into(),
    })?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TEXT_CAP;

    #[test]
    fn xberg_reads_a_real_pdf_and_says_it_produced_the_text() {
        // A genuine one-page PDF, built here rather than committed: the point
        // is that bytes in this shape come back as their text, and a binary
        // fixture would hide what is being asserted.
        let pdf = minimal_pdf("Transit data should remain inspectable.");
        let out = Xberg
            .extract(&Document::pdf(&pdf))
            .expect("a well-formed PDF must extract");

        assert!(
            out.text.contains("Transit data should remain inspectable"),
            "got: {:?}",
            out.text
        );
        assert!(
            out.producer.starts_with("xberg-1.3.10"),
            "producer must name the extractor: {}",
            out.producer
        );
        assert!(out.text.chars().count() <= TEXT_CAP);
    }

    #[test]
    fn a_pdf_that_is_not_a_pdf_is_an_error_rather_than_empty_text() {
        assert!(Xberg.extract(&Document::pdf(b"not a pdf at all")).is_err());
    }

    #[test]
    fn xberg_refuses_a_class_it_does_not_own() {
        // It does not return an empty string for a document it cannot read. An
        // empty body is indistinguishable from "this page had nothing", which
        // is what `content_status` means, and would be a lie here.
        let error = Xberg
            .extract(&Document::html(b"<p>hi</p>"))
            .expect_err("xberg is registered for pdf only");
        assert_eq!(error.to_string(), "xberg-1.3.10 does not read html input");
    }

    /// The smallest PDF that carries one line of extractable text.
    fn minimal_pdf(text: &str) -> Vec<u8> {
        let stream = format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET");
        let mut pdf = String::from("%PDF-1.4\n");
        let mut offsets = Vec::new();
        let objects = [
            "1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n".to_string(),
            "2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n".to_string(),
            "3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]\
             /Resources<</Font<</F1 4 0 R>>>>/Contents 5 0 R>>endobj\n"
                .to_string(),
            "4 0 obj<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>endobj\n".to_string(),
            format!(
                "5 0 obj<</Length {}>>stream\n{}\nendstream endobj\n",
                stream.len(),
                stream
            ),
        ];
        for object in &objects {
            offsets.push(pdf.len());
            pdf.push_str(object);
        }
        let xref_at = pdf.len();
        pdf.push_str(&format!(
            "xref\n0 {}\n0000000000 65535 f \n",
            objects.len() + 1
        ));
        for offset in &offsets {
            pdf.push_str(&format!("{offset:010} 00000 n \n"));
        }
        pdf.push_str(&format!(
            "trailer<</Size {}/Root 1 0 R>>\nstartxref\n{}\n%%EOF\n",
            objects.len() + 1,
            xref_at
        ));
        pdf.into_bytes()
    }
}
