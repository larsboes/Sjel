#![allow(unsafe_code)]
//! PDF pages drawn into GPUI images through Core Graphics.
//!
//! Core Graphics is the PDF renderer on macOS and iOS (`tools/visocr` draws every page of a PDF
//! through PDFKit for Apple Vision). This module draws the same pages for the reader, from the
//! hash-verified original the Documents capability serves — never from a copy, and never from a
//! network service, so the iPhone reads the same bytes offline.
//!
//! A [`Pdf`] is not `Send`: a Core Graphics document belongs to the thread that opened it. The
//! worker behind [`PageRenderer`] opens, draws and drops its document on one thread for that reason.

pub mod types;
pub mod document;
pub mod renderer;

pub use crate::documents::{PdfError, PdfRenderer};
pub use types::PageImage;
pub use document::Pdf;
pub use renderer::{default_renderer, PageRenderer};

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use types::{rotated_size, target_pixels};

    const ICON: &[u8] = include_bytes!("../../../../mac/Resources/MenuBarIcon.pdf");

    #[test]
    fn a_real_pdf_opens_and_draws_its_first_page() {
        let pdf = Pdf::open(ICON).expect("the tracked fixture is a PDF");
        assert!(pdf.page_count() >= 1);
        assert!(pdf.page_size(0).is_ok());

        let page = pdf.render(0, 48).expect("page one draws");
        assert_eq!(page.width, 48);
        assert_eq!(page.bgra.len(), 48 * page.height as usize * 4);
        assert!(
            page.bgra
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel != &[255, 255, 255, 255]),
            "a drawn page is not a blank one"
        );
    }

    #[test]
    fn a_page_outside_the_document_is_refused_by_name() {
        let pdf = Pdf::open(ICON).unwrap();
        assert_eq!(pdf.page_size(pdf.page_count()), Err(PdfError::NoSuchPage));
        assert_eq!(
            pdf.render(pdf.page_count(), 48).err(),
            Some(PdfError::NoSuchPage)
        );
    }

    #[test]
    fn bytes_that_are_not_a_pdf_are_refused() {
        assert_eq!(Pdf::open(b"not a pdf").err(), Some(PdfError::Unreadable));
        assert_eq!(Pdf::open(b"").err(), Some(PdfError::Unreadable));
    }

    #[test]
    fn a_sideways_page_reports_the_shape_the_reader_sees() {
        assert_eq!(rotated_size(595.0, 842.0, 0), (595.0, 842.0));
        assert_eq!(rotated_size(595.0, 842.0, 90), (842.0, 595.0));
        assert_eq!(rotated_size(595.0, 842.0, 270), (842.0, 595.0));
        assert_eq!(rotated_size(595.0, 842.0, 180), (595.0, 842.0));
        assert_eq!(rotated_size(595.0, 842.0, -90), (842.0, 595.0));
    }

    #[test]
    fn a_page_is_drawn_at_its_own_aspect_ratio() {
        assert_eq!(target_pixels(595.0, 842.0, 595), Ok((595, 842)));
        assert_eq!(target_pixels(595.0, 842.0, 1200), Ok((1200, 1698)));
        assert_eq!(target_pixels(0.0, 842.0, 600), Err(PdfError::TooLarge));
        assert_eq!(target_pixels(f64::NAN, 842.0, 600), Err(PdfError::TooLarge));
        assert_eq!(target_pixels(1.0, 20_000.0, 4000), Err(PdfError::TooLarge));
    }
}
