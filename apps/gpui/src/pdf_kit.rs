#![allow(unsafe_code)]
//! PDF pages drawn into GPUI images through Core Graphics.
//!
//! Core Graphics is the PDF renderer this machine already has, and the extraction ladder already
//! trusts it for scans (`tools/visocr` draws every page of a PDF through PDFKit for Apple Vision).
//! This module draws the same pages for the reader, from the hash-verified original the Documents
//! capability serves — never from a copy, and never from a network service, so the iPhone reads the
//! same bytes offline.
//!
//! This is the second module in the app that writes `unsafe` (`macos_glass.rs` is the first), and it
//! does so for exactly two calls:
//!
//! 1. `CGBitmapContextCreate` takes a raw buffer pointer. Passing null makes Core Graphics allocate
//!    the pixels and free them with the context, so nothing here owns a raw allocation.
//! 2. `CGBitmapContextGetData` hands that allocation back as a pointer. The slice built from it is
//!    bounded by the context's own `bytes_per_row × height`, covering the full target size, and the
//!    pixels are copied into a `Vec` before the context drops.
//!
//! A [`Pdf`] is not `Send`: a Core Graphics document belongs to the thread that opened it. The
//! worker behind [`PageRenderer`] opens, draws and drops its document on one thread for that reason.

use std::sync::{mpsc, Arc};

use futures::channel::oneshot;
use gpui::RenderImage;
use objc2_core_foundation::{CFData, CFRetained, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapContextGetBytesPerRow, CGBitmapContextGetData,
    CGBitmapContextGetHeight, CGBitmapInfo, CGColorSpace, CGContext, CGDataProvider,
    CGImageAlphaInfo, CGImageByteOrderInfo, CGPDFBox, CGPDFDocument, CGPDFPage,
};

/// The most one page may be drawn at. A PDF's media box is document-supplied data: a page claiming
/// 100 000 points would otherwise ask for gigapixels of RAM. 64 MP is 256 MiB of BGRA, well past a
/// 5 K display at 3× and far below anything that would exhaust this machine.
const MAX_PAGE_PIXELS: u64 = 64 * 1024 * 1024;

/// Why a page could not be drawn. Every variant is shown to the reader rather than swallowed into
/// an empty pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PdfError {
    /// The bytes are not a PDF Core Graphics can open.
    Unreadable,
    /// The page number is outside the document.
    NoSuchPage,
    /// The page box is empty, or drawing it would need more than [`MAX_PAGE_PIXELS`].
    TooLarge,
    /// Core Graphics would not build a bitmap for this page.
    NoBitmap,
}

impl std::fmt::Display for PdfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unreadable => "This file is not a PDF this Mac can read.",
            Self::NoSuchPage => "That page is not in this document.",
            Self::TooLarge => "This page is too large to draw here.",
            Self::NoBitmap => "This page could not be drawn.",
        })
    }
}

/// One page's pixels, in the order GPUI's image atlas wants them.
pub struct PageImage {
    pub width: u32,
    pub height: u32,
    /// BGRA, premultiplied, top row first.
    bgra: Vec<u8>,
}

impl PageImage {
    /// The image `gpui::img` draws.
    ///
    /// The buffer is BGRA, and `gpui`'s `Frame` is the only constructor `RenderImage` accepts; a
    /// BGRA buffer wrapped in it is what `travel/vector.rs` does for the same reason.
    pub fn into_render_image(self) -> Arc<RenderImage> {
        let buffer = image::RgbaImage::from_raw(self.width, self.height, self.bgra)
            .expect("a buffer sized width × height × 4 is exactly one image");
        Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]))
    }
}

/// An opened PDF, bound to the thread that opened it.
pub struct Pdf {
    document: CFRetained<CGPDFDocument>,
    pages: usize,
}

impl Pdf {
    /// Open the original bytes. The bytes are copied into the document, so the caller may drop
    /// them; a document of several hundred pages holds no reference to the file on disk.
    pub fn open(bytes: &[u8]) -> Result<Self, PdfError> {
        let data = CFData::from_bytes(bytes);
        let provider = CGDataProvider::with_cf_data(Some(&data)).ok_or(PdfError::Unreadable)?;
        let document = CGPDFDocument::with_provider(Some(&provider)).ok_or(PdfError::Unreadable)?;
        let pages = CGPDFDocument::number_of_pages(Some(&document));
        if pages == 0 {
            return Err(PdfError::Unreadable);
        }
        Ok(Self { document, pages })
    }

    pub fn page_count(&self) -> usize {
        self.pages
    }

    /// The page's size in points as it will be drawn: the media box with the page's own rotation
    /// applied, so a scan stored sideways reports the shape the reader sees.
    #[cfg(test)]
    fn page_size(&self, index: usize) -> Result<(f64, f64), PdfError> {
        let page = self.page(index)?;
        let box_rect = CGPDFPage::box_rect(Some(&page), CGPDFBox::MediaBox);
        let rotation = CGPDFPage::rotation_angle(Some(&page));
        Ok(rotated_size(
            box_rect.size.width,
            box_rect.size.height,
            rotation,
        ))
    }

    fn page(&self, index: usize) -> Result<CFRetained<CGPDFPage>, PdfError> {
        if index >= self.pages {
            return Err(PdfError::NoSuchPage);
        }
        // Core Graphics numbers pages from one; the reader numbers them from zero.
        CGPDFDocument::page(Some(&self.document), index + 1).ok_or(PdfError::NoSuchPage)
    }

    /// Draw page `index` `width` pixels wide, keeping the page's aspect ratio.
    pub fn render(&self, index: usize, width: u32) -> Result<PageImage, PdfError> {
        let page = self.page(index)?;
        let box_rect = CGPDFPage::box_rect(Some(&page), CGPDFBox::MediaBox);
        let rotation = CGPDFPage::rotation_angle(Some(&page));
        let (points_wide, points_tall) =
            rotated_size(box_rect.size.width, box_rect.size.height, rotation);
        let (width, height) = target_pixels(points_wide, points_tall, width)?;
        let target = CGRect::new(
            CGPoint::ZERO,
            CGSize::new(f64::from(width), f64::from(height)),
        );

        let space = CGColorSpace::new_device_rgb().ok_or(PdfError::NoBitmap)?;
        // Premultiplied first, 32-bit little-endian: BGRA in memory, which is what GPUI's atlas
        // reads (`travel/vector.rs` swaps MapLibre's RGBA for the same reason).
        let info = CGBitmapInfo(
            CGImageAlphaInfo::PremultipliedFirst.0 | CGImageByteOrderInfo::Order32Little.0,
        );
        // SAFETY: the buffer pointer is null, so Core Graphics allocates and owns the pixels; the
        // width, height and row length are the ones read back below.
        let context = unsafe {
            CGBitmapContextCreate(
                std::ptr::null_mut(),
                width as usize,
                height as usize,
                8,
                4 * width as usize,
                Some(&space),
                info.0,
            )
        }
        .ok_or(PdfError::NoBitmap)?;

        // Paper is white. A PDF page with no background would otherwise show the atlas's
        // transparent black, which reads as a rendering failure.
        CGContext::set_rgb_fill_color(Some(&context), 1.0, 1.0, 1.0, 1.0);
        CGContext::fill_rect(Some(&context), target);
        // `drawing_transform` maps the page's box into the target rectangle and accounts for the
        // page's own rotation and origin, so no flip or translate is written here by hand.
        let transform =
            CGPDFPage::drawing_transform(Some(&page), CGPDFBox::MediaBox, target, 0, true);
        CGContext::concat_ctm(Some(&context), transform);
        CGContext::draw_pdf_page(Some(&context), Some(&page));

        let bytes_per_row = CGBitmapContextGetBytesPerRow(Some(&context));
        let drawn_height = CGBitmapContextGetHeight(Some(&context));
        let data = CGBitmapContextGetData(Some(&context));
        if data.is_null() || drawn_height == 0 {
            return Err(PdfError::NoBitmap);
        }
        let row = width as usize * 4;
        if bytes_per_row < row || drawn_height != height as usize {
            return Err(PdfError::NoBitmap);
        }
        let mut bgra = Vec::with_capacity(row * drawn_height);
        for row_index in 0..drawn_height {
            // SAFETY: `data` is Core Graphics' buffer of `bytes_per_row × height` bytes, the context
            // is still alive, and the range read is one row inside that buffer.
            let start = unsafe { (data as *const u8).add(row_index * bytes_per_row) };
            // SAFETY: as above; `row` is at most `bytes_per_row`.
            let pixels = unsafe { std::slice::from_raw_parts(start, row) };
            bgra.extend_from_slice(pixels);
        }
        Ok(PageImage {
            width,
            height,
            bgra,
        })
    }
}

/// A page's drawn size in points, with `/Rotate` applied.
fn rotated_size(width: f64, height: f64, rotation: i32) -> (f64, f64) {
    if rotation.rem_euclid(180) == 90 {
        (height, width)
    } else {
        (width, height)
    }
}

/// The pixel size a page is drawn at for a requested width, refusing a page too large to hold.
fn target_pixels(points_wide: f64, points_tall: f64, width: u32) -> Result<(u32, u32), PdfError> {
    if !(points_wide.is_finite() && points_tall.is_finite())
        || points_wide <= 0.0
        || points_tall <= 0.0
    {
        return Err(PdfError::TooLarge);
    }
    let width = width.max(1);
    let height = (f64::from(width) * points_tall / points_wide)
        .round()
        .max(1.0);
    if height > f64::from(u32::MAX) {
        return Err(PdfError::TooLarge);
    }
    let height = height as u32;
    if u64::from(width) * u64::from(height) > MAX_PAGE_PIXELS {
        return Err(PdfError::TooLarge);
    }
    Ok((width, height))
}

enum Job {
    /// How many pages the document has, asked once when an original is opened.
    Count(oneshot::Sender<Result<usize, PdfError>>),
    Page {
        page: usize,
        width: u32,
        reply: oneshot::Sender<Result<PageImage, PdfError>>,
    },
}

impl Job {
    fn abandoned(&self) -> bool {
        match self {
            Self::Count(reply) => reply.is_canceled(),
            Self::Page { reply, .. } => reply.is_canceled(),
        }
    }

    fn finish(self, pdf: &Pdf) {
        match self {
            Self::Count(reply) => {
                let _ = reply.send(Ok(pdf.page_count()));
            }
            Self::Page { page, width, reply } => {
                let _ = reply.send(pdf.render(page, width));
            }
        }
    }

    fn fail(self, error: PdfError) {
        match self {
            Self::Count(reply) => {
                let _ = reply.send(Err(error));
            }
            Self::Page { reply, .. } => {
                let _ = reply.send(Err(error));
            }
        }
    }
}

struct Request {
    bytes: Arc<Vec<u8>>,
    job: Job,
}

/// Draws pages on one private thread.
///
/// A page takes long enough to drop a frame, and a Core Graphics document may not move between
/// threads, so the thread that opens the document is the thread that draws from it. The document is
/// reopened only when the bytes change, which for a reader means when another document is opened.
pub struct PageRenderer {
    requests: mpsc::SyncSender<Request>,
}

impl PageRenderer {
    /// # Errors
    /// The worker thread could not be started.
    pub fn new() -> Result<Self, String> {
        let (requests, incoming) = mpsc::sync_channel::<Request>(1);
        std::thread::Builder::new()
            .name("sjel-pdf-pages".into())
            .spawn(move || {
                let mut opened: Option<(Arc<Vec<u8>>, Pdf)> = None;
                while let Ok(request) = incoming.recv() {
                    if request.job.abandoned() {
                        // The reader moved on before this page was asked for.
                        continue;
                    }
                    let reused = opened
                        .as_ref()
                        .filter(|(bytes, _)| Arc::ptr_eq(bytes, &request.bytes));
                    let pdf = match reused {
                        Some((_, pdf)) => Some(pdf),
                        None => {
                            opened = match Pdf::open(&request.bytes) {
                                Ok(pdf) => Some((request.bytes.clone(), pdf)),
                                Err(error) => {
                                    request.job.fail(error);
                                    continue;
                                }
                            };
                            opened.as_ref().map(|(_, pdf)| pdf)
                        }
                    };
                    match pdf {
                        Some(pdf) => request.job.finish(pdf),
                        None => request.job.fail(PdfError::Unreadable),
                    }
                }
            })
            .map_err(|error| format!("Starting the PDF page worker: {error}"))?;
        Ok(Self { requests })
    }

    /// How many pages the opened bytes hold. Asked once, so the reader can size its page strip.
    pub fn info(&self, bytes: Arc<Vec<u8>>) -> oneshot::Receiver<Result<usize, PdfError>> {
        let (reply, answer) = oneshot::channel();
        let _ = self.requests.send(Request {
            bytes,
            job: Job::Count(reply),
        });
        answer
    }

    /// Ask for one page. The answer carries the pixels, or why there are none.
    pub fn render(
        &self,
        bytes: Arc<Vec<u8>>,
        page: usize,
        width: u32,
    ) -> oneshot::Receiver<Result<PageImage, PdfError>> {
        let (reply, answer) = oneshot::channel();
        let _ = self.requests.send(Request {
            bytes,
            job: Job::Page { page, width, reply },
        });
        answer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tracked vector PDF from the menu-bar app: one real page, no personal content.
    const ICON: &[u8] = include_bytes!("../../mac/Resources/MenuBarIcon.pdf");

    #[test]
    fn a_real_pdf_opens_and_draws_its_first_page() {
        let pdf = Pdf::open(ICON).expect("the tracked fixture is a PDF");
        assert!(pdf.page_count() >= 1);
        assert!(pdf.page_size(0).is_ok());

        let page = pdf.render(0, 48).expect("page one draws");
        assert_eq!(page.width, 48);
        assert_eq!(page.bgra.len(), 48 * page.height as usize * 4);
        // The icon is drawn on the white paper fill, so something must differ from blank paper.
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
        // A page box of no size is a document defect, not an instruction to allocate nothing.
        assert_eq!(target_pixels(0.0, 842.0, 600), Err(PdfError::TooLarge));
        assert_eq!(target_pixels(f64::NAN, 842.0, 600), Err(PdfError::TooLarge));
        // Three of these pages horizontally would be absurd; one of them at this width is too much.
        assert_eq!(target_pixels(1.0, 20_000.0, 4000), Err(PdfError::TooLarge));
    }
}
