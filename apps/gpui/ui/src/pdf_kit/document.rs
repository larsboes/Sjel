#![allow(unsafe_code)]

use super::types::PageImage;
use crate::documents::PdfError;

#[cfg(any(target_os = "macos", target_os = "ios"))]
use super::types::{rotated_size, target_pixels};
#[cfg(any(target_os = "macos", target_os = "ios"))]
use objc2_core_foundation::{CFData, CFRetained, CGPoint, CGRect, CGSize};
#[cfg(any(target_os = "macos", target_os = "ios"))]
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapContextGetBytesPerRow, CGBitmapContextGetData,
    CGBitmapContextGetHeight, CGBitmapInfo, CGColorSpace, CGContext, CGDataProvider,
    CGImageAlphaInfo, CGImageByteOrderInfo, CGPDFBox, CGPDFDocument, CGPDFPage,
};

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
    pub(crate) fn page_size(&self, index: usize) -> Result<(f64, f64), PdfError> {
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
        Ok(PageImage::new(width, height, bgra))
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
