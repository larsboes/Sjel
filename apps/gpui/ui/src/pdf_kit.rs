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

use std::sync::{mpsc, Arc};

pub use crate::documents::{PdfError, PdfRenderer};
use futures::channel::oneshot;
use gpui::RenderImage;
#[cfg(any(target_os = "macos", target_os = "ios"))]
use objc2_core_foundation::{CFData, CFRetained, CGPoint, CGRect, CGSize};
#[cfg(any(target_os = "macos", target_os = "ios"))]
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapContextGetBytesPerRow, CGBitmapContextGetData,
    CGBitmapContextGetHeight, CGBitmapInfo, CGColorSpace, CGContext, CGDataProvider,
    CGImageAlphaInfo, CGImageByteOrderInfo, CGPDFBox, CGPDFDocument, CGPDFPage,
};

#[cfg(any(target_os = "macos", target_os = "ios"))]
const MAX_PAGE_PIXELS: u64 = 64 * 1024 * 1024;

/// One page's pixels, in the order GPUI's image atlas wants them.
pub struct PageImage {
    pub width: u32,
    pub height: u32,
    /// BGRA, premultiplied, top row first.
    bgra: Vec<u8>,
}

impl PageImage {
    /// The image `gpui::img` draws.
    pub fn into_render_image(self) -> Arc<RenderImage> {
        let buffer = image::RgbaImage::from_raw(self.width, self.height, self.bgra)
            .expect("a buffer sized width × height × 4 is exactly one image");
        Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]))
    }
}

/// An opened PDF, bound to the thread that opened it.
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub struct Pdf {
    document: CFRetained<CGPDFDocument>,
    pages: usize,
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
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

        CGContext::set_rgb_fill_color(Some(&context), 1.0, 1.0, 1.0, 1.0);
        CGContext::fill_rect(Some(&context), target);
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

/// Without Core Graphics every document is refused by name, and the reader shows its text.
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
pub struct Pdf;

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
impl Pdf {
    pub fn open(_bytes: &[u8]) -> Result<Self, PdfError> {
        Err(PdfError::Unsupported)
    }

    pub fn page_count(&self) -> usize {
        0
    }

    pub fn render(&self, _index: usize, _width: u32) -> Result<PageImage, PdfError> {
        Err(PdfError::Unsupported)
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn rotated_size(width: f64, height: f64, rotation: i32) -> (f64, f64) {
    if rotation.rem_euclid(180) == 90 {
        (height, width)
    } else {
        (width, height)
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
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
    Count(oneshot::Sender<Result<usize, PdfError>>),
    Page {
        page: usize,
        width: u32,
        reply: oneshot::Sender<Result<Arc<RenderImage>, PdfError>>,
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
                let _ = reply.send(pdf.render(page, width).map(PageImage::into_render_image));
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
pub struct PageRenderer {
    requests: mpsc::SyncSender<Request>,
}

pub fn default_renderer() -> Result<Arc<dyn PdfRenderer>, String> {
    PageRenderer::new().map(|renderer| Arc::new(renderer) as Arc<dyn PdfRenderer>)
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
}

impl PdfRenderer for PageRenderer {
    fn info(&self, bytes: Arc<Vec<u8>>) -> oneshot::Receiver<Result<usize, PdfError>> {
        let (reply, answer) = oneshot::channel();
        let _ = self.requests.send(Request {
            bytes,
            job: Job::Count(reply),
        });
        answer
    }

    fn render(
        &self,
        bytes: Arc<Vec<u8>>,
        page: usize,
        width: u32,
    ) -> oneshot::Receiver<Result<Arc<RenderImage>, PdfError>> {
        let (reply, answer) = oneshot::channel();
        let _ = self.requests.send(Request {
            bytes,
            job: Job::Page { page, width, reply },
        });
        answer
    }

    #[cfg(target_os = "macos")]
    fn open_external(&self, filename: &str, bytes: &[u8]) {
        let dir = std::env::temp_dir().join("sjel-documents");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(filename);
        if std::fs::write(&path, bytes).is_ok() {
            let _ = std::process::Command::new("/usr/bin/open")
                .arg(&path)
                .spawn();
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    const ICON: &[u8] = include_bytes!("../../../mac/Resources/MenuBarIcon.pdf");

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
