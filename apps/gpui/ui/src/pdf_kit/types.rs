use super::document::Pdf;
use crate::documents::PdfError;
use futures::channel::oneshot;
use gpui::RenderImage;
use std::sync::Arc;

#[cfg(any(target_os = "macos", target_os = "ios"))]
pub const MAX_PAGE_PIXELS: u64 = 64 * 1024 * 1024;

/// One page's pixels, in the order GPUI's image atlas wants them.
pub struct PageImage {
    pub width: u32,
    pub height: u32,
    /// BGRA, premultiplied, top row first.
    pub bgra: Vec<u8>,
}

impl PageImage {
    pub fn new(width: u32, height: u32, bgra: Vec<u8>) -> Self {
        Self {
            width,
            height,
            bgra,
        }
    }

    /// The image `gpui::img` draws.
    pub fn into_render_image(self) -> Arc<RenderImage> {
        let buffer = image::RgbaImage::from_raw(self.width, self.height, self.bgra)
            .expect("a buffer sized width × height × 4 is exactly one image");
        Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]))
    }
}

pub enum Job {
    Count(oneshot::Sender<Result<usize, PdfError>>),
    Page {
        page: usize,
        width: u32,
        reply: oneshot::Sender<Result<Arc<RenderImage>, PdfError>>,
    },
}

impl Job {
    pub fn abandoned(&self) -> bool {
        match self {
            Self::Count(reply) => reply.is_canceled(),
            Self::Page { reply, .. } => reply.is_canceled(),
        }
    }

    pub fn finish(self, pdf: &Pdf) {
        match self {
            Self::Count(reply) => {
                let _ = reply.send(Ok(pdf.page_count()));
            }
            Self::Page { page, width, reply } => {
                let _ = reply.send(pdf.render(page, width).map(PageImage::into_render_image));
            }
        }
    }

    pub fn fail(self, error: PdfError) {
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

pub struct Request {
    pub bytes: Arc<Vec<u8>>,
    pub job: Job,
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
pub fn rotated_size(width: f64, height: f64, rotation: i32) -> (f64, f64) {
    if rotation.rem_euclid(180) == 90 {
        (height, width)
    } else {
        (width, height)
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
pub fn target_pixels(
    points_wide: f64,
    points_tall: f64,
    width: u32,
) -> Result<(u32, u32), PdfError> {
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
