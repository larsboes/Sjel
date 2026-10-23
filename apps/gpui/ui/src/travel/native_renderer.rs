//! Thread-affine MapLibre renderer. Only owned pixel buffers cross the worker boundary.
use std::num::NonZeroU32;

use maplibre_native::{CameraUpdate, ImageRenderer, ImageRendererBuilder, LatLng, Size, Static};

/// The same keyless basemap provider used by the dashboard; attribution is drawn by GPUI.
const STYLE: &str = "https://tiles.openfreemap.org/styles/liberty";

/// ## Every wait goes through `StyleLoadRequest::wait` / `RenderRequest::wait`
///
/// Those are the binding's own blocking waits, and they park on the current thread's
/// MapLibre run loop (`currentThreadRunLoopWait`). On Darwin that loop is CoreFoundation, so
/// a completion callback only runs while the loop itself runs: a hand-rolled
/// `RunLoopHandle::tick()` + sleep loop never delivers the callback and every render
/// "times out" after its full budget. Measured on this machine, the same Berlin frame takes
/// **837 ms** through `wait()` and never completes — 12 s of dead time — through tick polling.
///
/// Neither wait carries a deadline of its own; the cap belongs to the caller, which owns the
/// channel and can fall back to the raster map (`vector.rs`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Camera {
    pub latitude: f64,
    pub longitude: f64,
    /// Sjel's camera uses 256px world tiles. MapLibre uses 512px world tiles. Fractional, so a
    /// settled gesture renders exactly the zoom it stopped at.
    pub zoom: f64,
    /// Device pixels per layout point, handed to MapLibre as its pixel ratio: rendering stays
    /// the same geographic extent at the same zoom, but with a pixel for every device pixel.
    pub scale: u32,
    /// Logical pixels (points), not device pixels.
    pub width: u32,
    pub height: u32,
}

pub(super) struct Renderer {
    map: ImageRenderer<Static>,
    scale: u32,
}

impl Renderer {
    pub fn new(scale: u32) -> Result<Self, String> {
        // Native core asserts rather than returning an error if Metal is unavailable.
        let _device = metal::Device::system_default()
            .ok_or("No Metal device is available. Use the raster map on this system.")?;
        let mut map = ImageRendererBuilder::new()
            .with_pixel_ratio(scale as f32)
            .build_static_renderer();
        let url = STYLE
            .parse()
            .map_err(|error| format!("Invalid basemap URL: {error}"))?;
        map.load_style_from_url(&url)
            .wait()
            .map_err(|error| format!("Loading vector basemap: {error}"))?;
        Ok(Self { map, scale })
    }

    /// The pixel ratio this map was built with; `vector.rs` rebuilds on a change. The
    /// standalone probe example includes this module without `vector.rs`, so it reads as
    /// unused there.
    #[allow(dead_code)]
    pub fn scale(&self) -> u32 {
        self.scale
    }

    pub fn render(&mut self, camera: Camera) -> Result<image::RgbaImage, String> {
        // Bound allocation by what the map is made of, not by one axis: a full-window map at a
        // retina ratio is 3–8 MB of texture, and 12 million device pixels is the ceiling this
        // worker will hand to GPUI.
        let width = NonZeroU32::new(camera.width)
            .filter(|size| size.get() <= 4096)
            .ok_or("Map width must be between 1 and 4096 points.")?;
        let height = NonZeroU32::new(camera.height)
            .filter(|size| size.get() <= 2048)
            .ok_or("Map height must be between 1 and 2048 points.")?;
        let pixels = u64::from(width.get())
            * u64::from(height.get())
            * u64::from(self.scale)
            * u64::from(self.scale);
        if pixels > 12_000_000 {
            return Err("The map is too large to render at this window size.".into());
        }
        self.map.set_map_size(Size {
            width: width.get(),
            height: height.get(),
        });
        let camera = CameraUpdate::new()
            .center(LatLng {
                lat: camera.latitude,
                lng: camera.longitude,
            })
            .zoom(camera.zoom - 1.0);
        self.map
            .submit_render_static(&camera)
            .map_err(|error| format!("Starting vector map: {error}"))?
            .wait()
            .map(|image| image.as_image().clone())
            .map_err(|error| format!("Rendering vector map: {error}"))
    }
}
