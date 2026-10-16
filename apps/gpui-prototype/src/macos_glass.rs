#![allow(unsafe_code)]

use objc2::{runtime::AnyClass, MainThreadMarker};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSGlassEffectView, NSGlassEffectViewStyle, NSTextAlignment,
    NSTextField, NSView,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use gpui_kit::Window;

pub(crate) struct NativeGlassRegion {
    _view: objc2::rc::Retained<NSGlassEffectView>,
}

impl NativeGlassRegion {
    pub(crate) fn attach(window: &mut Window) -> Result<Self, &'static str> {
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
        let host = unsafe { &*handle.ns_view.as_ptr().cast::<NSView>() };
        let bounds = host.bounds();
        let width = 184.0;
        let height = 36.0;
        let frame = NSRect::new(
            NSPoint::new(
                bounds.origin.x + bounds.size.width - 32.0 - width,
                bounds.origin.y + bounds.size.height - 56.0 - height,
            ),
            NSSize::new(width, height),
        );

        let glass = NSGlassEffectView::new(main_thread);
        glass.setFrame(frame);
        glass.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewMinXMargin | NSAutoresizingMaskOptions::ViewMinYMargin,
        );
        glass.setStyle(NSGlassEffectViewStyle::Regular);

        let label = NSTextField::labelWithString(&NSString::from_str("On this Mac"), main_thread);
        label.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(width, height),
        ));
        label.setAlignment(NSTextAlignment::Right);
        glass.setContentView(Some(&label));
        host.addSubview(&glass);

        Ok(Self { _view: glass })
    }
}
