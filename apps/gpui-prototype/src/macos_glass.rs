#![allow(unsafe_code)]
//! Liquid Glass under the sidebar. GPUI draws every pixel into one Metal view, so a glass view
//! on top of it has nothing to refract. This puts two views behind GPUI's view and above GPUI's
//! own blur (`gpui-pre-macos-0.3.9/src/window.rs`, `set_background_appearance`): the window tint
//! under the sidebar strip, then a floating `NSGlassEffectView` on it. GPUI tints only the page
//! column, so the gap around the rounded panel gets the same tint as the page. GPUI keeps the
//! page tint itself: its translucent cards blend lighter over an AppKit tint than over its own. GPUI's view stays on top, so it
//! still receives every click and key.

use objc2::{runtime::AnyClass, MainThreadMarker};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBox, NSBoxType, NSColor, NSGlassEffectView,
    NSGlassEffectViewStyle, NSTitlePosition, NSView, NSWindowOrderingMode,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use gpui_kit::Window;

pub(crate) struct NativeGlassRegion {
    _glass: objc2::rc::Retained<NSGlassEffectView>,
    tint: objc2::rc::Retained<NSBox>,
    tint_rgba: u32,
}

impl NativeGlassRegion {
    /// Attaches a glass panel to the left `width` points of the window, `inset` points from its
    /// edges, the way a macOS 26 sidebar floats.
    pub(crate) fn attach(
        window: &mut Window,
        width: f64,
        inset: f64,
        corner_radius: f64,
    ) -> Result<Self, &'static str> {
        let main_thread = MainThreadMarker::new().ok_or("AppKit requires the main thread")?;
        if AnyClass::get(c"NSGlassEffectView").is_none() {
            return Err("NSGlassEffectView is unavailable on this macOS version");
        }

        let handle = window
            .window_handle()
            .map_err(|_| "GPUI did not provide an AppKit window handle")?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return Err("GPUI did not provide an AppKit window handle");
        };

        // SAFETY:
        // 1. GPUI's live AppKit window handle contains a non-null pointer to its native NSView.
        //    The borrowed window and its platform view remain alive for this synchronous call.
        // 2. `MainThreadMarker` proves AppKit access is on the main thread; the view cannot be
        //    concurrently torn down while GPUI is rendering this window.
        // 3. The pointer is cast directly to its NSView superclass without losing provenance.
        //    We create only a shared reference and call AppKit methods that use interior
        //    mutability; no Rust mutable alias is created and the pointer is not retained.
        let gpui_view = unsafe { &*handle.ns_view.as_ptr().cast::<NSView>() };
        // SAFETY: `superview` returns the window's content view, which GPUI added its view to
        // and which outlives this call; `Retained` keeps it alive while we use it.
        let content = unsafe { gpui_view.superview() }.ok_or("GPUI's view has no superview")?;

        let height = content.bounds().size.height;
        let strip = NSAutoresizingMaskOptions::ViewHeightSizable
            | NSAutoresizingMaskOptions::ViewMaxXMargin;
        let tint = NSBox::new(main_thread);
        tint.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(width, height),
        ));
        tint.setAutoresizingMask(strip);
        tint.setBoxType(NSBoxType::Custom);
        tint.setTitlePosition(NSTitlePosition::NoTitle);
        tint.setBorderWidth(0.0);
        content.addSubview_positioned_relativeTo(
            &tint,
            NSWindowOrderingMode::Below,
            Some(gpui_view),
        );

        let glass = NSGlassEffectView::new(main_thread);
        glass.setFrame(NSRect::new(
            NSPoint::new(inset, inset),
            NSSize::new(width - 2.0 * inset, height - 2.0 * inset),
        ));
        glass.setCornerRadius(corner_radius);
        glass.setAutoresizingMask(strip);
        glass.setStyle(NSGlassEffectViewStyle::Regular);
        content.addSubview_positioned_relativeTo(
            &glass,
            NSWindowOrderingMode::Below,
            Some(gpui_view),
        );

        Ok(Self {
            _glass: glass,
            tint,
            tint_rgba: 0,
        })
    }

    /// Fills the sidebar strip behind the glass with `rgba` (`0xRRGGBBAA`), as the page's tint.
    pub(crate) fn set_tint(&mut self, rgba: u32) {
        if self.tint_rgba == rgba {
            return;
        }
        self.tint_rgba = rgba;
        let [r, g, b, a] = rgba.to_be_bytes().map(|c| f64::from(c) / 255.0);
        self.tint
            .setFillColor(&NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, a));
    }
}
