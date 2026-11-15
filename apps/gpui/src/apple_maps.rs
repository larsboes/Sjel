#![allow(unsafe_code)]
//! Native macOS Apple Maps (`MKMapView`) embedded inline inside GPUI's window view.

use std::cell::RefCell;

use gpui_kit::{Bounds, Pixels, Window};
use objc2::encode::{Encode, Encoding};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{msg_send, MainThreadMarker};
use objc2_app_kit::{NSView, NSWindowOrderingMode};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use sjel_gpui_core::travel::Coordinate;
use sjel_gpui_ui::travel::{AppleMapPin, InlineAppleMapCommand};

#[repr(C)]
#[derive(Clone, Copy)]
struct MKMapPoint {
    x: f64,
    y: f64,
}

#[link(name = "MapKit", kind = "framework")]
extern "C" {
    fn MKMapPointForCoordinate(coordinate: CLLocationCoordinate2D) -> MKMapPoint;
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CLLocationCoordinate2D {
    latitude: f64,
    longitude: f64,
}

unsafe impl Encode for CLLocationCoordinate2D {
    const ENCODING: Encoding =
        Encoding::Struct("CLLocationCoordinate2D", &[f64::ENCODING, f64::ENCODING]);
}

#[repr(C)]
#[derive(Clone, Copy)]
struct MKCoordinateSpan {
    latitude_delta: f64,
    longitude_delta: f64,
}

unsafe impl Encode for MKCoordinateSpan {
    const ENCODING: Encoding =
        Encoding::Struct("MKCoordinateSpan", &[f64::ENCODING, f64::ENCODING]);
}

#[repr(C)]
#[derive(Clone, Copy)]
struct MKCoordinateRegion {
    center: CLLocationCoordinate2D,
    span: MKCoordinateSpan,
}

unsafe impl Encode for MKCoordinateRegion {
    const ENCODING: Encoding = Encoding::Struct(
        "MKCoordinateRegion",
        &[CLLocationCoordinate2D::ENCODING, MKCoordinateSpan::ENCODING],
    );
}

#[derive(PartialEq)]
struct MapSceneKey {
    lat_e5: i64,
    lon_e5: i64,
    zoom_e2: i64,
    pins: Vec<AppleMapPin>,
}

impl MapSceneKey {
    fn new(center: Coordinate, zoom: f64, pins: &[AppleMapPin]) -> Self {
        Self {
            lat_e5: (center.latitude * 100_000.0).round() as i64,
            lon_e5: (center.longitude * 100_000.0).round() as i64,
            zoom_e2: (zoom * 100.0).round() as i64,
            pins: pins.to_vec(),
        }
    }
}

struct InlineMapState {
    map_view: Retained<NSView>,
    scene: Option<MapSceneKey>,
}

thread_local! {
    static INLINE_MAP: RefCell<Option<InlineMapState>> = const { RefCell::new(None) };
}

fn region_for(center: Coordinate, zoom: f64) -> MKCoordinateRegion {
    let clamped_zoom = zoom.clamp(1.0, 18.0);
    let lat_delta = (180.0 / 2.0_f64.powf(clamped_zoom - 1.0)).clamp(0.005, 120.0);
    let lon_delta = (360.0 / 2.0_f64.powf(clamped_zoom - 1.0)).clamp(0.005, 180.0);
    MKCoordinateRegion {
        center: CLLocationCoordinate2D {
            latitude: center.latitude,
            longitude: center.longitude,
        },
        span: MKCoordinateSpan {
            latitude_delta: lat_delta,
            longitude_delta: lon_delta,
        },
    }
}

/// Handles inline Apple Maps commands from `Travel::map`.
pub fn handle_inline_command(window: &mut Window, cmd: InlineAppleMapCommand<'_>) {
    match cmd {
        InlineAppleMapCommand::Show {
            bounds,
            center,
            zoom,
            pins,
        } => {
            if let Err(err) = show_inline(window, bounds, center, zoom, pins) {
                eprintln!("apple_maps inline error: {err}");
            }
        }
        InlineAppleMapCommand::Hide => hide_inline(),
    }
}

/// Hides the inline `MKMapView` when no map is visible on the current page.
pub fn hide_inline() {
    if MainThreadMarker::new().is_none() {
        return;
    }
    INLINE_MAP.with(|slot| {
        if let Some(state) = slot.borrow().as_ref() {
            state.map_view.setHidden(true);
        }
    });
}

fn show_inline(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    center: Coordinate,
    zoom: f64,
    pins: &[AppleMapPin],
) -> Result<(), &'static str> {
    let _mtm = MainThreadMarker::new().ok_or("MapKit requires the main thread")?;
    // Ensure MapKit.framework is linked and its Objective-C classes are registered.
    let _ = unsafe {
        MKMapPointForCoordinate(CLLocationCoordinate2D {
            latitude: 0.0,
            longitude: 0.0,
        })
    };
    let map_cls = AnyClass::get(c"MKMapView").ok_or("MKMapView is unavailable")?;

    let handle = window
        .window_handle()
        .map_err(|_| "GPUI did not provide an AppKit window handle")?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return Err("GPUI did not provide an AppKit window handle");
    };
    // SAFETY: `handle.ns_view` is GPUI's live `NSView` for the current window on the main thread.
    let gpui_view = unsafe { &*handle.ns_view.as_ptr().cast::<NSView>() };

    let x = f64::from(bounds.origin.x);
    let w = f64::from(bounds.size.width);
    let h = f64::from(bounds.size.height);
    if w < 2.0 || h < 2.0 {
        hide_inline();
        return Ok(());
    }
    let y = if gpui_view.isFlipped() {
        f64::from(bounds.origin.y)
    } else {
        gpui_view.bounds().size.height - f64::from(bounds.origin.y) - h
    };
    let frame = NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));

    INLINE_MAP.with(|slot| {
        let mut guard = slot.borrow_mut();
        if guard.is_none() {
            // SAFETY: `MKMapView` is an `NSView` subclass; `initWithFrame:` returns a +1 retained instance.
            let raw_map: *mut NSView = unsafe {
                let alloc: *mut AnyObject = msg_send![map_cls, alloc];
                msg_send![alloc, initWithFrame: frame]
            };
            let map_view = unsafe { Retained::from_raw(raw_map) }
                .ok_or("MKMapView initWithFrame returned nil")?;
            map_view.setWantsLayer(true);
            // SAFETY: `MKMapView` responds to compass/zoom/scale and its CALayer responds to cornerRadius/masksToBounds.
            unsafe {
                let _: () = msg_send![&*map_view, setShowsCompass: true];
                let _: () = msg_send![&*map_view, setShowsZoomControls: true];
                let _: () = msg_send![&*map_view, setShowsScale: true];
                let layer: *mut AnyObject = msg_send![&*map_view, layer];
                if !layer.is_null() {
                    let _: () = msg_send![layer, setCornerRadius: 16.0_f64];
                    let _: () = msg_send![layer, setMasksToBounds: true];
                }
            }
            gpui_view.addSubview_positioned_relativeTo(
                &map_view,
                NSWindowOrderingMode::Above,
                None,
            );
            *guard = Some(InlineMapState {
                map_view,
                scene: None,
            });
        }

        let state = guard.as_mut().expect("initialized above");
        if unsafe { state.map_view.superview() }.as_deref() != Some(gpui_view) {
            gpui_view.addSubview_positioned_relativeTo(
                &state.map_view,
                NSWindowOrderingMode::Above,
                None,
            );
        }
        state.map_view.setFrame(frame);
        state.map_view.setHidden(false);

        let key = MapSceneKey::new(center, zoom, pins);
        if state.scene.as_ref() != Some(&key) {
            // SAFETY: `annotations` and `removeAnnotations:` are standard `MKMapView` selectors.
            unsafe {
                let existing: Retained<AnyObject> = msg_send![&*state.map_view, annotations];
                let _: () = msg_send![&*state.map_view, removeAnnotations: &*existing];
            }
            if let Some(pin_cls) = AnyClass::get(c"MKPointAnnotation") {
                for pin in pins {
                    // SAFETY: `MKPointAnnotation` responds to `new`, `setCoordinate:`, `setTitle:`, `setSubtitle:`.
                    unsafe {
                        let ann: Retained<AnyObject> = msg_send![pin_cls, new];
                        let coord = CLLocationCoordinate2D {
                            latitude: pin.coordinate.latitude,
                            longitude: pin.coordinate.longitude,
                        };
                        let _: () = msg_send![&*ann, setCoordinate: coord];
                        let _: () = msg_send![&*ann, setTitle: &*NSString::from_str(&pin.title)];
                        if !pin.subtitle.is_empty() {
                            let _: () =
                                msg_send![&*ann, setSubtitle: &*NSString::from_str(&pin.subtitle)];
                        }
                        let _: () = msg_send![&*state.map_view, addAnnotation: &*ann];
                    }
                }
            }
            let region = region_for(center, zoom);
            // SAFETY: `setRegion:animated:` is a standard `MKMapView` selector.
            unsafe {
                let _: () = msg_send![&*state.map_view, setRegion: region, animated: false];
            }
            state.scene = Some(key);
        }
        Ok(())
    })
}

/// Reports the unavailable detached map so the caller can open Apple Maps externally.
///
/// # Errors
/// A detached native map window is not implemented.
pub fn open_map_window(
    _title: &str,
    _center: Coordinate,
    _zoom: f64,
    _pins: &[AppleMapPin],
) -> Result<(), &'static str> {
    Err("Native Apple Maps is available inline only")
}
