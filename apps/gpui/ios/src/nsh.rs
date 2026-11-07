//! Mounts the NSH Documents reader (`sjel_gpui_ui::nsh_documents`) into a view from
//! `host/App.swift`, over the GPUI one (`docs/plans/gpui.md` §11). The view stays hidden until
//! the phone opens Documents.

#[cfg(target_os = "ios")]
thread_local! {
    /// Dropping the `AppView` stops the app, so the mounted one lives here.
    static MOUNTED: std::cell::RefCell<Option<nsh_platform_ios::AppView>> =
        const { std::cell::RefCell::new(None) };
}

/// Mounts the reader into `container`. Returns `false` off the main thread.
#[cfg(target_os = "ios")]
pub fn mount(container: &objc2_ui_kit::UIView) -> bool {
    use nsh_platform_ios::{set_ui_sets, AppView, Root, SurfaceOptions};
    use sjel_gpui_ui::nsh_documents::DocumentsScreen;
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return false;
    };
    set_ui_sets(vec![nsh_base::registry()]);
    sjel_gpui_ui::nsh_documents::set_close(crate::host::hide_nsh);
    let app = AppView::new(mtm, container.bounds(), SurfaceOptions::default());
    container.addSubview(app.view());
    app.run(Root::of::<DocumentsScreen>());
    MOUNTED.with(|slot| *slot.borrow_mut() = Some(app));
    true
}
