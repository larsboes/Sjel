//! Turning a ticket file into text a parser can read.
//!
//! This exists because of one measured failure. A realistic DB confirmation
//! puts the journey in a table, and `pdf_extract` flattens that table into
//! lines: the header row `Datum Ab Bahnhof An Bahnhof Zug Gleis` becomes
//! indistinguishable from data, and the ticket parser duly read two legs
//! running from "Bahnhof" to "Bahnhof Zug Gleis" while reporting `ok: true`
//! with nothing missing. That is a text-layer bug, not an OCR one: better
//! pixels would not have helped, because the pixels were never the problem.
//!
//! So the fix is a reader that keeps a table a table. `xberg` can emit Markdown,
//! where a row stays a row, and it reads images besides. `builtin` is the
//! original path, kept because it needs no external binary.
//!
//! The two are not interchangeable and this deliberately does not pretend they
//! are. [`Extraction`] carries `markdown` as an `Option`, because only a
//! layout-aware backend can produce one, and a caller that needs structure has
//! to see its absence rather than be handed a flattened substitute.
//!
//! The RESULT type is `libs/extraction`'s, not this module's. It used to be a
//! local `Document` struct saying the same three things — text, optional
//! markdown, which reader produced it — under a second set of names, which is
//! what made this capability the second consumer that promoted that crate out
//! of `capabilities/comms` (CONTRIBUTING.md#schemas-and-dependency-direction). The
//! two readers below stay here: they are this capability's, they key off a
//! ticket's file extension, and which one runs is an operator setting rather
//! than a property of the input class.

use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

use sjel_extraction::Extraction;

use crate::config::{Config, DocumentBackend};

pub fn read(bytes: &[u8], file_name: &str, config: &Config) -> Result<Extraction, String> {
    match config.document_backend {
        DocumentBackend::Builtin => builtin(bytes, file_name),
        DocumentBackend::Xberg => xberg(bytes, file_name, &config.xberg_bin, &config.ocr_language),
    }
}

fn extension_of(file_name: &str) -> String {
    std::path::Path::new(file_name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
}

/// The original reader. No external dependency, no images, no layout.
fn builtin(bytes: &[u8], file_name: &str) -> Result<Extraction, String> {
    let text = match extension_of(file_name).as_str() {
        "pdf" => crate::extractor::extract_pdf_text(bytes)
            .map_err(|e| format!("PDF extraction failed: {e}"))?,
        "eml" => crate::extractor::extract_email_text(bytes)
            .map_err(|e| format!("Email extraction failed: {e}"))?,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" | "tiff" => {
            return Err(
                "the builtin reader cannot read an image. Set document_backend to \"xberg\" \
                 in the transit overlay config, or send a PDF or text file."
                    .to_string(),
            );
        }
        _ => String::from_utf8_lossy(bytes).to_string(),
    };
    Ok(Extraction {
        // A ticket file has no title of its own, and the confirmation's own
        // subject line is the caller's to keep, not this reader's to invent.
        title: None,
        text,
        markdown: None,
        producer: "builtin",
    })
}

/// A file handed to a subprocess has to exist somewhere.
///
/// Removed on drop, including on unwind: the alternative is a ticket
/// confirmation left in the system temp directory after a panic, and a ticket
/// carries a name and an order number.
struct TempFile(PathBuf);

impl TempFile {
    fn write(bytes: &[u8], extension: &str) -> Result<Self, String> {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let name = if extension.is_empty() {
            format!("axon-transit-{}-{unique:x}", std::process::id())
        } else {
            format!("axon-transit-{}-{unique:x}.{extension}", std::process::id())
        };
        let path = std::env::temp_dir().join(name);
        let mut file =
            std::fs::File::create(&path).map_err(|e| format!("temp file {path:?}: {e}"))?;
        file.write_all(bytes)
            .map_err(|e| format!("temp file {path:?}: {e}"))?;
        Ok(Self(path))
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Shells out to the xberg CLI.
///
/// A subprocess rather than the `xberg` crate, on purpose and for now. Linking
/// it would pull a bundled Tesseract and an ONNX runtime into this capability's
/// build, and the crate is seven weeks past 1.0; a process boundary keeps that
/// out of this capability's Cargo dependencies and makes it removable by editing
/// config.
/// If it earns its place, linking it is the next step, not a rewrite.
///
/// Arguments go as an array, never through a shell, so a filename cannot become
/// a command.
fn xberg(
    bytes: &[u8],
    file_name: &str,
    binary: &str,
    ocr_language: &str,
) -> Result<Extraction, String> {
    let temp = TempFile::write(bytes, &extension_of(file_name))?;

    let markdown = run_xberg(binary, &temp.0, "markdown", ocr_language)?;
    // Plain text as well as Markdown: the ticket parser's regexes run against
    // prose, and asking for both costs one more call on a file already cached
    // by the first.
    let text =
        run_xberg(binary, &temp.0, "plain", ocr_language).unwrap_or_else(|_| markdown.clone());

    Ok(Extraction {
        title: None,
        text,
        markdown: Some(markdown),
        producer: "xberg",
    })
}

fn run_xberg(
    binary: &str,
    path: &PathBuf,
    content_format: &str,
    ocr_language: &str,
) -> Result<String, String> {
    // The language matters more than it looks. Left at the default, Tesseract
    // reads a German ticket's umlauts as noise: a measured run turned "Züge"
    // into "Ztige" and "möglich" into "méglich".
    let output = Command::new(binary)
        .arg("extract")
        .arg("--format")
        .arg("json")
        .arg("--content-format")
        .arg(content_format)
        .arg("--ocr-language")
        .arg(ocr_language)
        .arg(path)
        .output()
        .map_err(|e| {
            format!(
                "could not run {binary:?} ({e}). Install it with \
                 `cargo install xberg-cli`, or set document_backend back to \"builtin\"."
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "{binary} exited {}: {}",
            output.status.code().unwrap_or(-1),
            stderr.trim()
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    Ok(content_from_json(&stdout).unwrap_or(stdout))
}

/// Pulls the content out of xberg's JSON envelope.
///
/// The real shape is `{"result": {"content": ...}}`; the published CLI example
/// shows a bare `{"content": ...}`. Both are accepted because a wrapper that
/// only handles the documented one breaks the first time the docs are the thing
/// that is out of date, which is what happened here.
///
/// Falls back to the raw stdout when neither matches, rather than failing: a
/// changed envelope should cost the structure, not the read.
fn content_from_json(stdout: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(stdout).ok()?;
    let content = |node: &serde_json::Value| {
        node.get("content")
            .and_then(|c| c.as_str())
            .map(str::to_string)
    };
    value
        .get("result")
        .and_then(content)
        .or_else(|| content(&value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_builtin_reader_says_why_it_cannot_read_an_image() {
        let error = builtin(b"\x89PNG\r\n", "ticket.png")
            .expect_err("builtin has no OCR and must not pretend otherwise");
        // The message names the fix, because "not supported" sends the caller
        // nowhere.
        assert!(error.contains("xberg"), "got: {error}");
    }

    #[test]
    fn a_plain_text_ticket_needs_no_backend_at_all() {
        let doc = builtin(b"Von: Bonn Hbf\nNach: Frankfurt(Main)Hbf\n", "t.txt").unwrap();
        assert!(doc.text.contains("Bonn Hbf"));
        // No layout was recovered, and the absence is the honest answer.
        assert!(doc.markdown.is_none());
        // The producer name is the same string the overlay's
        // `document_backend` setting uses, so a reply and a config file cannot
        // disagree about which reader ran.
        assert_eq!(doc.producer, "builtin");
    }

    #[test]
    fn xbergs_json_envelope_is_unwrapped_but_never_required() {
        // The shape xberg actually serves.
        assert_eq!(
            content_from_json("{\"result\":{\"content\":\"| a | b |\"},\"x\":1}").as_deref(),
            Some("| a | b |")
        );
        // And the shape its docs show.
        assert_eq!(
            content_from_json("{\"content\":\"# Ticket\\n\\n| a | b |\",\"metadata\":{}}")
                .as_deref(),
            Some("# Ticket\n\n| a | b |")
        );
        // A changed envelope costs the structure, not the read.
        assert_eq!(content_from_json("not json at all"), None);
        assert_eq!(content_from_json(r#"{"other":1}"#), None);
    }

    #[test]
    fn a_temp_file_is_removed_when_it_goes_out_of_scope() {
        let path = {
            let temp = TempFile::write(b"ticket bytes", "pdf").unwrap();
            assert!(temp.0.is_file());
            assert_eq!(temp.0.extension().and_then(|e| e.to_str()), Some("pdf"));
            temp.0.clone()
        };
        assert!(
            !path.is_file(),
            "a ticket carries a name and an order number; it must not outlive the read"
        );
    }
}
