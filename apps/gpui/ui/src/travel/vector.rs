//! Experimental image composition path; it does not share Metal textures with GPUI yet.
use std::sync::{mpsc, Arc};
use std::time::Duration;

use futures::channel::oneshot;
use gpui::{Context, RenderImage};

use super::{
    native_renderer::{Camera, Renderer},
    Travel,
};

struct Request {
    camera: Camera,
    reply: oneshot::Sender<Result<Arc<RenderImage>, String>>,
}

/// The worker parks in MapLibre's own blocking wait, which has no deadline of its own. A
/// stall shows up here instead: past this, the raster map takes over rather than leaving
/// "Loading map…" on screen indefinitely.
const RENDER_DEADLINE: Duration = Duration::from_secs(12);

pub(super) struct VectorMap {
    worker: mpsc::SyncSender<Request>,
    pub frame: Option<(Camera, Arc<RenderImage>)>,
    busy: bool,
}

impl VectorMap {
    fn new() -> Result<Self, String> {
        let (worker, requests) = mpsc::sync_channel::<Request>(1);
        std::thread::Builder::new()
            .name("sjel-vector-map".into())
            .spawn(move || {
                // Construct, use and drop the !Send native renderer on this one thread.
                let mut renderer: Option<Renderer> = None;
                while let Ok(request) = requests.recv() {
                    if request.reply.is_canceled() {
                        continue;
                    }
                    let result = (|| {
                        // The pixel ratio is fixed when the renderer is built, so a window that
                        // moves to a display with a different density rebuilds its worker map.
                        if renderer
                            .as_ref()
                            .is_some_and(|map| map.scale() != request.camera.scale)
                        {
                            renderer = None;
                        }
                        if renderer.is_none() {
                            renderer = Some(Renderer::new(request.camera.scale)?);
                        }
                        let map = renderer.as_mut().ok_or("Vector renderer is unavailable.")?;
                        let mut pixels = map.render(request.camera)?;
                        // GPUI's Metal image atlas expects BGRA, MapLibre returns RGBA.
                        for pixel in pixels.as_chunks_mut::<4>().0 {
                            pixel.swap(0, 2);
                        }
                        Ok(Arc::new(RenderImage::new(vec![image::Frame::new(pixels)])))
                    })();
                    if result.is_err() {
                        // A timed-out native request must not contaminate the next render.
                        renderer = None;
                    }
                    let _ = request.reply.send(result);
                }
            })
            .map_err(|error| format!("Starting vector map worker: {error}"))?;
        Ok(Self {
            worker,
            frame: None,
            busy: false,
        })
    }
}

impl Travel {
    fn vector_camera(&self) -> Option<Camera> {
        let center = self.center?;
        Some(Camera {
            latitude: center.latitude,
            longitude: center.longitude,
            zoom: self.zoom.max(1.),
            scale: self.scale,
            width: self.viewport.0.ceil() as u32,
            height: self.viewport.1.ceil() as u32,
        })
    }

    pub(super) fn load_vector_map(&mut self, cx: &mut Context<Self>) {
        let Some(camera) = self.vector_camera() else {
            return;
        };
        if self.vector.is_none() {
            match VectorMap::new() {
                Ok(map) => self.vector = Some(map),
                Err(error) => {
                    self.map_error = Some(error);
                    return;
                }
            }
        }
        let Some(map) = self.vector.as_mut() else {
            return;
        };
        if map.busy || map.frame.as_ref().is_some_and(|(key, _)| *key == camera) {
            return;
        }
        let (reply, response) = oneshot::channel();
        if let Err(error) = map.worker.try_send(Request { camera, reply }) {
            self.map_error = Some(format!("Vector map worker is unavailable: {error}"));
            return;
        }
        map.busy = true;
        self.map_loading = true;
        self.map_error = None;
        let generation = self.map_generation;
        cx.spawn(async move |this, cx| {
            let result = match futures::future::select(
                response,
                cx.background_executor().timer(RENDER_DEADLINE),
            )
            .await
            {
                futures::future::Either::Left((reply, _)) => {
                    reply.unwrap_or_else(|_| Err("Vector map worker stopped.".into()))
                }
                futures::future::Either::Right((_, _)) => {
                    Err("Vector map took too long. Check your connection and retry.".into())
                }
            };
            let _ = this.update(cx, |view, cx| {
                let current =
                    view.vector_camera() == Some(camera) && view.map_generation == generation;
                let mut fallback_to_raster = false;
                if let Some(map) = &mut view.vector {
                    map.busy = false;
                    if current && view.vector_enabled {
                        match result {
                            Ok(image) => map.frame = Some((camera, image)),
                            Err(_) => fallback_to_raster = true,
                        }
                    }
                }
                if fallback_to_raster {
                    // Keep the map usable when Metal, style loading, or vector rendering fails.
                    view.vector_enabled = false;
                    view.map_error = None;
                    view.load_map(cx);
                }
                if view.vector_enabled {
                    view.map_loading = false;
                    if !current {
                        // At most one render is in flight. A changed camera gets the next slot.
                        view.load_map(cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
