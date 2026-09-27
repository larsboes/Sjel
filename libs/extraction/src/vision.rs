//! Rung 2 of the ladder: pages with no text layer, read by Apple Vision.
//!
//! Rung 1 reads a PDF that already carries its text. This rung is what happens
//! when there is none — a photographed page, a screenshot, a scan. It shells
//! out to `visocr`, the batch reader adopted at `tools/visocr/visocr.swift`,
//! which wraps `VNRecognizeTextRequest` over the OS framework
//! (`systems.toml [apple-vision]`).
//!
//! A subprocess and not a linked framework, for the reason `capabilities/transit`
//! already gives for xberg: a process boundary keeps a platform SDK out of every
//! consumer's Cargo build, and this crate is linked by two capabilities that must
//! keep compiling on a machine that has never seen Xcode. Arguments and paths go
//! over a pipe, never through a shell, so a filename cannot become a command.
//!
//! ## What this rung is good at, measured rather than assumed
//!
//! `upstreams.toml [auge]` carries both measurements of this engine, and
//! `eval/results/2026-09-02-apple-vision-baseline.md` is this crate's own run
//! over the frozen corpus: **100.0% prose recall, 58.8% notation**. Excellent on
//! printed German and English prose with no language pack and no model bytes,
//! and unable to return a formula — on a printed page it does not corrupt a
//! displayed equation, it deletes it and hands back the surrounding prose as
//! though the page had none.
//!
//! That second half is not a reason to leave the rung out; it is the reason rung
//! 3 exists, and [`crate::math`] is what decides between them.
//!
//! ## Absent rather than erased
//!
//! Vision is macOS-only, so on a Linux host this rung does not exist. That is
//! reported at RUNTIME as [`ExtractionError::Unavailable`], naming the OS,
//! rather than compiled away behind `cfg(target_os)`. Two reasons: this
//! repository guards platform-specific pieces by declaration and runtime check
//! rather than by attribute (`toolchain.toml`'s `os` field,
//! `tools/lib/platform.sh`), and a rung erased at compile time cannot say why it
//! is not there. A ladder walker needs that sentence to distinguish "no rung
//! here" from "this rung failed on this document".

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::{cap, Document, Extraction, ExtractionError, Extractor, InputClass, Result};

/// What this rung records as `producer`, and what it calls itself in an error.
pub const ENGINE: &str = "apple-vision";

/// Where to find the reader. Mirrors `capabilities/transit`'s `xberg_bin`: a
/// bare name resolved on `PATH`, overridable for a build that is not installed.
const BINARY_ENV: &str = "SJEL_VISOCR_BIN";
const DEFAULT_BINARY: &str = "visocr";

/// Record and field separators of the `visocr` batch protocol. See that tool's
/// header comment; recognized text contains newlines and tabs, so the framing
/// cannot use either.
const RECORD_SEPARATOR: char = '\u{1e}';
const UNIT_SEPARATOR: char = '\u{1f}';

/// Apple Vision, through `tools/visocr`.
///
/// Registered for [`InputClass::Image`] and for nothing else. A PDF is
/// deliberately not this rung's input, even a scanned one:
/// `NSImage(contentsOfFile:)` renders page one of a PDF and nothing else
/// (`tools/visocr/visocr.swift`), so accepting the class would mean returning
/// one page's text under a `producer` claiming the document was read. Axon has
/// no rasterizer, and a silent truncation is the exact failure this crate exists
/// to prevent, so the honest boundary is that rung 2 reads pixels. Rasterizing a
/// PDF into pages is a named follow-up, not something to fake here.
pub struct VisionOcr;

impl Extractor for VisionOcr {
    fn name(&self) -> &'static str {
        ENGINE
    }

    fn handles(&self, class: InputClass) -> bool {
        matches!(class, InputClass::Image)
    }

    fn extract(&self, doc: &Document<'_>) -> Result<Extraction> {
        if !self.handles(doc.class) {
            return Err(ExtractionError::UnsupportedClass {
                extractor: self.name(),
                class: doc.class,
            });
        }

        let temp = TempFile::write(doc.bytes)?;
        let mut read = recognize(std::slice::from_ref(&temp.0))?;
        let (_, text) = read.pop().ok_or_else(|| ExtractionError::Engine {
            engine: ENGINE,
            why: "visocr returned no record for the page".into(),
        })?;

        // Never `Ok("")`. An empty record means either "this file would not
        // load" or "this page had no text", and visocr cannot tell them apart
        // (its header says so). Both are this rung failing on this document,
        // and an empty body stored as success is indistinguishable from a page
        // that genuinely had nothing.
        if text.trim().is_empty() {
            return Err(ExtractionError::Engine {
                engine: ENGINE,
                why: "recognized no text on this page".into(),
            });
        }

        Ok(Extraction {
            // A photographed page has no title of its own, and inventing one
            // from the first recognized line would be a judgement, not a read.
            title: None,
            text: cap(text),
            // Vision returns text observations, not layout. `upstreams.toml
            // [auge]` records what that costs on a table: the rows come back
            // column-major and interleaved. `None` is the honest answer, and
            // the caller has to see it rather than be handed a flattened
            // substitute.
            markdown: None,
            producer: ENGINE,
        })
    }
}

/// Read many pages in one process.
///
/// This is the shape `tools/visocr` exists for: Vision loads its language assets
/// once per process, so a per-page fork pays that cost per page. The corpus gate
/// reads six pages with one child.
///
/// Returns each path with the text `visocr` produced for it, IN INPUT ORDER, and
/// deliberately does not judge an empty result — a caller scoring an engine
/// needs to see the empty string, while [`VisionOcr::extract`] turns it into an
/// error for a caller that would otherwise store it.
pub fn recognize(paths: &[PathBuf]) -> Result<Vec<(PathBuf, String)>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    if std::env::consts::OS != "macos" {
        return Err(ExtractionError::Unavailable {
            engine: ENGINE,
            why: format!(
                "Apple Vision is a macOS framework and this host runs {}. \
                 Rung 2 is absent here; see upstreams.toml [ocrs] for the case that \
                 would fill it.",
                std::env::consts::OS
            ),
        });
    }

    let binary = std::env::var(BINARY_ENV).unwrap_or_else(|_| DEFAULT_BINARY.to_string());
    let mut child = Command::new(&binary)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| ExtractionError::Unavailable {
            engine: ENGINE,
            why: format!(
                "could not run {binary:?} ({e}). Build it with tools/visocr/build.sh \
                 --install <dir on PATH>, or point {BINARY_ENV} at the built binary."
            ),
        })?;

    // The paths are written on their OWN thread while this one drains the
    // child, and that is a correctness requirement rather than a speed-up.
    // `tools/visocr` prints and flushes one record per input as it reads, so
    // its stdout fills while the batch is still being written. One thread doing
    // both writes deadlocks a large enough batch: the child blocks in `print`
    // on a full stdout pipe, stops calling `readLine`, and the parent then
    // blocks on a full stdin pipe waiting for a reader that will not return.
    // `wait_with_output` reads stdout and stderr together, so this thread is
    // the drain.
    let mut stdin = child.stdin.take().ok_or_else(|| ExtractionError::Engine {
        engine: ENGINE,
        why: "visocr stdin was not piped".into(),
    })?;
    let lines: Vec<String> = paths
        .iter()
        .map(|path| path.display().to_string())
        .collect();
    let writer = std::thread::spawn(move || -> std::io::Result<()> {
        for line in &lines {
            writeln!(stdin, "{line}")?;
        }
        stdin.flush()
        // stdin drops here, which is what ends visocr's read loop.
    });

    let output = child
        .wait_with_output()
        .map_err(|e| ExtractionError::Engine {
            engine: ENGINE,
            why: format!("waiting for visocr: {e}"),
        })?;
    // Joined after the wait, and reported after the exit status: a child that
    // died early breaks this pipe, and "visocr exited 1: <stderr>" says why
    // while "broken pipe" only says that it did.
    let written = writer.join().map_err(|_| ExtractionError::Engine {
        engine: ENGINE,
        why: "the thread writing paths to visocr panicked".into(),
    })?;
    if !output.status.success() {
        return Err(ExtractionError::Engine {
            engine: ENGINE,
            why: format!(
                "visocr exited {}: {}",
                output.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        });
    }
    written.map_err(|e| ExtractionError::Engine {
        engine: ENGINE,
        why: format!("writing paths to visocr: {e}"),
    })?;

    let records = parse_records(&String::from_utf8_lossy(&output.stdout));
    paths
        .iter()
        .map(|path| {
            let wanted = path.display().to_string();
            records
                .iter()
                .find(|(recorded, _)| *recorded == wanted)
                .map(|(_, text)| (path.clone(), text.clone()))
                .ok_or_else(|| ExtractionError::Engine {
                    engine: ENGINE,
                    why: format!("visocr returned no record for {wanted}"),
                })
        })
        .collect()
}

/// Split `\x1e<path>\x1f<text>` records out of visocr's stdout.
///
/// Separate from the process call so the protocol is testable without macOS,
/// which is the only part of this module a Linux `cargo test` can reach.
fn parse_records(stdout: &str) -> Vec<(String, String)> {
    stdout
        .split(RECORD_SEPARATOR)
        .filter(|record| !record.is_empty())
        .filter_map(|record| {
            let (path, text) = record.split_once(UNIT_SEPARATOR)?;
            // visocr writes one record per line, so the trailing newline
            // belongs to the framing and not to the recognized text.
            Some((path.to_string(), text.trim_end_matches('\n').to_string()))
        })
        .collect()
}

/// A file handed to a subprocess has to exist somewhere.
///
/// Removed on drop, including on unwind. The same reasoning as
/// `capabilities/transit`'s copy: the bytes are somebody's document, and a
/// panic must not leave it in the system temp directory.
struct TempFile(PathBuf);

impl TempFile {
    fn write(bytes: &[u8]) -> Result<Self> {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let path = std::env::temp_dir().join(format!(
            "axon-visocr-{}-{unique:x}.{}",
            std::process::id(),
            extension_for(bytes)
        ));
        std::fs::write(&path, bytes).map_err(|e| ExtractionError::Engine {
            engine: ENGINE,
            why: format!("temp file {}: {e}", path.display()),
        })?;
        Ok(Self(path))
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// The extension to give the temp file, from the bytes themselves.
///
/// `NSImage(contentsOfFile:)` sniffs content, so this is belt and braces — but a
/// temp file named for what it holds is also what makes a leftover
/// identifiable, and `bin` is a truthful answer for bytes nothing here
/// recognizes.
fn extension_for(bytes: &[u8]) -> &'static str {
    match bytes {
        b if b.starts_with(b"\x89PNG\r\n\x1a\n") => "png",
        b if b.starts_with(b"\xff\xd8\xff") => "jpg",
        b if b.starts_with(b"GIF8") => "gif",
        b if b.starts_with(b"%PDF") => "pdf",
        b if b.starts_with(b"II*\0") || b.starts_with(b"MM\0*") => "tiff",
        b if b.len() > 12 && b.starts_with(b"RIFF") && &b[8..12] == b"WEBP" => "webp",
        b if b.len() > 12 && &b[4..8] == b"ftyp" => "heic",
        _ => "bin",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_batch_protocol_survives_text_that_contains_newlines() {
        // The framing exists because recognized text has line breaks in it. A
        // line-based parser would split one page into several records here.
        let stdout =
            "\u{1e}/tmp/a.png\u{1f}erste Zeile\nzweite Zeile\n\u{1e}/tmp/b.png\u{1f}only one\n";
        let records = parse_records(stdout);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].0, "/tmp/a.png");
        assert_eq!(records[0].1, "erste Zeile\nzweite Zeile");
        assert_eq!(records[1].1, "only one");
    }

    #[test]
    fn an_empty_record_parses_as_empty_text_rather_than_being_dropped() {
        // visocr emits an empty field both for "would not load" and for "no
        // text found". The parser must hand that back, because the decision of
        // what an empty page means belongs to the caller.
        let records = parse_records("\u{1e}/tmp/blank.png\u{1f}\n");
        assert_eq!(records, vec![("/tmp/blank.png".into(), String::new())]);
    }

    #[test]
    fn vision_refuses_a_class_it_does_not_own() {
        let error = VisionOcr
            .extract(&Document::html(b"<p>hi</p>"))
            .expect_err("vision reads pixels, not markup");
        assert_eq!(error.to_string(), "apple-vision does not read html input");
    }

    #[test]
    fn a_pdf_is_not_this_rung_s_input_even_a_scanned_one() {
        // visocr renders page one of a PDF and nothing else, so accepting the
        // class would mean a silent first-page read. Refusing the class is what
        // keeps that from being possible at all.
        let error = VisionOcr
            .extract(&Document::pdf(b"%PDF-1.4"))
            .expect_err("rung 2 reads pixels");
        assert_eq!(error.to_string(), "apple-vision does not read pdf input");
    }

    #[test]
    fn the_temp_file_is_named_for_what_it_actually_holds() {
        assert_eq!(extension_for(b"\x89PNG\r\n\x1a\nrest"), "png");
        assert_eq!(extension_for(b"%PDF-1.4"), "pdf");
        assert_eq!(extension_for(b"nothing recognizable"), "bin");
    }

    #[test]
    fn a_batch_of_nothing_runs_no_process_at_all() {
        // Also the only assertion in this module that holds identically on a
        // host without macOS: no child, so no platform guard is reached.
        assert_eq!(recognize(&[]).unwrap(), Vec::new());
    }

    #[test]
    fn an_absent_reader_is_unavailable_and_names_how_to_build_it() {
        // Guarded on macOS only: elsewhere the OS check answers first, and it
        // answers with the same variant for a different, equally true reason.
        if std::env::consts::OS != "macos" {
            let error = recognize(&[PathBuf::from("/tmp/x.png")]).expect_err("not macOS");
            assert!(error.to_string().contains("macOS framework"), "{error}");
            return;
        }
        let _env = env_lock();
        let _binary = EnvGuard::set(BINARY_ENV, "/nonexistent/axon-visocr-that-is-not-installed");
        let error = recognize(&[PathBuf::from("/tmp/x.png")]).expect_err("binary is absent");
        assert!(
            matches!(error, ExtractionError::Unavailable { .. }),
            "an absent tool is a missing rung, not a document failure: {error:?}"
        );
        assert!(
            error.to_string().contains("tools/visocr/build.sh"),
            "{error}"
        );
    }

    #[test]
    fn a_batch_larger_than_the_pipe_completes_instead_of_deadlocking() {
        // The reason the paths go out on their own thread. `cat` stands in for
        // visocr because it has the shape that matters: it writes to stdout
        // while it is still reading stdin. With one thread doing both, this
        // batch fills the child's stdout pipe (~64 KiB), `cat` stops reading,
        // stdin fills, and neither side ever moves again. A regression here
        // HANGS rather than failing, which is what the deadlock does in
        // production too.
        if std::env::consts::OS != "macos" {
            return; // The OS guard answers first; there is no child to fill.
        }
        let _env = env_lock();
        let _binary = EnvGuard::set(BINARY_ENV, "/bin/cat");
        let paths: Vec<PathBuf> = (0..20_000)
            .map(|n| PathBuf::from(format!("/tmp/axon-visocr-pipe-probe-{n:012}.png")))
            .collect();
        // `cat` echoes the paths, so no record separator is ever emitted and
        // every page is missing. Reaching that answer at all is the assertion.
        let error = recognize(&paths).expect_err("cat emits no visocr records");
        assert!(error.to_string().contains("returned no record"), "{error}");
    }

    /// One lock for every env-touching test in this module: cargo runs a
    /// crate's tests as parallel threads of ONE process, and `SJEL_VISOCR_BIN`
    /// is process-global. Same shape as `libs/sjel-config`'s.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Restores an env var on drop, including on unwind — a bare `remove_var`
    /// after the assertion is skipped by a failing one, and the next test then
    /// runs against a binary path this one invented.
    struct EnvGuard(&'static str, Option<String>);

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self(key, previous)
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match self.1.take() {
                Some(value) => std::env::set_var(self.0, value),
                None => std::env::remove_var(self.0),
            }
        }
    }
}
