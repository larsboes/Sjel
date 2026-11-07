//! The NSH trial (`docs/plans/gpui.md` §11, stage 3): a screen drawn with UIKit views by NSH,
//! mounted by `host/App.swift` in a view beside the GPUI one.

use nsh_core::prelude::*;

#[component]
pub fn Trial() -> impl IntoElement {
    view()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_4()
        .child(text("Documents"))
        .child(text("Drawn by NSH with UIKit views."))
}

#[cfg(target_os = "ios")]
thread_local! {
    /// Dropping the `AppView` stops the app, so the mounted one lives here.
    static MOUNTED: std::cell::RefCell<Option<nsh_platform_ios::AppView>> =
        const { std::cell::RefCell::new(None) };
}

/// Mounts [`Trial`] into `container`. Returns `false` off the main thread.
#[cfg(target_os = "ios")]
pub fn mount(container: &objc2_ui_kit::UIView) -> bool {
    use nsh_platform_ios::{set_ui_sets, AppView, Root, SurfaceOptions};
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return false;
    };
    set_ui_sets(vec![nsh_base::registry()]);
    let app = AppView::new(mtm, container.bounds(), SurfaceOptions::default());
    container.addSubview(app.view());
    app.run(Root::of::<Trial>());
    MOUNTED.with(|slot| *slot.borrow_mut() = Some(app));
    true
}
