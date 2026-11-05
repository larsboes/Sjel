use super::document::Pdf;
use super::types::{Job, Request};
use crate::documents::{PdfError, PdfRenderer};
use futures::channel::oneshot;
use gpui::RenderImage;
use std::sync::{mpsc, Arc};

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
